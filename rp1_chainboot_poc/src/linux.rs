use crate::BootError;

const ARM64_IMAGE_MAGIC: [u8; 4] = [0x41, 0x52, 0x4d, 0x64];
const SDHCI_PRESENT_STATE: usize = 0x24;
const SDHCI_SOFTWARE_RESET: usize = 0x2f;
const SDHCI_INT_STATUS: usize = 0x30;
const SDHCI_INT_ENABLE: usize = 0x34;
const SDHCI_SIGNAL_ENABLE: usize = 0x38;
const SDHCI_CMD_INHIBIT: u32 = 1 << 0;
const SDHCI_DATA_INHIBIT: u32 = 1 << 1;
const SDHCI_RESET_CMD: u8 = 0x02;
const SDHCI_RESET_DATA: u8 = 0x04;
const SDHCI_INT_ALL_MASK: u32 = u32::MAX;
const SDHC_HOST_FALLBACK_BASE: usize = 0x10_00ff_f000;

pub struct LinuxImage {
    pub entry: usize,
    pub image_size: usize,
}

pub fn validate_arm64_image(base: usize, loaded_len: usize) -> Result<LinuxImage, BootError> {
    if loaded_len < 64 {
        return Err(BootError::LinuxImageInvalid);
    }
    // SAFETY: kernel image has just been copied to this physical range.
    let hdr = unsafe { core::slice::from_raw_parts(base as *const u8, 64) };
    if hdr[56..60] != ARM64_IMAGE_MAGIC {
        return Err(BootError::LinuxImageInvalid);
    }
    let image_size = le64(hdr, 16)? as usize;
    let image_size = if image_size == 0 {
        loaded_len
    } else {
        image_size
    };
    if image_size > loaded_len {
        return Err(BootError::LinuxImageInvalid);
    }
    crate::logln!("[KERNEL] Image header ok, entry=0x{:x}", base);
    Ok(LinuxImage {
        entry: base,
        image_size,
    })
}

pub fn quiesce_sdhc_from_dtb_or_fallback(_dtb: &dtb::DtbParser) -> Result<(), BootError> {
    crate::logln!("[SDHC] quiesce begin");
    let host = SDHC_HOST_FALLBACK_BASE;
    mmio_write32(host + SDHCI_INT_ENABLE, 0);
    mmio_write32(host + SDHCI_SIGNAL_ENABLE, 0);
    crate::logln!("[SDHC] interrupt masks disabled");

    for _ in 0..100_000 {
        let state = mmio_read32(host + SDHCI_PRESENT_STATE);
        if (state & (SDHCI_CMD_INHIBIT | SDHCI_DATA_INHIBIT)) == 0 {
            break;
        }
    }
    mmio_write32(host + SDHCI_INT_STATUS, SDHCI_INT_ALL_MASK);
    mmio_write8(
        host + SDHCI_SOFTWARE_RESET,
        SDHCI_RESET_CMD | SDHCI_RESET_DATA,
    );
    for _ in 0..100_000 {
        if (mmio_read8(host + SDHCI_SOFTWARE_RESET) & (SDHCI_RESET_CMD | SDHCI_RESET_DATA)) == 0 {
            crate::logln!("[SDHC] cmd/data reset ok");
            crate::logln!("[SDHC] quiesce done");
            return Ok(());
        }
    }
    Err(BootError::SdhcQuiesceFailure)
}

pub fn clean_dcache_poc(addr: usize, len: usize) {
    if len == 0 {
        return;
    }
    let line = 64usize;
    let mut p = addr & !(line - 1);
    let end = addr.saturating_add(len).next_multiple_of(line);
    while p < end {
        // SAFETY: `dc cvac` operates on the cache line containing the VA supplied in a register.
        unsafe {
            core::arch::asm!("dc cvac, {line}", line = in(reg) p, options(nostack, preserves_flags));
        }
        p += line;
    }
    // SAFETY: architectural barrier after cache maintenance.
    unsafe {
        core::arch::asm!("dsb sy", "isb", options(nostack, preserves_flags));
    }
}

pub fn invalidate_icache_all() {
    // SAFETY: invalidates local instruction cache before entering Linux.
    unsafe {
        core::arch::asm!(
            "ic iallu",
            "dsb sy",
            "isb",
            options(nostack, preserves_flags)
        );
    }
}

#[unsafe(link_section = ".text.boot.handoff")]
pub unsafe fn jump_to_linux_el2(kernel_entry: usize, dtb_addr: usize) -> ! {
    crate::logln!("[LINUX] preparing EL2 handoff: HCR_EL2=RW, CNTVOFF_EL2=0, CPTR_EL2=0");
    crate::logln!("[LINUX] jumping at EL2");
    // SAFETY: this is the terminal handoff path. It masks interrupts, disables stage-2 and EL2
    // stage-1 MMU/cache bits, sets the arm64 boot protocol registers, and branches to the Image
    // entry without constructing an EL1 wrapper.
    unsafe {
        core::arch::asm!(
            "msr daifset, #0xf",
            "dsb sy",
            "isb",
            "msr vttbr_el2, xzr",
            "msr cntvoff_el2, xzr",
            "msr cptr_el2, xzr",
            "mov x2, #(1 << 31)",
            "msr hcr_el2, x2",
            "isb",
            "mrs x2, sctlr_el2",
            "bic x2, x2, #1",
            "bic x2, x2, #(1 << 2)",
            "bic x2, x2, #(1 << 12)",
            "msr sctlr_el2, x2",
            "isb",
            "mov x16, {entry}",
            "mov x0, {dtb}",
            "mov x1, xzr",
            "mov x2, xzr",
            "mov x3, xzr",
            "br x16",
            entry = in(reg) kernel_entry,
            dtb = in(reg) dtb_addr,
            options(noreturn)
        );
    }
}

fn le64(bytes: &[u8], off: usize) -> Result<u64, BootError> {
    let b = bytes
        .get(off..off + 8)
        .ok_or(BootError::LinuxImageInvalid)?;
    Ok(u64::from_le_bytes([
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
    ]))
}

fn mmio_read32(addr: usize) -> u32 {
    // SAFETY: PoC SDHCI quiesce uses the documented host MMIO fallback.
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

fn mmio_write32(addr: usize, value: u32) {
    // SAFETY: PoC SDHCI quiesce uses the documented host MMIO fallback.
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
}

fn mmio_read8(addr: usize) -> u8 {
    // SAFETY: PoC SDHCI quiesce uses the documented host MMIO fallback.
    unsafe { core::ptr::read_volatile(addr as *const u8) }
}

fn mmio_write8(addr: usize, value: u8) {
    // SAFETY: PoC SDHCI quiesce uses the documented host MMIO fallback.
    unsafe { core::ptr::write_volatile(addr as *mut u8, value) }
}
