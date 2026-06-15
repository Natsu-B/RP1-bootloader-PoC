use crate::BootError;

pub const DTB_PTR: usize = 0x2000_0000;
pub const KERNEL_LOAD_BASE: usize = 0x0300_0000;
pub const KERNEL_MAX_SIZE: usize = 0x0800_0000;
pub const INITRAMFS_LOAD_BASE: usize = 0x1000_0000;
pub const INITRAMFS_MAX_SIZE: usize = 0x0800_0000;
pub const DTB_COPY_BASE: usize = 0x2020_0000;
pub const DTB_MAX_SIZE: usize = 0x0020_0000;
pub const RP1_IMG_SCRATCH_MAX: usize = 0x0002_0000;
pub const RP1_IMG_SCRATCH_BASE: usize = 0x1ffe_0000;

unsafe extern "C" {
    static _PROGRAM_START: u8;
    static _BSS_END: u8;
    static _STACK_BOTTOM: u8;
    static _STACK_TOP: u8;
}

#[derive(Clone, Copy)]
pub struct Range {
    pub name: &'static str,
    pub start: usize,
    pub end: usize,
}

impl Range {
    pub const fn new(name: &'static str, start: usize, len: usize) -> Self {
        Self {
            name,
            start,
            end: start + len,
        }
    }
}

pub fn program_range() -> Range {
    Range {
        name: "program",
        start: (&raw const _PROGRAM_START) as usize,
        end: (&raw const _BSS_END) as usize,
    }
}

pub fn rp1_scratch_slice() -> &'static mut [u8] {
    // SAFETY: RP1_IMG_SCRATCH_BASE is a fixed physical scratch range checked against the
    // bootloader image, stack, kernel, initramfs, and DTB placement before use.
    unsafe { core::slice::from_raw_parts_mut(RP1_IMG_SCRATCH_BASE as *mut u8, RP1_IMG_SCRATCH_MAX) }
}

pub fn stack_range() -> Range {
    Range {
        name: "stack",
        start: (&raw const _STACK_BOTTOM) as usize,
        end: (&raw const _STACK_TOP) as usize,
    }
}

pub fn check_no_overlap(ranges: &[Range]) -> Result<(), BootError> {
    for (idx, a) in ranges.iter().enumerate() {
        if a.start >= a.end {
            crate::logln!("[MEM] invalid range {} {:x}..{:x}", a.name, a.start, a.end);
            return Err(BootError::MemoryOverlap);
        }
        for b in &ranges[idx + 1..] {
            if a.start < b.end && b.start < a.end {
                crate::logln!(
                    "[MEM] overlap {} {:x}..{:x} with {} {:x}..{:x}",
                    a.name,
                    a.start,
                    a.end,
                    b.name,
                    b.start,
                    b.end
                );
                return Err(BootError::MemoryOverlap);
            }
        }
    }
    Ok(())
}

pub fn copy_to_phys(dst: usize, max_len: usize, src: &[u8]) -> Result<(), BootError> {
    if src.len() > max_len {
        return Err(BootError::AddressOverflow);
    }
    // SAFETY: caller selected a physical destination reserved for this boot stage.
    unsafe {
        core::ptr::copy_nonoverlapping(src.as_ptr(), dst as *mut u8, src.len());
    }
    Ok(())
}
