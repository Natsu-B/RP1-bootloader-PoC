//! TFTP boot glue for the RP1 GEM polling driver.
//!
//! This path is feature-gated because it is a hardware integration consumer of
//! the reusable `net` crate.  It does not alter the SDHC boot path.

use alloc::vec;
use arch_hal::soc::bcm2712;
use arch_hal::soc::bcm2712::rp1_gem::Rp1Gem;
use arch_hal::soc::bcm2712::rp1_gem::Rp1GemOptions;
use io_api::ethernet::MacAddr;
use net::Ipv4Addr;
use net::tftp;

use crate::BootError;
use crate::dtb_patch;
use crate::linux;
use crate::placement;

const TFTP_LOCAL_MAC: MacAddr = MacAddr([0x2c, 0xcf, 0x67, 0xc2, 0x9a, 0x58]);
const TFTP_LOCAL_IP: Ipv4Addr = [192, 168, 3, 2];
const TFTP_SERVER_IP: Ipv4Addr = [192, 168, 3, 1];
const TFTP_KERNEL_FILENAME: &str = "BCM2712.img";
#[cfg(feature = "tftp-initramfs")]
const TFTP_INITRAMFS_FILENAME: &str = "initramfs_2712";
const TFTP_TIMEOUT_US: u64 = 3_000_000;
const TFTP_MAX_RETRIES: usize = 3;
const TFTP_KERNEL_STAGING_MAX: usize = 32 * 1024 * 1024;

struct TimerClock {
    ticks_per_us: u64,
}

impl TimerClock {
    fn new() -> Self {
        Self {
            ticks_per_us: core::cmp::max(1, crate::timer::counter_frequency_hz() / 1_000_000),
        }
    }
}

impl tftp::TftpClock for TimerClock {
    fn now_us(&self) -> u64 {
        arch_timer::read_counter() / self.ticks_per_us
    }
}

/// Downloads configured Linux artifacts through RP1 GEM and enters the normal
/// EL2 handoff sequence only after complete, validated downloads.
pub fn boot_from_tftp(dtb: &dtb::DtbParser) -> Result<(), BootError> {
    crate::logln!(
        "[TFTP] config mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} local={}.{}.{}.{} server={}.{}.{}.{} kernel={} timeout_us={} retries={}",
        TFTP_LOCAL_MAC.0[0],
        TFTP_LOCAL_MAC.0[1],
        TFTP_LOCAL_MAC.0[2],
        TFTP_LOCAL_MAC.0[3],
        TFTP_LOCAL_MAC.0[4],
        TFTP_LOCAL_MAC.0[5],
        TFTP_LOCAL_IP[0],
        TFTP_LOCAL_IP[1],
        TFTP_LOCAL_IP[2],
        TFTP_LOCAL_IP[3],
        TFTP_SERVER_IP[0],
        TFTP_SERVER_IP[1],
        TFTP_SERVER_IP[2],
        TFTP_SERVER_IP[3],
        TFTP_KERNEL_FILENAME,
        TFTP_TIMEOUT_US,
        TFTP_MAX_RETRIES
    );
    let rp1 = bcm2712::init_rp1_with_options(
        dtb,
        bcm2712::Rp1InitOptions {
            mode: bcm2712::Rp1InitMode::Auto,
            strict: false,
        },
    )
    .map_err(|err| {
        crate::logln!("[TFTP] RP1 PCIe init failed: {:?}", err);
        BootError::Rp1Pcie
    })?;
    crate::logln!("[TFTP] RP1 PCIe init ok");

    let gem = Rp1Gem::init_from_rp1_config(&rp1, TFTP_LOCAL_MAC, Rp1GemOptions::default())
        .map_err(|err| {
            crate::logln!("[TFTP] Rp1Gem init failed: {:?}", err);
            BootError::Rp1Gem
        })?;
    crate::logln!("[TFTP] Rp1Gem init ok phy={}", gem.phy_address());
    match gem.link_state() {
        Ok(link) => crate::logln!(
            "[TFTP] link up={} speed={:?} full_duplex={}",
            link.up,
            link.speed,
            link.full_duplex
        ),
        Err(err) => crate::logln!("[TFTP] link query failed: {:?}", err),
    }

    let clock = TimerClock::new();
    let kernel_cfg = tftp_config(TFTP_KERNEL_FILENAME);
    crate::logln!("[TFTP] kernel download start {}", TFTP_KERNEL_FILENAME);
    let mut kernel_staging = vec![0u8; TFTP_KERNEL_STAGING_MAX];
    let kernel_len = match tftp::download_into(gem, &clock, &kernel_cfg, &mut kernel_staging) {
        Ok(len) => len,
        Err(err) => return gem_failure(gem, "kernel download", err),
    };
    crate::logln!(
        "[TFTP] kernel download complete addr=0x{:x} len={}",
        placement::KERNEL_LOAD_BASE,
        kernel_len
    );
    let kernel = crate::gzip::decompress_kernel_if_needed(
        &kernel_staging[..kernel_len],
        placement::KERNEL_LOAD_BASE,
        placement::KERNEL_MAX_SIZE,
    )?;
    drop(kernel_staging);
    let image = linux::validate_arm64_image(kernel.base, kernel.len, placement::KERNEL_MAX_SIZE)?;

    #[cfg(feature = "tftp-initramfs")]
    let initramfs_len = download_initramfs(gem, &clock)?;
    #[cfg(not(feature = "tftp-initramfs"))]
    let initramfs_len = {
        crate::logln!("[TFTP] initramfs disabled; patching an empty initrd range");
        0
    };
    let initrd_start = if initramfs_len == 0 {
        0
    } else {
        placement::INITRAMFS_LOAD_BASE
    };
    let initrd_end = initrd_start
        .checked_add(initramfs_len)
        .ok_or(BootError::AddressOverflow)?;

    let patched_dtb = dtb_patch::patch_dtb_for_linux(
        dtb,
        placement::DTB_COPY_BASE,
        placement::DTB_MAX_SIZE,
        initrd_start,
        initrd_end,
        None,
    )?;
    let regs = linux::read_el2_debug_regs();
    crate::logln!(
        "[LINUX] TFTP handoff entry=0x{:x} image_size={} dtb=0x{:x} len={} initrd=0x{:x}..0x{:x}",
        image.entry,
        image.image_size,
        patched_dtb.addr,
        patched_dtb.len,
        initrd_start,
        initrd_end
    );
    crate::logln!(
        "[LINUX] EL2 regs before TFTP handoff DAIF=0x{:x} CurrentEL=0x{:x} SCTLR_EL2=0x{:x} HCR_EL2=0x{:x} VTTBR_EL2=0x{:x} CNTVOFF_EL2=0x{:x} CPTR_EL2=0x{:x}",
        regs.daif,
        regs.current_el,
        regs.sctlr_el2,
        regs.hcr_el2,
        regs.vttbr_el2,
        regs.cntvoff_el2,
        regs.cptr_el2
    );

    gem.quiesce();
    crate::logln!("[TFTP] Rp1Gem quiesce complete");
    linux::clean_dcache_poc(kernel.base, image.image_size);
    linux::clean_dcache_poc(initrd_start, initramfs_len);
    linux::clean_dcache_poc(patched_dtb.addr, patched_dtb.len);
    linux::invalidate_icache_all();

    // SAFETY: all downloaded artifacts were bounded, kernel header validated,
    // DTB was patched into its reserved range, GEM was quiesced, and cache
    // maintenance completed before the terminal EL2 handoff.
    unsafe { linux::jump_to_linux_el2(image.entry, patched_dtb.addr) }
}

#[cfg(feature = "tftp-initramfs")]
fn download_initramfs(gem: &mut Rp1Gem, clock: &TimerClock) -> Result<usize, BootError> {
    let config = tftp_config(TFTP_INITRAMFS_FILENAME);
    crate::logln!(
        "[TFTP] initramfs download start {}",
        TFTP_INITRAMFS_FILENAME
    );
    let len = match tftp::download_into(
        gem,
        clock,
        &config,
        physical_output(
            placement::INITRAMFS_LOAD_BASE,
            placement::INITRAMFS_MAX_SIZE,
        ),
    ) {
        Ok(len) => len,
        Err(err) => return gem_failure(gem, "initramfs download", err),
    };
    crate::logln!(
        "[TFTP] initramfs download complete addr=0x{:x} len={}",
        placement::INITRAMFS_LOAD_BASE,
        len
    );
    Ok(len)
}

fn tftp_config(filename: &'static str) -> tftp::TftpConfig<'static> {
    tftp::TftpConfig {
        local_ip: TFTP_LOCAL_IP,
        server_ip: TFTP_SERVER_IP,
        server_port: tftp::TFTP_PORT,
        filename,
        timeout_us: TFTP_TIMEOUT_US,
        max_retries: TFTP_MAX_RETRIES,
    }
}

fn gem_failure<T>(
    gem: &mut Rp1Gem,
    stage: &'static str,
    err: tftp::TftpError,
) -> Result<T, BootError> {
    crate::logln!("[TFTP] {} failed: {:?}", stage, err);
    crate::logln!("[TFTP] Rp1Gem diagnostic: {:?}", gem.diagnostic_snapshot());
    crate::logln!("[TFTP] Rp1Gem last error: {:?}", gem.take_last_error());
    Err(BootError::Tftp)
}

fn physical_output(addr: usize, len: usize) -> &'static mut [u8] {
    // SAFETY: `main_flow` checked the complete reserved kernel/initramfs ranges
    // for overlap before entering this feature-gated path.  The TFTP client
    // writes only within this bounded slice and the PoC is single-core here.
    unsafe { core::slice::from_raw_parts_mut(addr as *mut u8, len) }
}
