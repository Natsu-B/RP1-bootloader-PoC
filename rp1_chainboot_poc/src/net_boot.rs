//! TFTP boot glue for the RP1 GEM polling driver.
//!
//! The default build enters this path when firmware booted from the network;
//! the `tftp-boot` feature also uses it as its transport implementation.

use alloc::vec;
use alloc::vec::Vec;
use arch_hal::soc::bcm2712;
use arch_hal::soc::bcm2712::rp1_gem::Rp1Gem;
use arch_hal::soc::bcm2712::rp1_gem::Rp1GemOptions;
use io_api::ethernet::{EthernetFrameIo, MacAddr};
use net::tftp;

use crate::BootError;
use crate::dhcp_boot::{self, DhcpError, NetworkBootLease};
use crate::dtb_patch;
use crate::linux;
use crate::placement;
use crate::rp1_dtb_policy::Rp1DtbPolicy;

const TFTP_LOCAL_MAC: MacAddr = MacAddr([0x2c, 0xcf, 0x67, 0xc2, 0x9a, 0x58]);
const TFTP_KERNEL_FILENAME: &str = "BCM2712.img";
const TFTP_RP1_ELF_FILENAME: &str = "RP1.elf";
const TFTP_RP1_CONFIG_FILENAME: &str = "config_rp1.txt";
const TFTP_FLUSH_FILENAME: &str = "__rp1_tftp_flush__";
#[cfg(feature = "tftp-initramfs")]
const TFTP_INITRAMFS_FILENAME: &str = "initramfs_2712";
const TFTP_TIMEOUT_US: u64 = 3_000_000;
const TFTP_MAX_RETRIES: usize = 3;
const TFTP_LOCAL_PORT_BASE: u16 = 49_152;
const TFTP_RP1_ELF_STAGING_MAX: usize = 512 * 1024;
const TFTP_RP1_CONFIG_STAGING_MAX: usize = 4096;
const TFTP_KERNEL_STAGING_MAX: usize = 32 * 1024 * 1024;

struct TftpSessionPorts {
    next: u16,
}

impl TftpSessionPorts {
    const fn new() -> Self {
        Self {
            next: TFTP_LOCAL_PORT_BASE,
        }
    }

    fn alloc(&mut self) -> u16 {
        let port = self.next;
        self.next = self.next.wrapping_add(1);
        if self.next == 0 {
            self.next = TFTP_LOCAL_PORT_BASE;
        }
        port
    }
}

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
#[cfg(feature = "tftp-boot")]
pub fn boot_from_tftp(dtb: &dtb::DtbParser) -> Result<(), BootError> {
    boot_from_tftp_with_dhcp(dtb)
}

pub fn boot_from_tftp_with_dhcp(dtb: &dtb::DtbParser) -> Result<(), BootError> {
    let clock = TimerClock::new();
    let skip_rp1_reload = cfg!(feature = "skip-rp1-reload");
    let mut ports = TftpSessionPorts::new();

    if skip_rp1_reload {
        let gem = init_tftp_gem(dtb)?;
        let lease = dhcp_boot::dhcp_acquire(&mut *gem, &clock).map_err(|err| {
            crate::logln!("[DHCP] failed: {:?}", err);
            map_dhcp_error(err)
        })?;
        let rp1_policy =
            download_rp1_policy_and_reload_if_needed(dtb, &mut *gem, &clock, &lease, &mut ports)?;
        return boot_kernel_from_tftp_with_lease(
            dtb, &mut *gem, &clock, &lease, rp1_policy, &mut ports,
        );
    }

    let (lease, rp1_policy) = {
        let gem = init_tftp_gem(dtb)?;
        let lease = dhcp_boot::dhcp_acquire(&mut *gem, &clock).map_err(|err| {
            crate::logln!("[DHCP] failed: {:?}", err);
            map_dhcp_error(err)
        })?;
        let rp1_policy =
            download_rp1_policy_and_reload_if_needed(dtb, &mut *gem, &clock, &lease, &mut ports)?;
        (lease, rp1_policy)
    };

    crate::logln!("[TFTP] reinitializing GEM after RP1 reload");
    let gem = init_tftp_gem_with_label(dtb, "post-rp1-reload")?;
    boot_kernel_from_tftp_with_lease(dtb, &mut *gem, &clock, &lease, rp1_policy, &mut ports)
}

fn download_rp1_policy_and_reload_if_needed(
    dtb: &dtb::DtbParser,
    gem: &mut Rp1Gem,
    clock: &TimerClock,
    lease: &NetworkBootLease,
    ports: &mut TftpSessionPorts,
) -> Result<Option<Rp1DtbPolicy>, BootError> {
    if cfg!(feature = "skip-rp1-reload") {
        crate::logln!(
            "[RP1BOOT] defer RP1 policy load until after kernel TFTP by feature skip-rp1-reload"
        );
        return Ok(None);
    }

    let policy = boot_rp1_from_tftp(dtb, gem, clock, lease, ports)?;
    // SAFETY: the pre-reload GEM is not used after this point. The full reload
    // path leaves this scope and obtains a fresh singleton instance before any
    // further network I/O.
    unsafe { gem.release_after_quiesce() };
    crate::logln!("[TFTP] dropping pre-reload GEM state");
    Ok(Some(policy))
}

fn boot_kernel_from_tftp_with_lease(
    dtb: &dtb::DtbParser,
    gem: &mut Rp1Gem,
    clock: &TimerClock,
    lease: &NetworkBootLease,
    mut rp1_policy: Option<Rp1DtbPolicy>,
    ports: &mut TftpSessionPorts,
) -> Result<(), BootError> {
    crate::logln!(
        "[TFTP] config mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} local={}.{}.{}.{} server={}.{}.{}.{} kernel={} timeout_us={} retries={}",
        gem.mac_addr().0[0],
        gem.mac_addr().0[1],
        gem.mac_addr().0[2],
        gem.mac_addr().0[3],
        gem.mac_addr().0[4],
        gem.mac_addr().0[5],
        lease.client_ip[0],
        lease.client_ip[1],
        lease.client_ip[2],
        lease.client_ip[3],
        lease.tftp_server_ip[0],
        lease.tftp_server_ip[1],
        lease.tftp_server_ip[2],
        lease.tftp_server_ip[3],
        TFTP_KERNEL_FILENAME,
        TFTP_TIMEOUT_US,
        TFTP_MAX_RETRIES
    );
    let skip_rp1_reload = cfg!(feature = "skip-rp1-reload");

    let kernel_cfg = tftp_config(lease, TFTP_KERNEL_FILENAME, ports.alloc());
    crate::logln!(
        "[TFTP] rrq file={} local_port={}",
        TFTP_KERNEL_FILENAME,
        kernel_cfg.local_port
    );
    crate::logln!("[TFTP] kernel download start {}", TFTP_KERNEL_FILENAME);
    let mut kernel_staging = vec![0u8; TFTP_KERNEL_STAGING_MAX];
    let kernel_len = match tftp::download_into(gem, clock, &kernel_cfg, &mut kernel_staging) {
        Ok(len) => len,
        Err(err) => return gem_failure(gem, lease, "kernel download", err),
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
    let initramfs_len = download_initramfs(gem, clock, lease, ports)?;
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

    if skip_rp1_reload {
        rp1_policy = Some(load_rp1_policy_from_tftp(gem, clock, lease, ports)?);
        crate::logln!("[RP1BOOT] skipped by feature skip-rp1-reload");
    }

    let patched_dtb = dtb_patch::patch_dtb_for_linux(
        dtb,
        placement::DTB_COPY_BASE,
        placement::DTB_MAX_SIZE,
        initrd_start,
        initrd_end,
        None,
        rp1_policy.as_ref(),
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

    // SAFETY: this is the terminal Linux handoff path. The GEM reference is
    // not used after release; Linux will probe and own the device next.
    unsafe { gem.release_after_linux_handoff() };
    crate::logln!("[TFTP] Rp1Gem Linux handoff cleanup complete");
    linux::clean_dcache_poc(kernel.base, image.image_size);
    linux::clean_dcache_poc(initrd_start, initramfs_len);
    linux::clean_dcache_poc(patched_dtb.addr, patched_dtb.len);
    linux::invalidate_icache_all();

    // SAFETY: all downloaded artifacts were bounded, kernel header validated,
    // DTB was patched into its reserved range, GEM was released for Linux, and
    // cache maintenance completed before the terminal EL2 handoff.
    unsafe { linux::jump_to_linux_el2(image.entry, patched_dtb.addr) }
}

fn map_dhcp_error(err: DhcpError) -> BootError {
    match err {
        DhcpError::Timeout => BootError::DhcpTimeout,
        DhcpError::InvalidPacket | DhcpError::UnexpectedMessage => BootError::DhcpInvalidPacket,
        DhcpError::NoTftpServer => BootError::DhcpNoTftpServer,
        DhcpError::Encode | DhcpError::Transmit => BootError::Dhcp,
    }
}

fn init_tftp_gem_with_label(
    dtb: &dtb::DtbParser,
    label: &'static str,
) -> Result<&'static mut Rp1Gem, BootError> {
    let rp1 = bcm2712::init_rp1_with_options(
        dtb,
        bcm2712::Rp1InitOptions {
            mode: bcm2712::Rp1InitMode::Auto,
            strict: false,
        },
    )
    .map_err(|err| {
        if label.is_empty() {
            crate::logln!("[TFTP] RP1 PCIe init failed: {:?}", err);
        } else {
            crate::logln!("[TFTP] {} RP1 PCIe init failed: {:?}", label, err);
        }
        BootError::Rp1Pcie
    })?;
    if label.is_empty() {
        crate::logln!("[TFTP] RP1 PCIe init ok");
    } else {
        crate::logln!("[TFTP] {} RP1 PCIe init ok", label);
    }

    let gem = Rp1Gem::init_from_rp1_config(&rp1, TFTP_LOCAL_MAC, Rp1GemOptions::default())
        .map_err(|err| {
            if label.is_empty() {
                crate::logln!("[TFTP] Rp1Gem init failed: {:?}", err);
            } else {
                crate::logln!("[TFTP] {} Rp1Gem init failed: {:?}", label, err);
            }
            BootError::Rp1Gem
        })?;
    if label.is_empty() {
        crate::logln!("[TFTP] Rp1Gem init ok phy={}", gem.phy_address());
    } else {
        crate::logln!("[TFTP] {} Rp1Gem init ok phy={}", label, gem.phy_address());
    }
    match gem.link_state() {
        Ok(link) => {
            if label.is_empty() {
                crate::logln!(
                    "[TFTP] link up={} speed={:?} full_duplex={}",
                    link.up,
                    link.speed,
                    link.full_duplex
                );
            } else {
                crate::logln!(
                    "[TFTP] {} link up={} speed={:?} full_duplex={}",
                    label,
                    link.up,
                    link.speed,
                    link.full_duplex
                );
            }
        }
        Err(err) => {
            if label.is_empty() {
                crate::logln!("[TFTP] link query failed: {:?}", err);
            } else {
                crate::logln!("[TFTP] {} link query failed: {:?}", label, err);
            }
        }
    }
    Ok(gem)
}

fn init_tftp_gem(dtb: &dtb::DtbParser) -> Result<&'static mut Rp1Gem, BootError> {
    init_tftp_gem_with_label(dtb, "")
}

fn boot_rp1_from_tftp(
    dtb: &dtb::DtbParser,
    gem: &mut Rp1Gem,
    clock: &TimerClock,
    lease: &NetworkBootLease,
    ports: &mut TftpSessionPorts,
) -> Result<Rp1DtbPolicy, BootError> {
    let (rp1_elf, policy) = load_rp1_elf_and_policy_from_tftp(gem, clock, lease, ports)?;

    let scratch = placement::rp1_scratch_slice();
    let image = crate::rp1_image::build_from_rp1_elf(
        &rp1_elf,
        scratch,
        crate::rp1_image::RP1_FALLBACK_STACK,
    )?;
    crate::logln!(
        "[RP1ELF] load_base=0x{:x} image_len={} entry=0x{:x} stack=0x{:x}",
        image.load_addr,
        image.payload.len(),
        image.entry,
        image.stack
    );

    gem.quiesce();
    crate::logln!("[TFTP] Rp1Gem quiesce before RP1 reload complete");
    crate::start_rp1_image(dtb, &image)?;
    Ok(policy)
}

fn load_rp1_policy_from_tftp(
    gem: &mut Rp1Gem,
    clock: &TimerClock,
    lease: &NetworkBootLease,
    ports: &mut TftpSessionPorts,
) -> Result<Rp1DtbPolicy, BootError> {
    let (_rp1_elf, policy) = load_rp1_elf_and_policy_from_tftp(gem, clock, lease, ports)?;
    Ok(policy)
}

fn load_rp1_elf_and_policy_from_tftp(
    gem: &mut Rp1Gem,
    clock: &TimerClock,
    lease: &NetworkBootLease,
    ports: &mut TftpSessionPorts,
) -> Result<(Vec<u8>, Rp1DtbPolicy), BootError> {
    crate::logln!("[TFTP] RP1 ELF download start {}", TFTP_RP1_ELF_FILENAME);
    let rp1_elf = download_tftp_required(
        gem,
        clock,
        lease,
        TFTP_RP1_ELF_FILENAME,
        TFTP_RP1_ELF_STAGING_MAX,
        ports,
    )?;
    crate::logln!("[TFTP] RP1 ELF download complete len={}", rp1_elf.len());

    let config = download_tftp_optional(
        gem,
        clock,
        lease,
        TFTP_RP1_CONFIG_FILENAME,
        TFTP_RP1_CONFIG_STAGING_MAX,
        ports,
    )?;
    let policy = crate::enforce_rp1_elf_note_policy_with_config(&rp1_elf, config.as_deref())?;
    flush_tftp_transfer(gem, clock, lease, ports)?;
    drain_rx(gem, clock, 200_000);
    Ok((rp1_elf, policy))
}

#[cfg(feature = "tftp-initramfs")]
fn download_initramfs(
    gem: &mut Rp1Gem,
    clock: &TimerClock,
    lease: &NetworkBootLease,
    ports: &mut TftpSessionPorts,
) -> Result<usize, BootError> {
    let config = tftp_config(lease, TFTP_INITRAMFS_FILENAME, ports.alloc());
    crate::logln!(
        "[TFTP] rrq file={} local_port={}",
        TFTP_INITRAMFS_FILENAME,
        config.local_port
    );
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
        Err(err) => return gem_failure(gem, lease, "initramfs download", err),
    };
    crate::logln!(
        "[TFTP] initramfs download complete addr=0x{:x} len={}",
        placement::INITRAMFS_LOAD_BASE,
        len
    );
    Ok(len)
}

fn tftp_config<'a>(
    lease: &NetworkBootLease,
    filename: &'a str,
    local_port: u16,
) -> tftp::TftpConfig<'a> {
    tftp::TftpConfig {
        local_ip: lease.client_ip,
        server_ip: lease.tftp_server_ip,
        server_port: tftp::TFTP_PORT,
        local_port,
        filename,
        timeout_us: TFTP_TIMEOUT_US,
        max_retries: TFTP_MAX_RETRIES,
    }
}

fn download_tftp_required(
    gem: &mut Rp1Gem,
    clock: &TimerClock,
    lease: &NetworkBootLease,
    filename: &'static str,
    max_len: usize,
    ports: &mut TftpSessionPorts,
) -> Result<Vec<u8>, BootError> {
    let config = tftp_config(lease, filename, ports.alloc());
    crate::logln!(
        "[TFTP] rrq file={} local_port={}",
        filename,
        config.local_port
    );
    let mut staging = vec![0u8; max_len];
    let len = match tftp::download_into(gem, clock, &config, &mut staging) {
        Ok(len) => len,
        Err(err) => return gem_failure(gem, lease, filename, err),
    };
    staging.truncate(len);
    Ok(staging)
}

fn download_tftp_optional(
    gem: &mut Rp1Gem,
    clock: &TimerClock,
    lease: &NetworkBootLease,
    filename: &'static str,
    max_len: usize,
    ports: &mut TftpSessionPorts,
) -> Result<Option<Vec<u8>>, BootError> {
    let config = tftp_config(lease, filename, ports.alloc());
    crate::logln!(
        "[TFTP] rrq file={} local_port={}",
        filename,
        config.local_port
    );
    let mut staging = vec![0u8; max_len];
    let len = match tftp::download_into(gem, clock, &config, &mut staging) {
        Ok(len) => len,
        Err(tftp::TftpError::ServerError { code: 1 }) => {
            crate::logln!("[TFTP] optional {} not found", filename);
            return Ok(None);
        }
        Err(err) => return gem_failure(gem, lease, filename, err),
    };
    staging.truncate(len);
    crate::logln!("[TFTP] optional {} found len={}", filename, len);
    Ok(Some(staging))
}

fn flush_tftp_transfer(
    gem: &mut Rp1Gem,
    clock: &TimerClock,
    lease: &NetworkBootLease,
    ports: &mut TftpSessionPorts,
) -> Result<(), BootError> {
    match download_tftp_optional(
        gem,
        clock,
        lease,
        TFTP_FLUSH_FILENAME,
        TFTP_RP1_CONFIG_STAGING_MAX,
        ports,
    ) {
        Ok(Some(bytes)) => {
            crate::logln!("[TFTP] flush consumed stale transfer len={}", bytes.len());
            Ok(())
        }
        Ok(None) => {
            crate::logln!("[TFTP] flush consumed server not-found response");
            Ok(())
        }
        Err(err) => Err(err),
    }
}

fn drain_rx(gem: &mut Rp1Gem, clock: &TimerClock, duration_us: u64) {
    let start = tftp::TftpClock::now_us(clock);
    let mut frame = [0u8; 1536];
    let mut drained = 0usize;
    while tftp::TftpClock::now_us(clock).wrapping_sub(start) < duration_us {
        if gem.try_recv_frame(&mut frame).is_some() {
            drained = drained.saturating_add(1);
        }
    }
    if drained != 0 {
        crate::logln!("[TFTP] drained {} stale RX frames", drained);
    }
}

fn gem_failure<T>(
    gem: &mut Rp1Gem,
    lease: &NetworkBootLease,
    stage: &'static str,
    err: tftp::TftpError,
) -> Result<T, BootError> {
    crate::logln!("[TFTP] {} failed: {:?}", stage, err);
    crate::logln!(
        "[TFTP] lease client={}.{}.{}.{} server={}.{}.{}.{} source={}",
        lease.client_ip[0],
        lease.client_ip[1],
        lease.client_ip[2],
        lease.client_ip[3],
        lease.tftp_server_ip[0],
        lease.tftp_server_ip[1],
        lease.tftp_server_ip[2],
        lease.tftp_server_ip[3],
        lease.tftp_server_source.as_str()
    );
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
