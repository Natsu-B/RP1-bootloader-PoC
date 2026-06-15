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
    Rp1Img,
    FwParts,
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

pub fn build_from_fw_parts<'a>(
    fw1: &'a [u8],
    fw2: &'a [u8],
    scratch: &'a mut [u8],
) -> Result<Rp1Image<'a>, BootError> {
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
