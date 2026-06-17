#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

extern crate alloc;

use alloc::alloc::Layout;
use core::alloc::{GlobalAlloc, Layout as CoreLayout};
use core::arch::global_asm;
use core::cell::UnsafeCell;

use arch_hal::soc::bcm2712;
use block_device_api::BlockDevice;
use dtb::DtbParser;

mod bcm2712_aon;
mod bcm2712_i2c;
mod boot_files;
mod dtb_patch;
mod gzip;
mod linux;
mod panic;
mod placement;
mod rp1_bootstrap;
mod rp1_image;

mod trace {
    use core::cell::UnsafeCell;
    use core::fmt;
    use core::fmt::Write;

    const TRACE_BUF_LEN: usize = 8192;

    #[repr(C)]
    pub struct TraceState {
        pub write: usize,
        pub buf: [u8; TRACE_BUF_LEN],
    }

    #[repr(transparent)]
    pub struct TraceCell(UnsafeCell<TraceState>);

    unsafe impl Sync for TraceCell {}

    #[unsafe(no_mangle)]
    pub static __TRACE_STATE: TraceCell = TraceCell(UnsafeCell::new(TraceState {
        write: 0,
        buf: [0; TRACE_BUF_LEN],
    }));

    pub fn puts(s: &str) {
        with_writer(|writer| {
            let _ = writer.write_str(s);
        });
    }

    pub fn write_fmt(args: fmt::Arguments<'_>) {
        with_writer(|writer| {
            let _ = fmt::write(writer, args);
        });
    }

    fn with_writer(f: impl FnOnce(&mut TraceWriter<'_>)) {
        // SAFETY: the PoC runs on a single core before handing off to Linux.
        let state = unsafe { &mut *__TRACE_STATE.0.get() };
        let mut writer = TraceWriter { state };
        f(&mut writer);
    }

    struct TraceWriter<'a> {
        state: &'a mut TraceState,
    }

    impl Write for TraceWriter<'_> {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            for b in s.bytes() {
                let idx = self.state.write % TRACE_BUF_LEN;
                self.state.buf[idx] = b;
                self.state.write = self.state.write.wrapping_add(1);
            }
            Ok(())
        }
    }
}

#[cfg(all(feature = "log-uart", feature = "log-semihosting"))]
compile_error!("features `log-uart` and `log-semihosting` are mutually exclusive");

#[cfg(not(any(feature = "log-uart", feature = "log-semihosting")))]
compile_error!("select exactly one log backend feature: `log-uart` or `log-semihosting`");

global_asm!(
    r#"
    .section .text.boot, "ax"
    .global _start
    .type _start, %function
_start:
    msr spsel, #1
    ldr x0, =_STACK_TOP
    mov sp, x0
    ldr x0, =_BSS_START
    ldr x1, =_BSS_END
1:
    cmp x0, x1
    b.hs 2f
    str xzr, [x0], #8
    b 1b
2:
    bl rust_main
3:
    wfe
    b 3b
    .size _start, . - _start
"#
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootError {
    I2cTimeout,
    I2cNack,
    I2cWrite,
    I2cRead,
    Gpio,
    Rp1ChunkTooLarge,
    Rp1ImageInvalid,
    Rp1ImageCrcMismatch,
    Rp1ImageTooLarge,
    SdFileNotFound,
    SdMount,
    SdOpen,
    SdRead,
    Gzip,
    DtbPatch,
    LinuxImageInvalid,
    MemoryOverlap,
    SdhcQuiesceFailure,
    El2HandoffPreparationFailure,
    AddressOverflow,
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_main() -> ! {
    if let Err(err) = main_flow() {
        fatal(err);
    }
    halt()
}

fn main_flow() -> Result<(), BootError> {
    logging::init();
    logln!("[BOOT] start EL2");
    timer::init();
    logln!("[TLS] init skipped: static BSP TLS symbols preserved by linker");
    logln!("[EXC] vector init skipped for PoC bringup");

    let dtb = DtbParser::init(placement::DTB_PTR).map_err(|_| BootError::DtbPatch)?;
    logln!("[DTB] parse ok");
    logln!("[ALLOC] static bump allocator ok: size={} bytes", HEAP_SIZE);

    placement::check_no_overlap(&[
        placement::program_range(),
        placement::stack_range(),
        placement::Range::new("dtb-input", placement::DTB_PTR, dtb.get_size()),
        placement::Range::new(
            "kernel",
            placement::KERNEL_LOAD_BASE,
            placement::KERNEL_MAX_SIZE,
        ),
        placement::Range::new(
            "initramfs",
            placement::INITRAMFS_LOAD_BASE,
            placement::INITRAMFS_MAX_SIZE,
        ),
        placement::Range::new(
            "dtb-copy",
            placement::DTB_COPY_BASE,
            placement::DTB_MAX_SIZE,
        ),
        placement::Range::new(
            "rp1-scratch",
            placement::RP1_IMG_SCRATCH_BASE,
            placement::RP1_IMG_SCRATCH_MAX,
        ),
    ])?;

    match bcm2712::init_rp1(&dtb) {
        Ok(_rp1_cfg) => {
            logln!("[RP1] init_rp1 ok");
            logln!("[RP1] existing RP1 visible");
        }
        Err(err) => {
            logln!("[RP1] init_rp1 failed: {:?}", err);
            logln!("[RP1] continuing to SDHC/RP1 bootstrap PoC path");
        }
    }

    logln!("[SDHC] init begin");
    let sdhc: &'static dyn BlockDevice = match bcm2712::sdhc::init_from_dtb(&dtb) {
        Ok(sdhc) => sdhc,
        Err(err) => {
            logln!("[SDHC] init failed: {:?}", err);
            return Err(BootError::SdMount);
        }
    };
    logln!("[SDHC] init ok");

    boot_files::probe_file(sdhc, "/config.txt", "/config.txt before reset")?;

    let rp1_img_file;
    let fw1_holder;
    let fw2_holder;
    let rp1_image = if cfg!(feature = "skip-rp1-reload") {
        logln!("[RP1BOOT] skipped by feature skip-rp1-reload");
        None
    } else {
        rp1_img_file = read_first_optional_file(
            sdhc,
            &["/RP1.img", "/rp1/RP1.img", "/rp1/rp1.img", "/RP1/RP1.IMG"],
        )?;
        if rp1_img_file.is_some() {
            logln!("[SD] /RP1.img found");
        } else {
            logln!("[SD] /RP1.img not found");
        }

        let fw_scratch = placement::rp1_scratch_slice();
        if let Some(ref image_bytes) = rp1_img_file {
            let image = rp1_image::parse_rp1_img(image_bytes)?;
            logln!(
                "[SD] /RP1.img ok: payload={} load=0x{:x} entry=0x{:x} stack=0x{:x}",
                image.payload.len(),
                image.load_addr,
                image.entry,
                image.stack
            );
            Some(image)
        } else {
            if cfg!(feature = "require-rp1-img") {
                return Err(BootError::SdFileNotFound);
            }
            logln!(
                "[RP1IMG] fallback fw-parts uses configured entry=0x{:08x} stack=0x{:08x}",
                rp1_image::RP1_FALLBACK_ENTRY | 1,
                rp1_image::RP1_FALLBACK_STACK
            );
            logln!("[RP1IMG] prefer /RP1.img for exact entry/stack");
            let fw1_candidate = read_first_optional_file(
                sdhc,
                &[
                    "/rp1c0fw1.bin",
                    "/rp1/rp1c0fw1.bin",
                    "/RP1/FW1.BIN",
                    "/RP1C0FW1.BIN",
                ],
            )?;
            let fw2_candidate = read_first_optional_file(
                sdhc,
                &[
                    "/rp1c0fw2.bin",
                    "/rp1/rp1c0fw2.bin",
                    "/RP1/FW2.BIN",
                    "/RP1C0FW2.BIN",
                ],
            )?;
            match (fw1_candidate, fw2_candidate) {
                (Some(fw1), Some(fw2)) => {
                    fw1_holder = fw1;
                    fw2_holder = fw2;
                    logln!(
                        "[SD] rp1 fw part0 ok: size={} checksum=0x{:08x}",
                        fw1_holder.len(),
                        rp1_image::checksum32(&fw1_holder)
                    );
                    logln!(
                        "[SD] rp1 fw part1 ok: size={} checksum=0x{:08x}",
                        fw2_holder.len(),
                        rp1_image::checksum32(&fw2_holder)
                    );
                    Some(rp1_image::build_from_fw_parts(
                        &fw1_holder,
                        &fw2_holder,
                        fw_scratch,
                    )?)
                }
                (None, _) => {
                    logln!("[RP1IMG] fw part0 not found");
                    None
                }
                (_, None) => {
                    logln!("[RP1IMG] fw part1 not found");
                    None
                }
            }
        }
    };

    if let Some(rp1_image) = rp1_image {
        let source = match rp1_image.source {
            rp1_image::Rp1ImageSource::Rp1Img => "RP1.img",
            rp1_image::Rp1ImageSource::FwParts => "fw-parts",
        };
        logln!("[RP1IMG] source={}", source);
        logln!(
            "[RP1IMG] payload size={} load=0x{:x} entry=0x{:x} stack=0x{:x}",
            rp1_image.payload.len(),
            rp1_image.load_addr,
            rp1_image.entry,
            rp1_image.stack
        );

        let i2c = bcm2712_i2c::Bcm2712I2c::from_dtb_or_fallback(&dtb);
        let run = bcm2712_aon::Rp1RunPin::from_dtb_or_fallback(&dtb);
        let mut bootstrap = rp1_bootstrap::Rp1Bootstrap::new(i2c, run);
        match bootstrap.reset_into_bootrom() {
            Ok(Some(chip_id)) => {
                logln!("[RP1BOOT] chip id = 0x{:08x}", chip_id);
                if let Err(err) = bootstrap.load_and_start(&rp1_image) {
                    handle_rp1_bootstrap_failure(err)?;
                }
            }
            Ok(None) => {
                logln!("[RP1BOOT] chip id unavailable; continuing with write-only bootstrap path");
                if let Err(err) = bootstrap.load_and_start(&rp1_image) {
                    handle_rp1_bootstrap_failure(err)?;
                }
            }
            Err(err) => {
                handle_rp1_bootstrap_failure(err)?;
            }
        }
    } else if !cfg!(feature = "skip-rp1-reload") {
        handle_rp1_bootstrap_failure(BootError::SdFileNotFound)?;
    }

    boot_files::probe_file(sdhc, "/config.txt", "/config.txt after reset")?;

    logln!("[KERNEL] probing raw BCM2712 image paths");
    let bcm2712_raw = read_first_optional_file(
        sdhc,
        &[
            "/BCM2712.img",
            "/BCM2712.IMG",
            "/bcm2712.img",
            "/bcm2712.IMG",
        ],
    )?;
    let (kernel_base, image) = if let Some(raw) = bcm2712_raw {
        placement::copy_to_phys(
            placement::KERNEL_LOAD_BASE,
            placement::KERNEL_MAX_SIZE,
            &raw,
        )?;
        logln!(
            "[KERNEL] /BCM2712.img raw placement base=0x{:x} len={}",
            placement::KERNEL_LOAD_BASE,
            raw.len()
        );
        (
            placement::KERNEL_LOAD_BASE,
            linux::LinuxImage {
                entry: placement::KERNEL_LOAD_BASE,
                image_size: raw.len(),
                text_offset: 0,
                flags: 0,
                image_base: placement::KERNEL_LOAD_BASE,
            },
        )
    } else {
        logln!("[KERNEL] raw BCM2712 image not found; probing /kernel_2712.img");
        let kernel_file = read_first_optional_file(
            sdhc,
            &[
                "/kernel_2712.img",
                "/KERNEL_2712.IMG",
                "/KERNEL~1.IMG",
                "/kernel8.img",
                "/KERNEL8.IMG",
            ],
        )?
        .ok_or(BootError::SdFileNotFound)?;
        logln!("[SD] kernel image selected: size={}", kernel_file.len());
        let kernel = gzip::decompress_kernel_if_needed(
            &kernel_file,
            placement::KERNEL_LOAD_BASE,
            placement::KERNEL_MAX_SIZE,
        )?;
        logln!(
            "[KERNEL] placement base=0x{:x} len={} gzip={}",
            kernel.base,
            kernel.len,
            kernel.was_gzip
        );
        let image =
            linux::validate_arm64_image(kernel.base, kernel.len, placement::KERNEL_MAX_SIZE)?;
        (kernel.base, image)
    };

    let initramfs = read_first_optional_file(
        sdhc,
        &[
            "/initramfs_2712",
            "/INITRAMFS_2712",
            "/INITRA~2",
            "/INITRA~1",
            "/INITRD",
            "/INITRD.IMG",
        ],
    )?
    .ok_or(BootError::SdFileNotFound)?;
    placement::copy_to_phys(
        placement::INITRAMFS_LOAD_BASE,
        placement::INITRAMFS_MAX_SIZE,
        &initramfs,
    )?;
    let initramfs_len = initramfs.len();
    drop(initramfs);
    let initrd_start = placement::INITRAMFS_LOAD_BASE;
    let initrd_end = initrd_start + initramfs_len;

    let cmdline = boot_files::read_optional_file(sdhc, "/cmdline.txt")?;
    linux::quiesce_sdhc_from_dtb_or_fallback(&dtb)?;

    let patched_dtb = dtb_patch::patch_dtb_for_linux(
        &dtb,
        placement::DTB_COPY_BASE,
        placement::DTB_MAX_SIZE,
        initrd_start,
        initrd_end,
        cmdline.as_deref(),
    )?;

    let regs = linux::read_el2_debug_regs();
    logln!(
        "[LINUX] handoff kernel entry=0x{:x} image_size={} text_offset=0x{:x} flags=0x{:x} image_base=0x{:x}",
        image.entry,
        image.image_size,
        image.text_offset,
        image.flags,
        image.image_base
    );
    logln!(
        "[LINUX] handoff dtb=0x{:x} len={} initrd=0x{:x}..0x{:x}",
        patched_dtb.addr,
        patched_dtb.len,
        initrd_start,
        initrd_end
    );
    logln!(
        "[LINUX] EL2 regs before handoff DAIF=0x{:x} CurrentEL=0x{:x} SCTLR_EL2=0x{:x} HCR_EL2=0x{:x} VTTBR_EL2=0x{:x} CNTVOFF_EL2=0x{:x} CPTR_EL2=0x{:x}",
        regs.daif,
        regs.current_el,
        regs.sctlr_el2,
        regs.hcr_el2,
        regs.vttbr_el2,
        regs.cntvoff_el2,
        regs.cptr_el2
    );

    linux::clean_dcache_poc(kernel_base, image.image_size);
    linux::clean_dcache_poc(initrd_start, initramfs_len);
    linux::clean_dcache_poc(patched_dtb.addr, patched_dtb.len);
    linux::invalidate_icache_all();

    // SAFETY: terminal EL2 direct handoff; all boot protocol registers are set in asm.
    unsafe { linux::jump_to_linux_el2(image.entry, patched_dtb.addr) }
}

fn handle_rp1_bootstrap_failure(err: BootError) -> Result<(), BootError> {
    logln!(
        "[RP1BOOT] bootstrap failed: {:?}; refusing Linux handoff unless continue-on-rp1-bootstrap-failure is enabled",
        err
    );
    if cfg!(feature = "continue-on-rp1-bootstrap-failure") {
        logln!("[RP1BOOT] continuing by feature continue-on-rp1-bootstrap-failure");
        Ok(())
    } else {
        Err(err)
    }
}

pub fn fatal(err: BootError) -> ! {
    logln!("[FATAL] {:?}", err);
    halt()
}

fn read_first_optional_file(
    sdhc: &'static dyn BlockDevice,
    paths: &[&str],
) -> Result<Option<allocator::AlignedSliceBox<u8>>, BootError> {
    for path in paths {
        match boot_files::read_optional_file(sdhc, path) {
            Ok(Some(bytes)) => {
                logln!("[SD] selected {}", path);
                return Ok(Some(bytes));
            }
            Ok(None) => {
                logln!("[SD] {} not found", path);
            }
            Err(err) => {
                logln!("[SD] {} error: {:?}", path, err);
                return Err(err);
            }
        }
    }
    Ok(None)
}

pub fn halt() -> ! {
    loop {
        // SAFETY: WFE is the intended low-power fatal loop for this bootloader.
        unsafe {
            core::arch::asm!("wfe", options(nomem, nostack, preserves_flags));
        }
    }
}

#[global_allocator]
static GLOBAL_ALLOCATOR: BumpAllocator = BumpAllocator {
    offset: UnsafeCell::new(0),
    heap: UnsafeCell::new([0; HEAP_SIZE]),
};

const HEAP_SIZE: usize = 80 * 1024 * 1024;

struct BumpAllocator {
    offset: UnsafeCell<usize>,
    heap: UnsafeCell<[u8; HEAP_SIZE]>,
}

unsafe impl Sync for BumpAllocator {}

unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: CoreLayout) -> *mut u8 {
        let align = layout.align();
        let size = layout.size();
        // SAFETY: the PoC runs single-core before Linux handoff, so the bump pointer does not
        // need atomic RMW instructions while MMU/cache attributes are still firmware-defined.
        let current = unsafe { *self.offset.get() };
        let aligned = (current + align - 1) & !(align - 1);
        let next = match aligned.checked_add(size) {
            Some(next) if next <= HEAP_SIZE => next,
            _ => return core::ptr::null_mut(),
        };
        unsafe {
            *self.offset.get() = next;
            (*self.heap.get()).as_mut_ptr().add(aligned)
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: CoreLayout) {
        let heap = unsafe { (*self.heap.get()).as_mut_ptr() };
        let ptr_addr = ptr as usize;
        let heap_addr = heap as usize;
        let Some(offset) = ptr_addr.checked_sub(heap_addr) else {
            return;
        };
        let Some(end) = offset.checked_add(layout.size()) else {
            return;
        };
        let current = unsafe { *self.offset.get() };
        if end == current {
            unsafe {
                *self.offset.get() = offset;
            }
        }
    }
}

#[alloc_error_handler]
fn alloc_error(layout: Layout) -> ! {
    logln!(
        "[ALLOC] allocation failed size={} align={}",
        layout.size(),
        layout.align()
    );
    halt()
}

pub mod timer {
    pub fn init() {}

    pub fn delay_micros(us: u64) {
        let mut count = us.saturating_mul(200);
        while count != 0 {
            // SAFETY: single-cycle hint used only for bounded delay loops.
            unsafe {
                core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
            }
            count -= 1;
        }
    }
}

#[cfg(feature = "log-uart")]
pub mod uart {
    use core::fmt;
    use core::fmt::Write;

    const UART_BASE: usize = 0x10_7d00_1000;
    const UART_DR: usize = 0x00;
    const UART_FR: usize = 0x18;
    const UART_FR_TXFF: u32 = 1 << 5;

    pub fn init() {}

    pub fn puts(s: &str) {
        for b in s.bytes() {
            putc(b);
        }
    }

    pub fn putc(b: u8) {
        if b == b'\n' {
            putc(b'\r');
        }
        for _ in 0..100_000 {
            if (read32(UART_FR) & UART_FR_TXFF) == 0 {
                break;
            }
        }
        write32(UART_DR, u32::from(b));
    }

    pub fn _print(args: fmt::Arguments<'_>) {
        let _ = Writer.write_fmt(args);
    }

    struct Writer;

    impl fmt::Write for Writer {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            puts(s);
            Ok(())
        }
    }

    fn read32(off: usize) -> u32 {
        // SAFETY: debug UART base follows the Raspberry Pi 5 EL2 PoC mapping.
        unsafe { core::ptr::read_volatile((UART_BASE + off) as *const u32) }
    }

    fn write32(off: usize, value: u32) {
        // SAFETY: debug UART base follows the Raspberry Pi 5 EL2 PoC mapping.
        unsafe { core::ptr::write_volatile((UART_BASE + off) as *mut u32, value) }
    }
}

#[cfg(feature = "log-semihosting")]
pub mod semihosting {
    use core::fmt;
    use core::fmt::Write;

    const SYS_WRITE0: usize = 0x04;

    pub fn init() {}

    pub fn puts(s: &str) {
        let mut buf = [0u8; 128];
        let mut len = 0usize;
        for b in s.bytes() {
            if len + 2 >= buf.len() {
                write0(&mut buf, len);
                len = 0;
            }
            if b == b'\n' {
                buf[len] = b'\r';
                len += 1;
            }
            buf[len] = b;
            len += 1;
        }
        if len != 0 {
            write0(&mut buf, len);
        }
    }

    fn write0(buf: &mut [u8; 128], len: usize) {
        buf[len] = 0;
        // SAFETY: AArch64 semihosting uses x0 for the operation, x1 for the argument
        // pointer, and `hlt #0xf000` as the trap. SYS_WRITE0 reads a NUL-terminated
        // byte string from x1.
        unsafe {
            core::arch::asm!(
                "hlt #0xf000",
                in("x0") SYS_WRITE0,
                in("x1") buf.as_ptr() as usize,
                options(nostack, preserves_flags)
            );
        }
    }

    pub fn _print(args: fmt::Arguments<'_>) {
        let _ = Writer.write_fmt(args);
    }

    struct Writer;

    impl fmt::Write for Writer {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            puts(s);
            Ok(())
        }
    }
}

pub mod logging {
    use core::fmt;

    #[cfg(feature = "log-uart")]
    pub fn init() {
        crate::uart::init();
    }

    #[cfg(feature = "log-semihosting")]
    pub fn init() {
        crate::semihosting::init();
    }

    #[cfg(feature = "log-uart")]
    pub fn puts(s: &str) {
        crate::trace::puts(s);
        crate::uart::puts(s);
    }

    #[cfg(feature = "log-semihosting")]
    pub fn puts(s: &str) {
        crate::trace::puts(s);
        crate::semihosting::puts(s);
    }

    #[cfg(feature = "log-uart")]
    pub fn _print(args: fmt::Arguments<'_>) {
        crate::trace::write_fmt(args);
        crate::uart::_print(args);
    }

    #[cfg(feature = "log-semihosting")]
    pub fn _print(args: fmt::Arguments<'_>) {
        crate::trace::write_fmt(args);
        crate::semihosting::_print(args);
    }
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::logging::_print(core::format_args!($($arg)*))
    };
}

#[macro_export]
macro_rules! logln {
    () => {
        $crate::logging::puts("\n")
    };
    ($fmt:literal $(, $($arg:tt)+)?) => {
        $crate::logging::_print(core::format_args!(concat!($fmt, "\n") $(, $($arg)+)?))
    };
}
