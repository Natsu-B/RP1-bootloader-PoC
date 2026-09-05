use crate::BootError;

pub const RP1_IMG_MAGIC: u32 = u32::from_le_bytes(*b"RP1I");
pub const RP1_SRAM_BASE: u32 = 0x2000_0000;
pub const RP1_FALLBACK_LOAD_ADDR: u32 = 0x2000_0000;
pub const RP1_FALLBACK_ENTRY: u32 = 0x2000_0141;
pub const RP1_FALLBACK_STACK: u32 = 0x1000_30d0;
pub const RP1_MAX_IMAGE_LEN: usize = 0x1_0000;
const RP1_HEADER_LEN_MIN: usize = 0x20;
const RP1_LOCAL_DSRAM_BASE: u32 = 0x1000_2000;
const RP1_LOCAL_DSRAM_END: u32 = 0x1000_4000;
const ELF32_HEADER_LEN: usize = 52;
const ELF32_PHDR_LEN: usize = 32;
const ELFCLASS32: u8 = 1;
const ELFDATA2LSB: u8 = 1;
const EM_ARM: u16 = 40;
const PT_LOAD: u32 = 1;
pub const RP1_ELF_MAX_LOGGED_LOADS: usize = 8;

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

#[derive(Clone, Copy)]
pub struct Rp1ElfLoad {
    pub file_offset: u32,
    pub vaddr: u32,
    pub paddr: u32,
    pub filesz: u32,
    pub memsz: u32,
    pub flags: u32,
    pub align: u32,
}

#[derive(Clone, Copy)]
pub struct Rp1ElfInfo {
    pub entry: u32,
    pub vector0_sp: u32,
    pub vector1_reset: u32,
    pub phnum: u16,
    pub load_count: usize,
    pub loads: [Rp1ElfLoad; RP1_ELF_MAX_LOGGED_LOADS],
}

impl Rp1ElfInfo {
    pub fn loads(&self) -> &[Rp1ElfLoad] {
        &self.loads[..self.load_count]
    }
}

const EMPTY_ELF_LOAD: Rp1ElfLoad = Rp1ElfLoad {
    file_offset: 0,
    vaddr: 0,
    paddr: 0,
    filesz: 0,
    memsz: 0,
    flags: 0,
    align: 0,
};

pub fn inspect_rp1_elf(elf_bytes: &[u8]) -> Result<Rp1ElfInfo, BootError> {
    if elf_bytes.len() < ELF32_HEADER_LEN
        || elf_bytes.get(0..4) != Some(b"\x7fELF")
        || elf_bytes[4] != ELFCLASS32
        || elf_bytes[5] != ELFDATA2LSB
        || le16(elf_bytes, 18)? != EM_ARM
    {
        return Err(BootError::Rp1ImageInvalid);
    }

    let entry = le32(elf_bytes, 24)?;
    let phoff = le32(elf_bytes, 28)? as usize;
    let phentsize = le16(elf_bytes, 42)? as usize;
    let phnum = le16(elf_bytes, 44)?;
    if phentsize < ELF32_PHDR_LEN {
        return Err(BootError::Rp1ImageInvalid);
    }

    let mut loads = [EMPTY_ELF_LOAD; RP1_ELF_MAX_LOGGED_LOADS];
    let mut load_count = 0usize;
    for index in 0..usize::from(phnum) {
        let ph = phoff
            .checked_add(
                index
                    .checked_mul(phentsize)
                    .ok_or(BootError::AddressOverflow)?,
            )
            .ok_or(BootError::AddressOverflow)?;
        let ph_end = ph
            .checked_add(ELF32_PHDR_LEN)
            .ok_or(BootError::AddressOverflow)?;
        if ph_end > elf_bytes.len() {
            return Err(BootError::Rp1ImageInvalid);
        }
        let p_type = le32(elf_bytes, ph)?;
        if p_type != PT_LOAD {
            continue;
        }
        if load_count >= RP1_ELF_MAX_LOGGED_LOADS {
            return Err(BootError::Rp1ImageInvalid);
        }
        let load = Rp1ElfLoad {
            file_offset: le32(elf_bytes, ph + 4)?,
            vaddr: le32(elf_bytes, ph + 8)?,
            paddr: le32(elf_bytes, ph + 12)?,
            filesz: le32(elf_bytes, ph + 16)?,
            memsz: le32(elf_bytes, ph + 20)?,
            flags: le32(elf_bytes, ph + 24)?,
            align: le32(elf_bytes, ph + 28)?,
        };
        let file_end = (load.file_offset as usize)
            .checked_add(load.filesz as usize)
            .ok_or(BootError::AddressOverflow)?;
        if load.filesz > load.memsz || file_end > elf_bytes.len() {
            return Err(BootError::Rp1ImageInvalid);
        }
        loads[load_count] = load;
        load_count += 1;
    }
    if load_count == 0 {
        return Err(BootError::Rp1ImageInvalid);
    }

    let vector0_sp = read_loaded_u32(elf_bytes, &loads[..load_count], RP1_SRAM_BASE)?;
    let vector1_reset = read_loaded_u32(elf_bytes, &loads[..load_count], RP1_SRAM_BASE + 4)?;

    Ok(Rp1ElfInfo {
        entry,
        vector0_sp,
        vector1_reset,
        phnum,
        load_count,
        loads,
    })
}

pub fn elf_load_file_bytes<'a>(
    elf_bytes: &'a [u8],
    load: &Rp1ElfLoad,
) -> Result<&'a [u8], BootError> {
    let start = load.file_offset as usize;
    let end = start
        .checked_add(load.filesz as usize)
        .ok_or(BootError::AddressOverflow)?;
    elf_bytes.get(start..end).ok_or(BootError::Rp1ImageInvalid)
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
    let materialized = elf::materialize_elf32_arm_le(
        elf_bytes,
        scratch,
        elf::MaterializeOptions {
            load_base: u64::from(RP1_SRAM_BASE),
            max_image_size: scratch.len(),
            require_entry_in_range: true,
        },
    )
    .map_err(|err| {
        crate::logln!("[RP1ELF] materialize failed: {:?}", err);
        BootError::Rp1ImageInvalid
    })?;
    if materialized.image_len == 0 || materialized.image_len > RP1_MAX_IMAGE_LEN {
        return Err(BootError::Rp1ImageTooLarge);
    }
    let entry = u32::try_from(materialized.entry).map_err(|_| BootError::Rp1ImageInvalid)?;
    let vector_stack = le32(&scratch[..materialized.image_len], 0)
        .ok()
        .filter(|stack| is_valid_stack(*stack));
    let stack = vector_stack
        .or_else(|| {
            materialized
                .stack
                .and_then(|stack| u32::try_from(stack).ok())
        })
        .unwrap_or(fallback_stack);
    if entry == 0 || stack == 0 {
        return Err(BootError::Rp1ImageInvalid);
    }
    Ok(Rp1Image {
        payload: &scratch[..materialized.image_len],
        load_addr: RP1_SRAM_BASE,
        entry: entry | 1,
        stack,
        source: Rp1ImageSource::Rp1Elf,
    })
}

pub fn is_valid_stack(stack: u32) -> bool {
    ((stack >= RP1_SRAM_BASE) && (stack <= RP1_SRAM_BASE + RP1_MAX_IMAGE_LEN as u32))
        || (cfg!(feature = "rp1-allow-local-dsram-vector-stack")
            && (stack >= RP1_LOCAL_DSRAM_BASE)
            && (stack <= RP1_LOCAL_DSRAM_END))
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

fn le16(bytes: &[u8], off: usize) -> Result<u16, BootError> {
    let end = off.checked_add(2).ok_or(BootError::AddressOverflow)?;
    let src = bytes.get(off..end).ok_or(BootError::Rp1ImageInvalid)?;
    Ok(u16::from_le_bytes([src[0], src[1]]))
}

fn read_loaded_u32(elf_bytes: &[u8], loads: &[Rp1ElfLoad], addr: u32) -> Result<u32, BootError> {
    let bytes = read_loaded_bytes(elf_bytes, loads, addr, 4)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_loaded_bytes<'a>(
    elf_bytes: &'a [u8],
    loads: &[Rp1ElfLoad],
    addr: u32,
    len: u32,
) -> Result<&'a [u8], BootError> {
    let end = addr.checked_add(len).ok_or(BootError::AddressOverflow)?;
    for load in loads {
        let load_file_end = load
            .paddr
            .checked_add(load.filesz)
            .ok_or(BootError::AddressOverflow)?;
        if addr >= load.paddr && end <= load_file_end {
            let delta = addr - load.paddr;
            let file_off = load
                .file_offset
                .checked_add(delta)
                .ok_or(BootError::AddressOverflow)? as usize;
            let file_end = file_off
                .checked_add(len as usize)
                .ok_or(BootError::AddressOverflow)?;
            return elf_bytes
                .get(file_off..file_end)
                .ok_or(BootError::Rp1ImageInvalid);
        }
    }
    Err(BootError::Rp1ImageInvalid)
}
