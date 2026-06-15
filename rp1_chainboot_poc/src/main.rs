#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

extern crate alloc;

use alloc::alloc::Layout;
use core::alloc::{GlobalAlloc, Layout as CoreLayout};
use core::arch::global_asm;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

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

    let _rp1_cfg = bcm2712::init_rp1(&dtb).map_err(|_| BootError::El2HandoffPreparationFailure)?;
    logln!("[RP1] init_rp1 ok");
    logln!("[RP1] existing RP1 visible");

    let sdhc: &'static dyn BlockDevice =
        bcm2712::sdhc::init_from_dtb(&dtb).map_err(|_| BootError::SdMount)?;
    logln!("[SDHC] init ok");

    boot_files::probe_file(sdhc, "/config.txt", "/config.txt before reset")?;

    let rp1_img_file = boot_files::read_optional_file(sdhc, "/RP1.img")?;
    if rp1_img_file.is_some() {
        logln!("[SD] /RP1.img found");
    } else {
        logln!("[SD] /RP1.img not found");
    }

    let fw_scratch = placement::rp1_scratch_slice();
    let fw1_holder;
    let fw2_holder;
    let rp1_image = if let Some(ref image_bytes) = rp1_img_file {
        let image = rp1_image::parse_rp1_img(image_bytes)?;
        logln!(
            "[SD] /RP1.img ok: payload={} load=0x{:x} entry=0x{:x} stack=0x{:x}",
            image.payload.len(),
            image.load_addr,
            image.entry,
            image.stack
        );
        image
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
        fw1_holder = boot_files::read_required_file(sdhc, "/rp1c0fw1.bin")?;
        logln!(
            "[SD] /rp1c0fw1.bin ok: size={} checksum=0x{:08x}",
            fw1_holder.len(),
            rp1_image::checksum32(&fw1_holder)
        );
        fw2_holder = boot_files::read_required_file(sdhc, "/rp1c0fw2.bin")?;
        logln!(
            "[SD] /rp1c0fw2.bin ok: size={} checksum=0x{:08x}",
            fw2_holder.len(),
            rp1_image::checksum32(&fw2_holder)
        );
        rp1_image::build_from_fw_parts(&fw1_holder, &fw2_holder, fw_scratch)?
    };

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
    match bootstrap.reset_into_bootrom()? {
        Some(chip_id) => logln!("[RP1BOOT] chip id = 0x{:08x}", chip_id),
        None => logln!("[RP1BOOT] chip id unavailable; continuing with write-only bootstrap path"),
    }
    bootstrap.load_and_start(&rp1_image)?;

    boot_files::probe_file(sdhc, "/config.txt", "/config.txt after reset")?;

    let kernel_file = boot_files::read_required_file(sdhc, "/kernel_2712.img")?;
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
    let image = linux::validate_arm64_image(kernel.base, kernel.len)?;

    let initramfs = boot_files::read_required_file(sdhc, "/initramfs_2712")?;
    placement::copy_to_phys(
        placement::INITRAMFS_LOAD_BASE,
        placement::INITRAMFS_MAX_SIZE,
        &initramfs,
    )?;
    let initrd_start = placement::INITRAMFS_LOAD_BASE;
    let initrd_end = initrd_start + initramfs.len();

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

    linux::clean_dcache_poc(kernel.base, image.image_size);
    linux::clean_dcache_poc(initrd_start, initramfs.len());
    linux::clean_dcache_poc(patched_dtb.addr, patched_dtb.len);
    linux::invalidate_icache_all();

    // SAFETY: terminal EL2 direct handoff; all boot protocol registers are set in asm.
    unsafe { linux::jump_to_linux_el2(image.entry, patched_dtb.addr) }
}

pub fn fatal(err: BootError) -> ! {
    logln!("[FATAL] {:?}", err);
    halt()
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
    offset: AtomicUsize::new(0),
    heap: UnsafeCell::new([0; HEAP_SIZE]),
};

const HEAP_SIZE: usize = 8 * 1024 * 1024;

struct BumpAllocator {
    offset: AtomicUsize,
    heap: UnsafeCell<[u8; HEAP_SIZE]>,
}

unsafe impl Sync for BumpAllocator {}

unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: CoreLayout) -> *mut u8 {
        let align = layout.align();
        let size = layout.size();
        let mut current = self.offset.load(Ordering::Relaxed);
        loop {
            let aligned = (current + align - 1) & !(align - 1);
            let next = match aligned.checked_add(size) {
                Some(next) if next <= HEAP_SIZE => next,
                _ => return core::ptr::null_mut(),
            };
            match self
                .offset
                .compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => {
                    // SAFETY: `aligned..next` has been reserved by the atomic bump pointer.
                    return unsafe { (*self.heap.get()).as_mut_ptr().add(aligned) };
                }
                Err(observed) => current = observed,
            }
        }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: CoreLayout) {}
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

    const SYS_WRITEC: usize = 0x03;

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
        let byte = b;
        // SAFETY: AArch64 semihosting uses x0 for the operation, x1 for the argument
        // pointer, and `hlt #0xf000` as the trap. SYS_WRITEC reads one byte from x1.
        unsafe {
            core::arch::asm!(
                "hlt #0xf000",
                in("x0") SYS_WRITEC,
                in("x1") &byte as *const u8 as usize,
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
        crate::uart::puts(s);
    }

    #[cfg(feature = "log-semihosting")]
    pub fn puts(s: &str) {
        crate::semihosting::puts(s);
    }

    #[cfg(feature = "log-uart")]
    pub fn _print(args: fmt::Arguments<'_>) {
        crate::uart::_print(args);
    }

    #[cfg(feature = "log-semihosting")]
    pub fn _print(args: fmt::Arguments<'_>) {
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
