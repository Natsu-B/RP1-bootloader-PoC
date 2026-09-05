#![no_std]
#![no_main]
#![feature(alloc_error_handler)]

extern crate alloc;

use alloc::alloc::Layout;
use core::alloc::{GlobalAlloc, Layout as CoreLayout};
use core::arch::global_asm;
use core::cell::UnsafeCell;

#[cfg(not(feature = "tftp-boot"))]
use arch_hal::soc::bcm2712;
use block_device_api::BlockDevice;
use dtb::DtbParser;

mod bcm2712_aon;
mod bcm2712_i2c;
mod boot_context;
mod boot_files;
mod dhcp_boot;
mod dtb_patch;
mod gzip;
mod hash;
mod linux;
mod net_boot;
mod panic;
mod placement;
mod rp1_bootstrap;
#[cfg(feature = "rp1-clock-independence-proof")]
mod rp1_clock_independence;
mod rp1_config;
mod rp1_dtb_policy;
mod rp1_image;
#[cfg(feature = "rp1-inbound-monitor-block-proof")]
mod rp1_inbound_monitor;
mod rp1_note;

const RP1_ELF_PATHS: &[&str] = &["/RP1.elf", "/rp1/RP1.elf", "/rp1/rp1.elf", "/RP1/RP1.ELF"];

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

#[cfg(all(
    feature = "rp1-clock-independence-proof",
    feature = "rp1-inbound-monitor-block-proof"
))]
compile_error!(
    "features `rp1-clock-independence-proof` and `rp1-inbound-monitor-block-proof` are mutually exclusive"
);

#[cfg(all(feature = "rp1-gpio22-start-proof", feature = "rp1-gdb-debug-stub"))]
compile_error!("rp1-gpio22-start-proof must halt before host-side RP1 PCIe reinitialization");

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
    Rp1Pcie,
    Rp1Gem,
    Tftp,
    MissingRp1Note,
    InvalidRp1Note,
    Rp1ConfigInvalid,
    Rp1DtbPolicyInvalid,
    Rp1DtbNodeNotFound,
    Dhcp,
    DhcpTimeout,
    DhcpInvalidPacket,
    DhcpNoTftpServer,
    BootModeDtbNodeMissing,
    BootModeDtbNodeInvalid,
    FirmwareBootedFromSdOrEmmc,
    FirmwareBootedFromUsbMsd,
    FirmwareBootModeUnsupported,
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
    logln!(
        "[TIMER] generic timer freq={} Hz",
        timer::counter_frequency_hz()
    );
    logln!("[TLS] init skipped: static BSP TLS symbols preserved by linker");
    logln!("[EXC] vector init skipped for PoC bringup");

    let dtb = DtbParser::init(placement::DTB_PTR).map_err(|_| BootError::DtbPatch)?;
    logln!("[DTB] parse ok");
    #[cfg(feature = "tftp-boot")]
    {
        match boot_context::FirmwareBootContext::from_dtb(&dtb) {
            Ok(ctx) => ctx.log(),
            Err(err) => logln!("[BOOTCTX] unavailable: {:?}", err),
        }
    }
    #[cfg(not(feature = "tftp-boot"))]
    let default_boot_ctx = {
        let boot_ctx = boot_context::FirmwareBootContext::from_dtb(&dtb)?;
        boot_ctx.log();
        boot_ctx.enforce_default_policy()?;
        boot_ctx
    };
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

    #[cfg(not(feature = "tftp-boot"))]
    {
        if default_boot_ctx.boot_mode == boot_context::FirmwareBootMode::Network {
            return net_boot::boot_from_tftp_with_dhcp(&dtb);
        }
    }

    #[cfg(feature = "tftp-boot")]
    {
        return net_boot::boot_from_tftp(&dtb);
    }

    #[cfg(not(feature = "tftp-boot"))]
    {
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

        let rp1_elf_file;
        let rp1_img_file;
        let fw1_holder;
        let fw2_holder;
        let mut rp1_policy = None;
        let rp1_image = if cfg!(feature = "skip-rp1-reload") {
            logln!("[RP1BOOT] skipped by feature skip-rp1-reload");
            None
        } else {
            rp1_elf_file = read_first_optional_file_with_path(sdhc, RP1_ELF_PATHS)?;
            if rp1_elf_file.is_some() {
                logln!("[SD] /RP1.elf found");
            } else {
                logln!("[SD] /RP1.elf not found");
            }
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
            if let Some((elf_path, ref elf_bytes)) = rp1_elf_file {
                log_rp1_elf_file_selection(elf_path, elf_bytes)?;
                rp1_policy = Some(enforce_rp1_elf_note_policy_from_sd(sdhc, elf_bytes)?);
                let image = rp1_image::build_from_rp1_elf(
                    elf_bytes,
                    fw_scratch,
                    rp1_image::RP1_FALLBACK_STACK,
                )?;
                logln!(
                    "[RP1ELF] load_base=0x{:x} image_len={} entry=0x{:x} stack=0x{:x}",
                    image.load_addr,
                    image.payload.len(),
                    image.entry,
                    image.stack
                );
                log_rp1_elf_materialized(&image);
                Some(image)
            } else if let Some(ref image_bytes) = rp1_img_file {
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
                rp1_image::Rp1ImageSource::Rp1Elf => "RP1.elf",
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
                    logln!(
                        "[RP1BOOT] chip id unavailable; continuing with write-only bootstrap path"
                    );
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
            rp1_policy.as_ref(),
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

/// Loads and starts RP1 firmware selected from the SD boot partition.
///
/// This is also used by the TFTP kernel path: AArch64 kernel transport and RP1
/// firmware source remain separate policies for the first combined bring-up.
pub(crate) fn boot_rp1_from_sd(
    sdhc: &'static dyn BlockDevice,
    dtb: &DtbParser,
) -> Result<(), BootError> {
    boot_files::probe_file(sdhc, "/config.txt", "/config.txt before RP1 reset")?;
    if cfg!(feature = "skip-rp1-reload") {
        logln!("[RP1BOOT] skipped by feature skip-rp1-reload");
        return Ok(());
    }

    let rp1_elf_file = read_first_optional_file_with_path(sdhc, RP1_ELF_PATHS)?;
    if rp1_elf_file.is_some() {
        logln!("[SD] /RP1.elf found");
    }
    let rp1_img_file = read_first_optional_file(
        sdhc,
        &["/RP1.img", "/rp1/RP1.img", "/rp1/rp1.img", "/RP1/RP1.IMG"],
    )?;
    let scratch = placement::rp1_scratch_slice();
    let image = if let Some((elf_path, ref elf_bytes)) = rp1_elf_file {
        log_rp1_elf_file_selection(elf_path, elf_bytes)?;
        let _policy = enforce_rp1_elf_note_policy_from_sd(sdhc, elf_bytes)?;
        let image =
            rp1_image::build_from_rp1_elf(elf_bytes, scratch, rp1_image::RP1_FALLBACK_STACK)?;
        logln!(
            "[RP1ELF] load_base=0x{:x} image_len={} entry=0x{:x} stack=0x{:x}",
            image.load_addr,
            image.payload.len(),
            image.entry,
            image.stack
        );
        log_rp1_elf_materialized(&image);
        Some(image)
    } else if let Some(ref image_bytes) = rp1_img_file {
        Some(rp1_image::parse_rp1_img(image_bytes)?)
    } else {
        if cfg!(feature = "require-rp1-img") {
            return Err(BootError::SdFileNotFound);
        }
        let fw1 = read_first_optional_file(
            sdhc,
            &[
                "/rp1c0fw1.bin",
                "/rp1/rp1c0fw1.bin",
                "/RP1/FW1.BIN",
                "/RP1C0FW1.BIN",
            ],
        )?;
        let fw2 = read_first_optional_file(
            sdhc,
            &[
                "/rp1c0fw2.bin",
                "/rp1/rp1c0fw2.bin",
                "/RP1/FW2.BIN",
                "/RP1C0FW2.BIN",
            ],
        )?;
        match (fw1, fw2) {
            (Some(fw1), Some(fw2)) => Some(rp1_image::build_from_fw_parts(&fw1, &fw2, scratch)?),
            _ => None,
        }
    };
    let Some(image) = image else {
        return handle_rp1_bootstrap_failure(BootError::SdFileNotFound);
    };
    start_rp1_image(dtb, &image)?;
    boot_files::probe_file(sdhc, "/config.txt", "/config.txt after RP1 reset")?;
    Ok(())
}

pub(crate) fn start_rp1_image(
    dtb: &DtbParser,
    image: &rp1_image::Rp1Image<'_>,
) -> Result<(), BootError> {
    start_rp1_image_with_debug_sram(dtb, image, None)
}

pub(crate) fn start_rp1_image_with_debug_sram(
    dtb: &DtbParser,
    image: &rp1_image::Rp1Image<'_>,
    debug_sram: Option<(usize, usize)>,
) -> Result<(), BootError> {
    let source = match image.source {
        rp1_image::Rp1ImageSource::Rp1Elf => "RP1.elf",
        rp1_image::Rp1ImageSource::Rp1Img => "RP1.img",
        rp1_image::Rp1ImageSource::FwParts => "fw-parts",
    };
    logln!("[RP1IMG] source={}", source);
    let i2c = bcm2712_i2c::Bcm2712I2c::from_dtb_or_fallback(dtb);
    let run = bcm2712_aon::Rp1RunPin::from_dtb_or_fallback(dtb);
    let mut bootstrap = rp1_bootstrap::Rp1Bootstrap::new(i2c, run);
    #[cfg(feature = "rp1-gdb-debug-stub")]
    log_rp1_pcie_audit(dtb, "pre-rp1-reload");
    match bootstrap.reset_into_bootrom() {
        Ok(Some(chip_id)) => logln!("[RP1BOOT] chip id = 0x{:08x}", chip_id),
        Ok(None) => {
            logln!("[RP1BOOT] chip id unavailable; continuing with write-only bootstrap path")
        }
        Err(err) => return handle_rp1_bootstrap_failure(err),
    }
    #[cfg(feature = "rp1-gdb-debug-stub")]
    {
        log_rp1_pcie_audit(dtb, "after-rp1-reset");
        log_rp1_pcie_audit(dtb, "after-rp1-bootrom-probe");
    }
    #[cfg(feature = "rp1-gdb-debug-stub")]
    {
        if let Err(err) = bootstrap.load_image(image) {
            handle_rp1_bootstrap_failure(err)?;
        }
        logln!("[RP1BOOT] image loaded");
        log_rp1_pcie_audit(dtb, "after-rp1-image-load");
        if let Err(err) = bootstrap.program_scratch(image.entry, image.stack) {
            handle_rp1_bootstrap_failure(err)?;
        }
        logln!("[RP1BOOT] scratch programmed");
        if let Err(err) = bootstrap.start() {
            handle_rp1_bootstrap_failure(err)?;
        }
        logln!("[RP1BOOT] proc0 started");
    }
    #[cfg(not(feature = "rp1-gdb-debug-stub"))]
    {
        if let Err(err) = bootstrap.load_and_start(image) {
            handle_rp1_bootstrap_failure(err)?;
        }
        #[cfg(feature = "rp1-gpio22-start-proof")]
        {
            logln!("[RP1STARTPROOF] proc0 started; host halted before PCIe initialization");
            halt();
        }
    }
    #[cfg(feature = "rp1-gdb-debug-stub")]
    {
        let mut debug_sram = debug_sram;
        if let Some((sram_base, sram_size)) = debug_sram {
            let mut transport =
                rp1_bootstrap::rp1_debug_stub::Rp1PcieTransport::new(sram_base, sram_size);
            transport.log_probe("after-rp1-proc0-start");
            transport.log_phase_readback("after-rp1-proc0-start");
        }
        for (attempt, delay_ms) in [10u64, 100, 500, 1_000].into_iter().enumerate() {
            crate::timer::delay_millis(delay_ms);
            let phase = match delay_ms {
                10 => "after-rp1-proc0-start+10ms",
                100 => "after-rp1-proc0-start+100ms",
                500 => "after-rp1-proc0-start+500ms",
                _ => "after-rp1-proc0-start+1000ms",
            };
            log_rp1_pcie_audit(dtb, phase);
            if let Some((sram_base, sram_size)) = debug_sram {
                let mut transport =
                    rp1_bootstrap::rp1_debug_stub::Rp1PcieTransport::new(sram_base, sram_size);
                transport.log_probe(phase);
                transport.log_phase_readback(phase);
            }
            match arch_hal::soc::bcm2712::init_rp1_with_options(
                dtb,
                arch_hal::soc::bcm2712::Rp1InitOptions {
                    mode: arch_hal::soc::bcm2712::Rp1InitMode::Auto,
                    strict: false,
                },
            ) {
                Ok(rp1) => {
                    logln!(
                        "[RP1PCIE:post-rp1-reload-reinit] attempt={} result=success",
                        attempt
                    );
                    log_rp1_pcie_raw_diag("post-rp1-reload-reinit");
                    log_rp1_pcie_config_dump(&rp1, "post-rp1-reload-reinit");
                    if let Some((base, size)) = rp1.shared_sram_addr {
                        match (usize::try_from(base), usize::try_from(size)) {
                            (Ok(base), Ok(size)) => {
                                logln!(
                                    "[RP1GDB] post-start shared SRAM BAR cpu=0x{:x} size=0x{:x} attempt={}",
                                    base,
                                    size,
                                    attempt
                                );
                                logln!(
                                    "[RP1PCIE:post-rp1-reload-reinit] bar2_cpu=0x{:x} size=0x{:x}",
                                    base,
                                    size
                                );
                                debug_sram = Some((base, size));
                                let mut transport =
                                    rp1_bootstrap::rp1_debug_stub::Rp1PcieTransport::new(
                                        base, size,
                                    );
                                transport.log_probe("post-rp1-reload-reinit");
                                transport.log_phase_readback("post-rp1-reload-reinit");
                                log_rp1_clock_host_alias_snapshot(&rp1, "post-rp1-reload-reinit");
                                log_rp1_reset_host_alias_snapshot(&rp1, "post-rp1-reload-reinit");
                                crate::timer::delay_millis(500);
                                transport.log_probe("post-rp1-reload-reinit+500ms");
                                transport.log_phase_readback("post-rp1-reload-reinit+500ms");
                                transport.log_pll_core_lock_result("post-rp1-reload-reinit+500ms");
                                #[cfg(feature = "rp1-boot-rom-dump")]
                                transport.log_boot_rom_dump("post-rp1-reload-reinit+500ms");
                                log_rp1_clock_host_alias_snapshot(
                                    &rp1,
                                    "post-rp1-reload-reinit+500ms",
                                );
                                log_rp1_reset_host_alias_snapshot(
                                    &rp1,
                                    "post-rp1-reload-reinit+500ms",
                                );
                                log_rp1_uart0_host_alias_snapshot(
                                    &rp1,
                                    "post-rp1-reload-reinit+500ms",
                                );
                                log_rp1_i2c1_host_alias_snapshot(
                                    &rp1,
                                    "post-rp1-reload-reinit+500ms",
                                );
                                log_rp1_spi0_host_alias_snapshot(
                                    &rp1,
                                    "post-rp1-reload-reinit+500ms",
                                );
                                #[cfg(feature = "rp1-clock-independence-proof")]
                                {
                                    rp1_clock_independence::run_after_full_init(&rp1);
                                    halt();
                                }
                                #[cfg(all(
                                    not(feature = "rp1-clock-independence-proof"),
                                    feature = "rp1-inbound-monitor-block-proof"
                                ))]
                                {
                                    rp1_inbound_monitor::run_after_full_init(&rp1);
                                    halt();
                                }
                                #[cfg(not(any(
                                    feature = "rp1-clock-independence-proof",
                                    feature = "rp1-inbound-monitor-block-proof"
                                )))]
                                break;
                            }
                            _ => logln!("[RP1GDB] post-start shared SRAM BAR conversion failed"),
                        }
                    } else {
                        logln!("[RP1GDB] post-start shared SRAM BAR missing");
                    }
                }
                Err(err) => {
                    logln!(
                        "[RP1GDB] post-start RP1 PCIe init retry {} failed: {:?}",
                        attempt,
                        err
                    );
                    logln!(
                        "[RP1PCIE:post-rp1-reload-reinit] attempt={} result={:?}",
                        attempt,
                        err
                    );
                }
            }
        }
        let Some((sram_base, sram_size)) = debug_sram else {
            logln!("[RP1GDB] shared SRAM BAR unavailable");
            if cfg!(feature = "rp1-linux-observe-failure") {
                logln!(
                    "[RP1LINUXOBS] continuing to Linux handoff after RP1 post-reload PCIe failure"
                );
                return Ok(());
            }
            return Err(BootError::Rp1Pcie);
        };
        logln!(
            "[RP1GDB] transport=pcie-sram sram_base=0x{:x} size=0x{:x}",
            sram_base,
            sram_size
        );
        if cfg!(feature = "rp1-linux-observe-failure") {
            logln!("[RP1LINUXOBS] RP1 PCIe recovered; continuing to Linux handoff");
            return Ok(());
        }
        let mut transport =
            rp1_bootstrap::rp1_debug_stub::Rp1PcieTransport::new(sram_base, sram_size);
        rp1_bootstrap::rp1_debug_stub::serve_with_transport(&mut transport);
    }
    #[cfg(not(feature = "rp1-gdb-debug-stub"))]
    Ok(())
}

#[cfg(feature = "rp1-gdb-debug-stub")]
fn log_rp1_pcie_audit(dtb: &DtbParser, label: &'static str) {
    logln!("[RP1PCIE:{}] audit begin", label);
    log_rp1_pcie_raw_diag(label);
    match arch_hal::soc::bcm2712::init_rp1_with_options(
        dtb,
        arch_hal::soc::bcm2712::Rp1InitOptions {
            mode: arch_hal::soc::bcm2712::Rp1InitMode::AuditOnly,
            strict: false,
        },
    ) {
        Ok(rp1) => {
            logln!("[RP1PCIE:{}] audit result=success", label);
            arch_hal::soc::bcm2712::dump_rp1_pcie_diagnostics(&rp1);
        }
        Err(err) => logln!("[RP1PCIE:{}] audit result={:?}", label, err),
    }
}

#[cfg(feature = "rp1-gdb-debug-stub")]
fn log_rp1_pcie_config_dump(rp1: &arch_hal::soc::bcm2712::Rp1Config, label: &'static str) {
    const RP1_PERIPHERAL_BASE: u64 = 0x4000_0000;
    const RP1_PCIE_CFG_BASE: u64 = 0x4010_8000;
    let Some((peripheral_base, peripheral_size)) = rp1.peripheral_addr else {
        logln!("[RP1PCIECFG] {} peripheral BAR missing", label);
        return;
    };
    let Some(offset) = RP1_PCIE_CFG_BASE.checked_sub(RP1_PERIPHERAL_BASE) else {
        logln!("[RP1PCIECFG] {} invalid cfg offset", label);
        return;
    };
    let Some(end) = offset.checked_add(0x80) else {
        logln!("[RP1PCIECFG] {} cfg offset overflow", label);
        return;
    };
    if end > peripheral_size {
        logln!(
            "[RP1PCIECFG] {} cfg outside BAR peripheral=0x{:x} size=0x{:x} offset=0x{:x}",
            label,
            peripheral_base,
            peripheral_size,
            offset
        );
        return;
    }
    let Some(cpu_base) = peripheral_base.checked_add(offset) else {
        logln!("[RP1PCIECFG] {} cfg cpu alias overflow", label);
        return;
    };
    logln!(
        "[RP1PCIECFG] {} peripheral_base=0x{:x} size=0x{:x} cfg_cpu=0x{:x}",
        label,
        peripheral_base,
        peripheral_size,
        cpu_base
    );
    for row in (0..0x80usize).step_by(16) {
        unsafe {
            let a = core::ptr::read_volatile((cpu_base as usize + row) as *const u32);
            let b = core::ptr::read_volatile((cpu_base as usize + row + 4) as *const u32);
            let c = core::ptr::read_volatile((cpu_base as usize + row + 8) as *const u32);
            let d = core::ptr::read_volatile((cpu_base as usize + row + 12) as *const u32);
            logln!(
                "[RP1PCIECFG] {} +0x{:03x}: {:08x} {:08x} {:08x} {:08x}",
                label,
                row,
                a,
                b,
                c,
                d
            );
        }
    }
}

#[cfg(feature = "rp1-gdb-debug-stub")]
fn log_rp1_clock_host_alias_snapshot(rp1: &arch_hal::soc::bcm2712::Rp1Config, label: &'static str) {
    const RP1_PERIPHERAL_BASE: u64 = 0x4000_0000;
    const PLL_SYS_BASE: u64 = 0x4002_0000;
    const CLK_UART_BASE: u64 = 0x4001_8054;
    const PLL_SYS_WINDOW_SIZE: u64 = 0x18;
    const CLK_UART_WINDOW_SIZE: u64 = 0x08;

    let Some((peripheral_base, peripheral_size)) = rp1.peripheral_addr else {
        logln!("[RP1CLKHOST] {} peripheral BAR missing", label);
        return;
    };
    let Some(pll_offset) = PLL_SYS_BASE.checked_sub(RP1_PERIPHERAL_BASE) else {
        logln!("[RP1CLKHOST] {} invalid PLL_SYS offset", label);
        return;
    };
    let Some(clk_uart_offset) = CLK_UART_BASE.checked_sub(RP1_PERIPHERAL_BASE) else {
        logln!("[RP1CLKHOST] {} invalid CLK_UART offset", label);
        return;
    };
    let Some(pll_end) = pll_offset.checked_add(PLL_SYS_WINDOW_SIZE) else {
        logln!("[RP1CLKHOST] {} PLL_SYS offset overflow", label);
        return;
    };
    let Some(clk_uart_end) = clk_uart_offset.checked_add(CLK_UART_WINDOW_SIZE) else {
        logln!("[RP1CLKHOST] {} CLK_UART offset overflow", label);
        return;
    };
    if pll_end > peripheral_size || clk_uart_end > peripheral_size {
        logln!(
            "[RP1CLKHOST] {} clock block outside BAR peripheral=0x{:x} size=0x{:x}",
            label,
            peripheral_base,
            peripheral_size
        );
        return;
    }
    let Some(pll_cpu) = peripheral_base.checked_add(pll_offset) else {
        logln!("[RP1CLKHOST] {} PLL_SYS CPU alias overflow", label);
        return;
    };
    let Some(clk_uart_cpu) = peripheral_base.checked_add(clk_uart_offset) else {
        logln!("[RP1CLKHOST] {} CLK_UART CPU alias overflow", label);
        return;
    };
    let (Ok(pll_cpu), Ok(clk_uart_cpu)) = (usize::try_from(pll_cpu), usize::try_from(clk_uart_cpu))
    else {
        logln!("[RP1CLKHOST] {} clock CPU alias conversion failed", label);
        return;
    };

    unsafe {
        let pll_sys_cs = core::ptr::read_volatile(pll_cpu as *const u32);
        let pll_sys_pwr = core::ptr::read_volatile((pll_cpu + 0x04) as *const u32);
        let pll_sys_fbdiv_int = core::ptr::read_volatile((pll_cpu + 0x08) as *const u32);
        let pll_sys_fbdiv_frac = core::ptr::read_volatile((pll_cpu + 0x0c) as *const u32);
        let pll_sys_prim = core::ptr::read_volatile((pll_cpu + 0x10) as *const u32);
        let pll_sys_sec = core::ptr::read_volatile((pll_cpu + 0x14) as *const u32);
        let clk_uart_ctrl = core::ptr::read_volatile(clk_uart_cpu as *const u32);
        let clk_uart_div_int = core::ptr::read_volatile((clk_uart_cpu + 0x04) as *const u32);
        logln!(
            "[RP1CLKHOST] {} pll_cpu=0x{:x} PLL_SYS_CS={:08x} PLL_SYS_PWR={:08x} PLL_SYS_PRIM={:08x} bit4={}",
            label,
            pll_cpu,
            pll_sys_cs,
            pll_sys_pwr,
            pll_sys_prim,
            (pll_sys_prim >> 4) & 1
        );
        logln!(
            "[RP1CLKHOST] {} clk_uart_cpu=0x{:x} CLK_UART_CTRL={:08x} CLK_UART_DIV_INT={:08x}",
            label,
            clk_uart_cpu,
            clk_uart_ctrl,
            clk_uart_div_int
        );
        logln!(
            "[RP1CLKHOST] {} PLL_SYS_FBDIV_INT={:08x} PLL_SYS_FBDIV_FRAC={:08x} PLL_SYS_SEC={:08x}",
            label,
            pll_sys_fbdiv_int,
            pll_sys_fbdiv_frac,
            pll_sys_sec
        );
    }
}

#[cfg(feature = "rp1-gdb-debug-stub")]
fn log_rp1_reset_host_alias_snapshot(rp1: &arch_hal::soc::bcm2712::Rp1Config, label: &'static str) {
    const RP1_PERIPHERAL_BASE: u64 = 0x4000_0000;
    const RESETS_BASE: u64 = 0x4001_4000;
    const RESETS_WINDOW_SIZE: u64 = 0x24;

    let Some((peripheral_base, peripheral_size)) = rp1.peripheral_addr else {
        logln!("[RP1RESETHOST] {} peripheral BAR missing", label);
        return;
    };
    let Some(reset_offset) = RESETS_BASE.checked_sub(RP1_PERIPHERAL_BASE) else {
        logln!("[RP1RESETHOST] {} invalid reset offset", label);
        return;
    };
    let Some(reset_end) = reset_offset.checked_add(RESETS_WINDOW_SIZE) else {
        logln!("[RP1RESETHOST] {} reset offset overflow", label);
        return;
    };
    if reset_end > peripheral_size {
        logln!(
            "[RP1RESETHOST] {} reset block outside BAR peripheral=0x{:x} size=0x{:x}",
            label,
            peripheral_base,
            peripheral_size
        );
        return;
    }
    let Some(reset_cpu) = peripheral_base.checked_add(reset_offset) else {
        logln!("[RP1RESETHOST] {} reset CPU alias overflow", label);
        return;
    };
    let Ok(reset_cpu) = usize::try_from(reset_cpu) else {
        logln!("[RP1RESETHOST] {} reset CPU alias conversion failed", label);
        return;
    };

    unsafe {
        let ctrl = core::ptr::read_volatile(reset_cpu as *const u32);
        let ctrl1 = core::ptr::read_volatile((reset_cpu + 0x04) as *const u32);
        let ctrl2 = core::ptr::read_volatile((reset_cpu + 0x08) as *const u32);
        let done = core::ptr::read_volatile((reset_cpu + 0x18) as *const u32);
        let done1 = core::ptr::read_volatile((reset_cpu + 0x1c) as *const u32);
        let done2 = core::ptr::read_volatile((reset_cpu + 0x20) as *const u32);
        logln!(
            "[RP1RESETHOST] {} reset_cpu=0x{:x} CTRL={:08x} DONE={:08x} ctrl29={} done29={} ctrl26={} done26={}",
            label,
            reset_cpu,
            ctrl,
            done,
            (ctrl >> 29) & 1,
            (done >> 29) & 1,
            (ctrl >> 26) & 1,
            (done >> 26) & 1
        );
        logln!(
            "[RP1RESETHOST] {} CTRL0={:08x} CTRL1={:08x} CTRL2={:08x} DONE0={:08x} DONE1={:08x} DONE2={:08x}",
            label,
            ctrl,
            ctrl1,
            ctrl2,
            done,
            done1,
            done2
        );
        logln!(
            "[RP1RESETHOST] {} UART0 bank=1 bit=26 ctrl={} done={}",
            label,
            (ctrl1 >> 26) & 1,
            (done1 >> 26) & 1
        );
    }
}

#[cfg(feature = "rp1-gdb-debug-stub")]
fn log_rp1_uart0_host_alias_snapshot(rp1: &arch_hal::soc::bcm2712::Rp1Config, label: &'static str) {
    const RP1_PERIPHERAL_BASE: u64 = 0x4000_0000;
    const RP1_UART0_BASE: u64 = 0x4003_0000;
    const UART0_WINDOW_SIZE: u64 = 0x1000;

    let Some((peripheral_base, peripheral_size)) = rp1.peripheral_addr else {
        logln!("[RP1UART0HOST] {} peripheral BAR missing", label);
        return;
    };
    let Some(offset) = RP1_UART0_BASE.checked_sub(RP1_PERIPHERAL_BASE) else {
        logln!("[RP1UART0HOST] {} invalid UART0 offset", label);
        return;
    };
    let Some(end) = offset.checked_add(UART0_WINDOW_SIZE) else {
        logln!("[RP1UART0HOST] {} UART0 offset overflow", label);
        return;
    };
    if end > peripheral_size {
        logln!(
            "[RP1UART0HOST] {} UART0 outside BAR peripheral=0x{:x} size=0x{:x} offset=0x{:x}",
            label,
            peripheral_base,
            peripheral_size,
            offset
        );
        return;
    }
    let Some(cpu_base) = peripheral_base.checked_add(offset) else {
        logln!("[RP1UART0HOST] {} UART0 CPU alias overflow", label);
        return;
    };
    let Ok(cpu_base) = usize::try_from(cpu_base) else {
        logln!("[RP1UART0HOST] {} UART0 CPU alias conversion failed", label);
        return;
    };

    unsafe {
        let rsr = core::ptr::read_volatile((cpu_base + 0x04) as *const u32);
        let fr = core::ptr::read_volatile((cpu_base + 0x18) as *const u32);
        let ibrd = core::ptr::read_volatile((cpu_base + 0x24) as *const u32);
        let fbrd = core::ptr::read_volatile((cpu_base + 0x28) as *const u32);
        let lcr_h = core::ptr::read_volatile((cpu_base + 0x2c) as *const u32);
        let cr = core::ptr::read_volatile((cpu_base + 0x30) as *const u32);
        let imsc = core::ptr::read_volatile((cpu_base + 0x38) as *const u32);
        let ris = core::ptr::read_volatile((cpu_base + 0x3c) as *const u32);
        let mis = core::ptr::read_volatile((cpu_base + 0x40) as *const u32);
        let icr = core::ptr::read_volatile((cpu_base + 0x44) as *const u32);
        let pid0 = core::ptr::read_volatile((cpu_base + 0xfe0) as *const u32);
        let pid1 = core::ptr::read_volatile((cpu_base + 0xfe4) as *const u32);
        let pid2 = core::ptr::read_volatile((cpu_base + 0xfe8) as *const u32);
        let pid3 = core::ptr::read_volatile((cpu_base + 0xfec) as *const u32);
        let cid0 = core::ptr::read_volatile((cpu_base + 0xff0) as *const u32);
        let cid1 = core::ptr::read_volatile((cpu_base + 0xff4) as *const u32);
        let cid2 = core::ptr::read_volatile((cpu_base + 0xff8) as *const u32);
        let cid3 = core::ptr::read_volatile((cpu_base + 0xffc) as *const u32);
        logln!(
            "[RP1UART0HOST] {} cpu=0x{:x} RSR={:08x} FR={:08x} IBRD={:08x} FBRD={:08x}",
            label,
            cpu_base,
            rsr,
            fr,
            ibrd,
            fbrd
        );
        logln!(
            "[RP1UART0HOST] {} LCR_H={:08x} CR={:08x} IMSC={:08x} RIS={:08x} MIS={:08x} ICR={:08x}",
            label,
            lcr_h,
            cr,
            imsc,
            ris,
            mis,
            icr
        );
        logln!(
            "[RP1UART0HOST] {} PID={:02x}:{:02x}:{:02x}:{:02x} CID={:02x}:{:02x}:{:02x}:{:02x}",
            label,
            pid3 & 0xff,
            pid2 & 0xff,
            pid1 & 0xff,
            pid0 & 0xff,
            cid3 & 0xff,
            cid2 & 0xff,
            cid1 & 0xff,
            cid0 & 0xff
        );
    }
}

#[cfg(feature = "rp1-gdb-debug-stub")]
fn log_rp1_i2c1_host_alias_snapshot(rp1: &arch_hal::soc::bcm2712::Rp1Config, label: &'static str) {
    const RP1_PERIPHERAL_BASE: u64 = 0x4000_0000;
    const RP1_I2C1_BASE: u64 = 0x4007_4000;
    const I2C1_WINDOW_SIZE: u64 = 0x1000;

    let Some((peripheral_base, peripheral_size)) = rp1.peripheral_addr else {
        logln!("[RP1I2C1HOST] {} peripheral BAR missing", label);
        return;
    };
    let Some(offset) = RP1_I2C1_BASE.checked_sub(RP1_PERIPHERAL_BASE) else {
        logln!("[RP1I2C1HOST] {} invalid I2C1 offset", label);
        return;
    };
    let Some(end) = offset.checked_add(I2C1_WINDOW_SIZE) else {
        logln!("[RP1I2C1HOST] {} I2C1 offset overflow", label);
        return;
    };
    if end > peripheral_size {
        logln!(
            "[RP1I2C1HOST] {} I2C1 outside BAR peripheral=0x{:x} size=0x{:x} offset=0x{:x}",
            label,
            peripheral_base,
            peripheral_size,
            offset
        );
        return;
    }
    let Some(cpu_base) = peripheral_base.checked_add(offset) else {
        logln!("[RP1I2C1HOST] {} I2C1 CPU alias overflow", label);
        return;
    };
    let Ok(cpu_base) = usize::try_from(cpu_base) else {
        logln!("[RP1I2C1HOST] {} I2C1 CPU alias conversion failed", label);
        return;
    };

    unsafe {
        let con = core::ptr::read_volatile(cpu_base as *const u32);
        let tar = core::ptr::read_volatile((cpu_base + 0x04) as *const u32);
        let ss_hcnt = core::ptr::read_volatile((cpu_base + 0x14) as *const u32);
        let ss_lcnt = core::ptr::read_volatile((cpu_base + 0x18) as *const u32);
        let intr_stat = core::ptr::read_volatile((cpu_base + 0x2c) as *const u32);
        let intr_mask = core::ptr::read_volatile((cpu_base + 0x30) as *const u32);
        let raw_intr = core::ptr::read_volatile((cpu_base + 0x34) as *const u32);
        let rx_tl = core::ptr::read_volatile((cpu_base + 0x38) as *const u32);
        let tx_tl = core::ptr::read_volatile((cpu_base + 0x3c) as *const u32);
        let enable = core::ptr::read_volatile((cpu_base + 0x6c) as *const u32);
        let status = core::ptr::read_volatile((cpu_base + 0x70) as *const u32);
        let txflr = core::ptr::read_volatile((cpu_base + 0x74) as *const u32);
        let rxflr = core::ptr::read_volatile((cpu_base + 0x78) as *const u32);
        let sda_hold = core::ptr::read_volatile((cpu_base + 0x7c) as *const u32);
        let tx_abrt_source = core::ptr::read_volatile((cpu_base + 0x80) as *const u32);
        let enable_status = core::ptr::read_volatile((cpu_base + 0x9c) as *const u32);
        let comp_param = core::ptr::read_volatile((cpu_base + 0xf4) as *const u32);
        let comp_version = core::ptr::read_volatile((cpu_base + 0xf8) as *const u32);
        let comp_type = core::ptr::read_volatile((cpu_base + 0xfc) as *const u32);
        logln!(
            "[RP1I2C1HOST] {} cpu=0x{:x} CON={:08x} TAR={:08x} SS_HCNT={:08x} SS_LCNT={:08x}",
            label,
            cpu_base,
            con,
            tar,
            ss_hcnt,
            ss_lcnt
        );
        logln!(
            "[RP1I2C1HOST] {} INTR_STAT={:08x} INTR_MASK={:08x} RAW_INTR={:08x} RX_TL={:08x} TX_TL={:08x}",
            label,
            intr_stat,
            intr_mask,
            raw_intr,
            rx_tl,
            tx_tl
        );
        logln!(
            "[RP1I2C1HOST] {} ENABLE={:08x} ENABLE_STATUS={:08x} STATUS={:08x} TXFLR={:08x} RXFLR={:08x}",
            label,
            enable,
            enable_status,
            status,
            txflr,
            rxflr
        );
        logln!(
            "[RP1I2C1HOST] {} SDA_HOLD={:08x} TX_ABRT_SOURCE={:08x} COMP_PARAM={:08x} COMP_VERSION={:08x} COMP_TYPE={:08x}",
            label,
            sda_hold,
            tx_abrt_source,
            comp_param,
            comp_version,
            comp_type
        );
    }
}

#[cfg(feature = "rp1-gdb-debug-stub")]
fn log_rp1_spi0_host_alias_snapshot(rp1: &arch_hal::soc::bcm2712::Rp1Config, label: &'static str) {
    const RP1_PERIPHERAL_BASE: u64 = 0x4000_0000;
    const RP1_SPI0_BASE: u64 = 0x4005_0000;
    const SPI0_WINDOW_SIZE: u64 = 0x130;

    let Some((peripheral_base, peripheral_size)) = rp1.peripheral_addr else {
        logln!("[RP1SPI0HOST] {} peripheral BAR missing", label);
        return;
    };
    let Some(offset) = RP1_SPI0_BASE.checked_sub(RP1_PERIPHERAL_BASE) else {
        logln!("[RP1SPI0HOST] {} invalid SPI0 offset", label);
        return;
    };
    let Some(end) = offset.checked_add(SPI0_WINDOW_SIZE) else {
        logln!("[RP1SPI0HOST] {} SPI0 offset overflow", label);
        return;
    };
    if end > peripheral_size {
        logln!(
            "[RP1SPI0HOST] {} SPI0 block outside BAR peripheral=0x{:x} size=0x{:x} offset=0x{:x}",
            label,
            peripheral_base,
            peripheral_size,
            offset
        );
        return;
    }
    let Some(cpu_base) = peripheral_base.checked_add(offset) else {
        logln!("[RP1SPI0HOST] {} SPI0 CPU alias overflow", label);
        return;
    };
    let Ok(cpu_base) = usize::try_from(cpu_base) else {
        logln!("[RP1SPI0HOST] {} SPI0 CPU alias conversion failed", label);
        return;
    };

    unsafe {
        let ctrlr0 = core::ptr::read_volatile(cpu_base as *const u32);
        let ctrlr1 = core::ptr::read_volatile((cpu_base + 0x04) as *const u32);
        let ssienr = core::ptr::read_volatile((cpu_base + 0x08) as *const u32);
        let ser = core::ptr::read_volatile((cpu_base + 0x10) as *const u32);
        let baudr = core::ptr::read_volatile((cpu_base + 0x14) as *const u32);
        let txftlr = core::ptr::read_volatile((cpu_base + 0x18) as *const u32);
        let rxftlr = core::ptr::read_volatile((cpu_base + 0x1c) as *const u32);
        let txflr = core::ptr::read_volatile((cpu_base + 0x20) as *const u32);
        let rxflr = core::ptr::read_volatile((cpu_base + 0x24) as *const u32);
        let sr = core::ptr::read_volatile((cpu_base + 0x28) as *const u32);
        let imr = core::ptr::read_volatile((cpu_base + 0x2c) as *const u32);
        let isr = core::ptr::read_volatile((cpu_base + 0x30) as *const u32);
        let risr = core::ptr::read_volatile((cpu_base + 0x34) as *const u32);
        let dmacr = core::ptr::read_volatile((cpu_base + 0x4c) as *const u32);
        let idr = core::ptr::read_volatile((cpu_base + 0x58) as *const u32);
        let version = core::ptr::read_volatile((cpu_base + 0x5c) as *const u32);
        let rx_sample_dly = core::ptr::read_volatile((cpu_base + 0xf0) as *const u32);
        let cs_override = core::ptr::read_volatile((cpu_base + 0xf4) as *const u32);
        logln!(
            "[RP1SPI0HOST] {} cpu=0x{:x} CTRLR0={:08x} CTRLR1={:08x} SSIENR={:08x} SER={:08x} BAUDR={:08x}",
            label,
            cpu_base,
            ctrlr0,
            ctrlr1,
            ssienr,
            ser,
            baudr
        );
        logln!(
            "[RP1SPI0HOST] {} TXFTLR={:08x} RXFTLR={:08x} TXFLR={:08x} RXFLR={:08x} SR={:08x}",
            label,
            txftlr,
            rxftlr,
            txflr,
            rxflr,
            sr
        );
        logln!(
            "[RP1SPI0HOST] {} IMR={:08x} ISR={:08x} RISR={:08x} DMACR={:08x}",
            label,
            imr,
            isr,
            risr,
            dmacr
        );
        logln!(
            "[RP1SPI0HOST] {} IDR={:08x} VERSION={:08x} RX_SAMPLE_DLY={:08x} CS_OVERRIDE={:08x}",
            label,
            idr,
            version,
            rx_sample_dly,
            cs_override
        );
    }
}

#[cfg(feature = "rp1-gdb-debug-stub")]
pub(crate) fn log_rp1_pcie_raw_diag(label: &'static str) {
    const PCIE_BASE: usize = 0x10_0012_0000;
    const REG_PCIE_CTRL: usize = 0x4064;
    const REG_PCIE_STATUS: usize = 0x4068;
    const REG_CONFIG_DATA: usize = 0x8000;
    const REG_CONFIG_ADDRESS: usize = 0x9000;
    const RP1_BDF: u32 = 0x0010_0000;

    unsafe {
        let ctrl = core::ptr::read_volatile((PCIE_BASE + REG_PCIE_CTRL) as *const u32);
        let status = core::ptr::read_volatile((PCIE_BASE + REG_PCIE_STATUS) as *const u32);
        logln!(
            "[RP1PCIE:{}] root ctrl=0x{:08x} status=0x{:08x} link_up={}",
            label,
            ctrl,
            status,
            (status & 0x30) == 0x30
        );
        core::ptr::write_volatile((PCIE_BASE + REG_CONFIG_ADDRESS) as *mut u32, RP1_BDF);
        core::arch::asm!("dsb sy", options(nostack, preserves_flags));
        let cfg = PCIE_BASE + REG_CONFIG_DATA;
        let id = core::ptr::read_volatile((cfg + 0x00) as *const u32);
        let cmd_status = core::ptr::read_volatile((cfg + 0x04) as *const u32);
        let class_rev = core::ptr::read_volatile((cfg + 0x08) as *const u32);
        let bhlc = core::ptr::read_volatile((cfg + 0x0c) as *const u32);
        let bar0 = core::ptr::read_volatile((cfg + 0x10) as *const u32);
        let bar1 = core::ptr::read_volatile((cfg + 0x14) as *const u32);
        let bar2 = core::ptr::read_volatile((cfg + 0x18) as *const u32);
        logln!(
            "[RP1PCIE:{}] ep_id=0x{:08x} cmd_status=0x{:08x} class_rev=0x{:08x} bhlc=0x{:08x}",
            label,
            id,
            cmd_status,
            class_rev,
            bhlc
        );
        logln!(
            "[RP1PCIE:{}] bar0=0x{:08x} bar1=0x{:08x} bar2=0x{:08x}",
            label,
            bar0,
            bar1,
            bar2
        );
    }
}

fn enforce_rp1_elf_note_policy_from_sd(
    sdhc: &'static dyn BlockDevice,
    elf_bytes: &[u8],
) -> Result<rp1_dtb_policy::Rp1DtbPolicy, BootError> {
    let cfg_file = if matches!(
        rp1_note::parse_rp1_note(elf_bytes),
        rp1_note::Rp1NoteState::Missing
    ) {
        boot_files::read_optional_file(sdhc, "/config_rp1.txt")?
    } else {
        None
    };
    enforce_rp1_elf_note_policy_with_config(elf_bytes, cfg_file.as_deref())
}

pub(crate) fn enforce_rp1_elf_note_policy_with_config(
    elf_bytes: &[u8],
    cfg_file: Option<&[u8]>,
) -> Result<rp1_dtb_policy::Rp1DtbPolicy, BootError> {
    match rp1_note::parse_rp1_note(elf_bytes) {
        rp1_note::Rp1NoteState::Valid(note) => {
            logln!(
                "[RP1NOTE] valid: owner_rp1=0x{:x} owner_linux=0x{:x} owner_disabled=0x{:x} mailbox=0x{:x} version_kind={}",
                note.owner_rp1,
                note.owner_linux,
                note.owner_disabled,
                note.mailbox_flags,
                note.firmware_version_kind,
            );
            let policy = rp1_dtb_policy::Rp1DtbPolicy::from_note(&note)?;
            log_rp1_note_policy_owners(&policy);
            Ok(policy)
        }
        rp1_note::Rp1NoteState::Missing => {
            let cfg = rp1_config::parse_optional_config(cfg_file)
                .map_err(|_| BootError::Rp1ConfigInvalid)?;
            if cfg.force_boot {
                logln!(
                    "[RP1NOTE] missing; using explicit /config_rp1.txt debug override force_boot=true"
                );
                let policy = rp1_dtb_policy::Rp1DtbPolicy::from_config(&cfg)?;
                log_rp1_note_policy_owners(&policy);
                Ok(policy)
            } else {
                logln!(
                    "[RP1NOTE] missing; refusing Linux handoff without valid .note.rp1 or explicit config override"
                );
                Err(BootError::MissingRp1Note)
            }
        }
        rp1_note::Rp1NoteState::Invalid => {
            logln!("[RP1NOTE] invalid; refusing RP1 reload/Linux handoff");
            Err(BootError::InvalidRp1Note)
        }
    }
}

fn log_rp1_note_policy_owners(policy: &rp1_dtb_policy::Rp1DtbPolicy) {
    for spec in rp1_dtb_policy::RP1_DEVICE_DTB_NODES {
        logln!(
            "[RP1NOTE] owner {}={}",
            spec.name,
            policy.owner_of(spec.bit).as_str()
        );
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

fn read_first_optional_file_with_path(
    sdhc: &'static dyn BlockDevice,
    paths: &'static [&'static str],
) -> Result<Option<(&'static str, allocator::AlignedSliceBox<u8>)>, BootError> {
    for path in paths {
        match boot_files::read_optional_file(sdhc, path) {
            Ok(Some(bytes)) => {
                logln!("[SD] selected {}", path);
                return Ok(Some((path, bytes)));
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

pub(crate) fn log_rp1_elf_file_selection(
    path: &str,
    elf_bytes: &[u8],
) -> Result<rp1_image::Rp1ElfInfo, BootError> {
    let file_digest = hash::sha256_bytes(elf_bytes);
    logln!("[RP1SRC] selected=ELF path={}", path);
    timer::delay_millis(2);
    logln!("[RP1ELF] len=0x{:x}", elf_bytes.len());
    timer::delay_millis(2);
    hash::log_sha256_len("rp1.selected.file", &file_digest, elf_bytes.len());
    hash::log_sha256_len("rp1.elf.full", &file_digest, elf_bytes.len());
    logln!("[RP1ELF] file_sha256={}", hash::Sha256Hex(&file_digest));
    timer::delay_millis(2);

    let info = rp1_image::inspect_rp1_elf(elf_bytes)?;
    logln!(
        "[RP1ELF] entry=0x{:08x} sp=0x{:08x}",
        info.entry,
        info.vector0_sp
    );
    timer::delay_millis(2);
    logln!("[RP1ELF] e_entry=0x{:08x}", info.entry);
    timer::delay_millis(2);
    logln!("[RP1ELF] vector0_sp=0x{:08x}", info.vector0_sp);
    timer::delay_millis(2);
    logln!("[RP1ELF] vector1_reset=0x{:08x}", info.vector1_reset);
    timer::delay_millis(2);
    logln!("[RP1ELF] phnum={}", info.phnum);
    timer::delay_millis(2);
    for (index, load) in info.loads().iter().enumerate() {
        logln!(
            "[RP1ELF] load[{}] off=0x{:x} vaddr=0x{:08x} paddr=0x{:08x} filesz=0x{:x} memsz=0x{:x} flags=0x{:x} align=0x{:x}",
            index,
            load.file_offset,
            load.vaddr,
            load.paddr,
            load.filesz,
            load.memsz,
            load.flags,
            load.align
        );
        timer::delay_millis(2);
        if index == 0 {
            logln!(
                "[RP1ELF] load0 off=0x{:x} addr=0x{:08x} size=0x{:x}",
                load.file_offset,
                load.paddr,
                load.filesz
            );
            timer::delay_millis(2);
            let ptload = rp1_image::elf_load_file_bytes(elf_bytes, load)?;
            let ptload_digest = hash::sha256_bytes(ptload);
            hash::log_sha256_len("rp1.elf.ptload.0.file", &ptload_digest, ptload.len());
        }
    }
    Ok(info)
}

pub(crate) fn log_rp1_elf_materialized(image: &rp1_image::Rp1Image<'_>) {
    let materialized_digest = hash::sha256_bytes(image.payload);
    hash::log_sha256_len(
        "rp1.materialized",
        &materialized_digest,
        image.payload.len(),
    );
    logln!(
        "[RP1ELF] start entry=0x{:08x} stack=0x{:08x} thumb={}",
        image.entry,
        image.stack,
        image.entry & 1
    );
    timer::delay_millis(2);
    logln!(
        "[RP1ELF] start entry=0x{:08x} sp=0x{:08x}",
        image.entry,
        image.stack
    );
    timer::delay_millis(2);
}

pub(crate) fn apply_rp1_elf_start_contract(
    image: &mut rp1_image::Rp1Image<'_>,
    config: &rp1_config::Rp1Config,
) -> Result<(), BootError> {
    if config.rp1_start_contract_explicit {
        let entry = config
            .rp1_entry_override
            .ok_or(BootError::Rp1ConfigInvalid)?
            | 1;
        let stack = config
            .rp1_stack_override
            .ok_or(BootError::Rp1ConfigInvalid)?;
        if entry == 1 || stack == 0 || !rp1_image::is_valid_stack(stack) {
            return Err(BootError::Rp1ConfigInvalid);
        }
        image.entry = entry;
        image.stack = stack;
        logln!("[RP1ELF] start_contract=explicit");
        timer::delay_millis(2);
        logln!("[RP1ELF] start_entry=0x{:08x} source=override", entry);
        timer::delay_millis(2);
        logln!("[RP1ELF] start_stack=0x{:08x} source=override", stack);
        timer::delay_millis(2);
    } else {
        logln!("[RP1ELF] start_contract=default");
        timer::delay_millis(2);
        logln!("[RP1ELF] start_entry=0x{:08x} source=elf", image.entry);
        timer::delay_millis(2);
        logln!("[RP1ELF] start_stack=0x{:08x} source=elf", image.stack);
        timer::delay_millis(2);
    }
    Ok(())
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
    use core::cell::UnsafeCell;
    use core::sync::atomic::{AtomicBool, Ordering};
    use core::time::Duration;

    use arch_timer::SystemTimer;

    struct TimerCell(UnsafeCell<SystemTimer>);

    // SAFETY: This bootloader runs timer initialization/use on the boot CPU during
    // early single-threaded bringup. No concurrent mutable access is expected.
    unsafe impl Sync for TimerCell {}

    static TIMER: TimerCell = TimerCell(UnsafeCell::new(SystemTimer::new()));
    static INITIALIZED: AtomicBool = AtomicBool::new(false);

    pub fn init() {
        if INITIALIZED.load(Ordering::Acquire) {
            return;
        }

        // SAFETY: Early boot single-core initialization. See TimerCell Sync comment.
        unsafe {
            (*TIMER.0.get()).init();
        }

        INITIALIZED.store(true, Ordering::Release);
    }

    pub fn delay_micros(us: u64) {
        ensure_init();

        // SAFETY: Early boot single-core use. No concurrent mutable access occurs.
        unsafe {
            (*TIMER.0.get()).wait(Duration::from_micros(us));
        }
    }

    pub fn delay_millis(ms: u64) {
        ensure_init();

        // SAFETY: Early boot single-core use. No concurrent mutable access occurs.
        unsafe {
            (*TIMER.0.get()).wait(Duration::from_millis(ms));
        }
    }

    pub fn counter_frequency_hz() -> u64 {
        ensure_init();

        // SAFETY: Read-only access after initialization.
        unsafe { (*TIMER.0.get()).counter_frequency_hz().get() }
    }

    fn ensure_init() {
        if !INITIALIZED.load(Ordering::Acquire) {
            init();
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
