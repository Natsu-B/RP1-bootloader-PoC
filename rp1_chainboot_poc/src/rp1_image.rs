use alloc::vec::Vec;

use crate::rp1_note::{
    Rp1MemoryProfile, RP1_PRIVATE_LAYOUT_V1_MAX_IMAGE_LEN, RP1_SHARED_SRAM_V2_MAX_IMAGE_LEN,
};
use crate::BootError;

pub const RP1_IMG_MAGIC: u32 = u32::from_le_bytes(*b"RP1I");
pub const RP1_SRAM_BASE: u32 = 0x2000_0000;
pub const RP1_FALLBACK_LOAD_ADDR: u32 = 0x2000_0000;
pub const RP1_FALLBACK_ENTRY: u32 = 0x2000_0141;
pub const RP1_FALLBACK_STACK: u32 = 0x1000_30d0;
pub const RP1_MAX_IMAGE_LEN: usize = 0x1_0000;
const RP1_HEADER_LEN_MIN: usize = 0x20;

pub struct Rp1Image<'a> {
    pub payload: &'a [u8],
    pub load_addr: u32,
    pub entry: u32,
    pub stack: u32,
    pub source: Rp1ImageSource,
}

#[derive(Clone, Copy)]
pub enum Rp1ImageSource {
    Rp1Elf,
    Rp1Img,
    FwParts,
}

/// Builds an RP1 bootstrap image from generic ELF32 ARM load segments.
///
/// Stack and Thumb-state policy remain in this RP1-specific layer; the generic
/// ELF materializer only copies loadable memory ranges.
pub fn build_from_rp1_elf<'a>(
    elf_bytes: &[u8],
    scratch: &'a mut [u8],
    fallback_stack: u32,
) -> Result<Rp1Image<'a>, BootError> {
    build_from_rp1_elf_with_profile(elf_bytes, scratch, fallback_stack, Rp1MemoryProfile::Legacy)
}

pub fn build_from_rp1_elf_with_profile<'a>(
    elf_bytes: &[u8],
    scratch: &'a mut [u8],
    fallback_stack: u32,
    profile: Rp1MemoryProfile,
) -> Result<Rp1Image<'a>, BootError> {
    let max_image_size = max_image_size_for_profile(profile);
    if stack_for_profile(profile, fallback_stack) == 0 {
        return Err(BootError::Rp1ImageInvalid);
    }
    let preflight = preflight_elf32_arm_le(elf_bytes, scratch.len(), max_image_size)
        .map_err(|_| BootError::Rp1ImageInvalid)?;
    let materialized = elf::materialize_elf32_arm_le(
        elf_bytes,
        scratch,
        elf::MaterializeOptions {
            load_base: u64::from(RP1_SRAM_BASE),
            max_image_size: core::cmp::min(scratch.len(), max_image_size),
            require_entry_in_range: true,
        },
    )
    .map_err(|err| {
        crate::logln!("[RP1ELF] materialize failed: {:?}", err);
        BootError::Rp1ImageInvalid
    })?;
    if materialized.image_len == 0 || materialized.image_len > max_image_size {
        return Err(BootError::Rp1ImageTooLarge);
    }
    if materialized.image_len != preflight.image_len || materialized.entry != preflight.entry {
        return Err(BootError::Rp1ImageInvalid);
    }
    let entry = u32::try_from(materialized.entry).map_err(|_| BootError::Rp1ImageInvalid)?;
    if entry == 0 {
        return Err(BootError::Rp1ImageInvalid);
    }
    Ok(Rp1Image {
        payload: &scratch[..materialized.image_len],
        load_addr: RP1_SRAM_BASE,
        entry: entry | 1,
        stack: stack_for_profile(profile, fallback_stack),
        source: Rp1ImageSource::Rp1Elf,
    })
}

fn max_image_size_for_profile(profile: Rp1MemoryProfile) -> usize {
    match profile {
        Rp1MemoryProfile::Legacy => RP1_MAX_IMAGE_LEN,
        Rp1MemoryProfile::PrivateLayoutV1 => RP1_PRIVATE_LAYOUT_V1_MAX_IMAGE_LEN,
        Rp1MemoryProfile::SharedSramV2 => RP1_SHARED_SRAM_V2_MAX_IMAGE_LEN,
    }
}

fn stack_for_profile(profile: Rp1MemoryProfile, fallback_stack: u32) -> u32 {
    match profile {
        Rp1MemoryProfile::Legacy => fallback_stack,
        Rp1MemoryProfile::PrivateLayoutV1 => crate::rp1_note::RP1_PRIVATE_LAYOUT_V1_STACK_TOP,
        Rp1MemoryProfile::SharedSramV2 => crate::rp1_note::RP1_SHARED_SRAM_V2_STACK_TOP,
    }
}

struct Rp1ElfPreflight {
    image_len: usize,
    entry: u64,
}

fn preflight_elf32_arm_le(
    elf_bytes: &[u8],
    output_capacity: usize,
    max_image_size: usize,
) -> Result<Rp1ElfPreflight, elf::ElfErr> {
    let elf = elf::Elf32::parse_arm_le(elf_bytes)?;
    let load_base = u64::from(RP1_SRAM_BASE);
    let max_end = load_base
        .checked_add(max_image_size as u64)
        .ok_or(elf::ElfErr::Invalid)?;
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    let mut image_end = load_base;

    elf.for_each_load_segment(|segment| {
        let end = segment
            .paddr
            .checked_add(segment.mem_size)
            .ok_or(elf::ElfErr::Invalid)?;
        if segment.paddr < load_base || end > max_end {
            return Err(elf::ElfErr::Invalid);
        }
        let dst_end = usize::try_from(end - load_base).map_err(|_| elf::ElfErr::Invalid)?;
        if dst_end > output_capacity {
            return Err(elf::ElfErr::TooShort);
        }
        for (start, prior_end) in ranges.iter().copied() {
            if segment.paddr < prior_end && end > start {
                return Err(elf::ElfErr::Invalid);
            }
        }
        ranges.push((segment.paddr, end));
        image_end = image_end.max(end);
        Ok(())
    })?;

    if ranges.is_empty() || image_end == load_base {
        return Err(elf::ElfErr::Invalid);
    }
    let entry = u64::from(elf.entry()?);
    if entry < load_base || entry >= image_end {
        return Err(elf::ElfErr::Invalid);
    }

    Ok(Rp1ElfPreflight {
        image_len: usize::try_from(image_end - load_base).map_err(|_| elf::ElfErr::Invalid)?,
        entry,
    })
}

pub fn parse_rp1_img(bytes: &[u8]) -> Result<Rp1Image<'_>, BootError> {
    if bytes.len() < RP1_HEADER_LEN_MIN {
        return Err(BootError::Rp1ImageInvalid);
    }
    let magic = le32(bytes, 0)?;
    let header_len = le32(bytes, 4)? as usize;
    let image_len = le32(bytes, 8)? as usize;
    let load_addr = le32(bytes, 12)?;
    let entry = le32(bytes, 16)?;
    let stack = le32(bytes, 20)?;
    let crc32 = le32(bytes, 24)?;

    if magic != RP1_IMG_MAGIC
        || header_len < RP1_HEADER_LEN_MIN
        || header_len > bytes.len()
        || image_len == 0
        || image_len > RP1_MAX_IMAGE_LEN
        || header_len
            .checked_add(image_len)
            .is_none_or(|end| end > bytes.len())
        || load_addr != RP1_SRAM_BASE
        || stack == 0
    {
        return Err(BootError::Rp1ImageInvalid);
    }

    let entry_addr = entry & !1;
    if entry_addr < load_addr || entry_addr >= load_addr.saturating_add(image_len as u32) {
        return Err(BootError::Rp1ImageInvalid);
    }

    let payload = &bytes[header_len..header_len + image_len];
    if crc32 != 0 && crc32_ieee(payload) != crc32 {
        return Err(BootError::Rp1ImageCrcMismatch);
    }

    Ok(Rp1Image {
        payload,
        load_addr,
        entry: entry | 1,
        stack,
        source: Rp1ImageSource::Rp1Img,
    })
}

pub fn build_from_fw_parts<'scratch>(
    fw1: &[u8],
    fw2: &[u8],
    scratch: &'scratch mut [u8],
) -> Result<Rp1Image<'scratch>, BootError> {
    let total = fw1
        .len()
        .checked_add(fw2.len())
        .ok_or(BootError::AddressOverflow)?;
    if total == 0 || total > RP1_MAX_IMAGE_LEN || total > scratch.len() {
        return Err(BootError::Rp1ImageTooLarge);
    }
    scratch[..fw1.len()].copy_from_slice(fw1);
    scratch[fw1.len()..total].copy_from_slice(fw2);
    Ok(Rp1Image {
        payload: &scratch[..total],
        load_addr: RP1_FALLBACK_LOAD_ADDR,
        entry: RP1_FALLBACK_ENTRY | 1,
        stack: RP1_FALLBACK_STACK,
        source: Rp1ImageSource::FwParts,
    })
}

pub fn checksum32(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0u32, |acc, &b| acc.wrapping_add(u32::from(b)))
}

pub fn crc32_ieee(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn le32(bytes: &[u8], off: usize) -> Result<u32, BootError> {
    let end = off.checked_add(4).ok_or(BootError::AddressOverflow)?;
    let src = bytes.get(off..end).ok_or(BootError::Rp1ImageInvalid)?;
    Ok(u32::from_le_bytes([src[0], src[1], src[2], src[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rp1_note::RP1_SHARED_SRAM_V2_STACK_TOP;

    fn v2_profile() -> Rp1MemoryProfile {
        Rp1MemoryProfile::SharedSramV2
    }

    fn v1_profile() -> Rp1MemoryProfile {
        Rp1MemoryProfile::PrivateLayoutV1
    }

    #[test]
    fn private_v1_image_len_boundary_is_f800() {
        assert_eq!(max_image_size_for_profile(v1_profile()), 0xf800);
        assert!(0xf800 <= max_image_size_for_profile(v1_profile()));
        assert!(0xf801 > max_image_size_for_profile(v1_profile()));
    }

    #[test]
    fn v2_image_len_boundary_is_f700() {
        assert_eq!(max_image_size_for_profile(v2_profile()), 0xf700);
        assert!(0xf700 <= max_image_size_for_profile(v2_profile()));
        assert!(0xf701 > max_image_size_for_profile(v2_profile()));
    }

    #[test]
    fn v2_stack_and_entry_come_from_profile() {
        let profile = v2_profile();
        assert_eq!(
            stack_for_profile(profile, RP1_FALLBACK_STACK),
            RP1_SHARED_SRAM_V2_STACK_TOP
        );
    }

    #[test]
    fn legacy_profile_keeps_default_compatibility() {
        assert_eq!(
            max_image_size_for_profile(Rp1MemoryProfile::Legacy),
            RP1_MAX_IMAGE_LEN
        );
        assert_eq!(
            stack_for_profile(Rp1MemoryProfile::Legacy, RP1_FALLBACK_STACK),
            RP1_FALLBACK_STACK
        );
    }

    #[test]
    fn invalid_elf_preflight_preserves_scratch_sentinel() {
        let cases = [
            elf32(&[load(0x100, RP1_SRAM_BASE - 4, 4, 4)], RP1_SRAM_BASE + 1),
            elf32(
                &[
                    load(0x100, RP1_SRAM_BASE, 4, 4),
                    load(0x104, RP1_SRAM_BASE + 2, 4, 4),
                ],
                RP1_SRAM_BASE + 1,
            ),
            elf32(&[load(0x100, RP1_SRAM_BASE, 4, 4)], RP1_SRAM_BASE + 8),
            elf32(&[load(0x100, RP1_SRAM_BASE, 32, 32)], RP1_SRAM_BASE + 1),
            elf32(
                &[load(
                    0x100,
                    RP1_SRAM_BASE,
                    0,
                    (RP1_PRIVATE_LAYOUT_V1_MAX_IMAGE_LEN + 1) as u32,
                )],
                RP1_SRAM_BASE + 1,
            ),
            elf32(
                &[load(
                    0x100,
                    RP1_SRAM_BASE,
                    0,
                    (RP1_SHARED_SRAM_V2_MAX_IMAGE_LEN + 1) as u32,
                )],
                RP1_SRAM_BASE + 1,
            ),
        ];
        for (idx, data) in cases.into_iter().enumerate() {
            let profile = if idx == 4 { v1_profile() } else { v2_profile() };
            let mut scratch = [0xa5; 16];
            assert!(build_from_rp1_elf_with_profile(
                &data,
                &mut scratch,
                RP1_FALLBACK_STACK,
                profile,
            )
            .is_err());
            assert_eq!(scratch, [0xa5; 16]);
        }
    }

    const ET_EXEC: u16 = 2;
    const EM_ARM: u16 = 40;
    const PT_LOAD: u32 = 1;
    const ELF32_HEADER_LEN: usize = 52;
    const ELF32_PROGRAM_HEADER_LEN: usize = 32;

    fn elf32(headers: &[[u32; 8]], entry: u32) -> [u8; 512] {
        let mut data = [0u8; 512];
        data[0..4].copy_from_slice(b"\x7fELF");
        data[4] = 1;
        data[5] = 1;
        data[6] = 1;
        put_u16(&mut data, 16, ET_EXEC);
        put_u16(&mut data, 18, EM_ARM);
        put_u32(&mut data, 20, 1);
        put_u32(&mut data, 24, entry);
        put_u32(&mut data, 28, ELF32_HEADER_LEN as u32);
        put_u16(&mut data, 40, ELF32_HEADER_LEN as u16);
        put_u16(&mut data, 42, ELF32_PROGRAM_HEADER_LEN as u16);
        put_u16(&mut data, 44, headers.len() as u16);
        for (index, header) in headers.iter().enumerate() {
            let offset = ELF32_HEADER_LEN + index * ELF32_PROGRAM_HEADER_LEN;
            for (word, value) in header.iter().enumerate() {
                put_u32(&mut data, offset + word * 4, *value);
            }
        }
        data
    }

    fn load(offset: u32, paddr: u32, file_size: u32, mem_size: u32) -> [u32; 8] {
        [PT_LOAD, offset, paddr, paddr, file_size, mem_size, 5, 4]
    }

    fn put_u16(out: &mut [u8], offset: usize, value: u16) {
        out[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(out: &mut [u8], offset: usize, value: u32) {
        out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
}
