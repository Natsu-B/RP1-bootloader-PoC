use rp1_abi::note::{
    RP1_NOTE_ABI_VERSION, RP1_NOTE_MAGIC, RP1_NOTE_NAME, RP1_NOTE_TYPE_BOOT_V1,
    RP1_VERSION_NON_PIO, Rp1BootInfoV1,
};

pub enum Rp1NoteState {
    Valid(Rp1BootInfo),
    Missing,
    Invalid,
}

pub struct Rp1BootInfo {
    pub owner_rp1: u64,
    pub owner_linux: u64,
    pub owner_disabled: u64,
    pub mailbox_flags: u32,
    pub firmware_version_kind: u32,
}

pub fn parse_rp1_note(elf_bytes: &[u8]) -> Rp1NoteState {
    let Some(note_section) = find_note_section(elf_bytes) else {
        return Rp1NoteState::Missing;
    };
    parse_note_section(note_section)
}

// This PoC currently finds .note.rp1 through the ELF section header table.
// Future loaders should prefer PT_NOTE because sections are not required at
// runtime and may be stripped from otherwise loadable ELF images.
fn find_note_section(elf: &[u8]) -> Option<&[u8]> {
    if elf.get(0..4) != Some(b"\x7fELF" as &[u8]) {
        return None;
    }
    if elf.get(4) != Some(&1) || elf.get(5) != Some(&1) {
        return None;
    }

    let shoff = le32_opt(elf, 32)? as usize;
    let shentsize = le16_opt(elf, 46)? as usize;
    let shnum = le16_opt(elf, 48)? as usize;
    let shstrndx = le16_opt(elf, 50)? as usize;
    if shoff == 0 || shentsize < 40 || shnum == 0 || shstrndx >= shnum {
        return None;
    }

    let shstr = section_header(elf, shoff, shentsize, shstrndx)?;
    let shstrtab = section_data(elf, shstr)?;
    for idx in 0..shnum {
        let sh = section_header(elf, shoff, shentsize, idx)?;
        let name_off = le32_opt(sh, 0)? as usize;
        if section_name(shstrtab, name_off) == Some(b".note.rp1" as &[u8]) {
            return section_data(elf, sh);
        }
    }

    None
}

fn parse_note_section(section: &[u8]) -> Rp1NoteState {
    let mut off = 0usize;
    while off < section.len() {
        let Some(namesz) = le32_opt(section, off) else {
            return Rp1NoteState::Invalid;
        };
        let Some(descsz) = le32_opt(section, off + 4) else {
            return Rp1NoteState::Invalid;
        };
        let Some(note_type) = le32_opt(section, off + 8) else {
            return Rp1NoteState::Invalid;
        };

        let name_start = match off.checked_add(12) {
            Some(value) => value,
            None => return Rp1NoteState::Invalid,
        };
        let name_end = match name_start.checked_add(namesz as usize) {
            Some(value) => value,
            None => return Rp1NoteState::Invalid,
        };
        let desc_start = match align4(name_end) {
            Some(value) => value,
            None => return Rp1NoteState::Invalid,
        };
        let desc_end = match desc_start.checked_add(descsz as usize) {
            Some(value) => value,
            None => return Rp1NoteState::Invalid,
        };
        let next = match align4(desc_end) {
            Some(value) => value,
            None => return Rp1NoteState::Invalid,
        };
        let Some(name) = section.get(name_start..name_end) else {
            return Rp1NoteState::Invalid;
        };
        let Some(desc) = section.get(desc_start..desc_end) else {
            return Rp1NoteState::Invalid;
        };

        if name == RP1_NOTE_NAME.as_slice() && note_type == RP1_NOTE_TYPE_BOOT_V1 {
            return parse_boot_info(desc);
        }

        if next <= off {
            return Rp1NoteState::Invalid;
        }
        off = next;
    }

    Rp1NoteState::Invalid
}

fn parse_boot_info(desc: &[u8]) -> Rp1NoteState {
    if desc.len() < Rp1BootInfoV1::SIZE {
        return Rp1NoteState::Invalid;
    }
    if desc.get(0..8) != Some(RP1_NOTE_MAGIC.as_slice()) {
        return Rp1NoteState::Invalid;
    }
    if le16_opt(desc, 8) != Some(RP1_NOTE_ABI_VERSION) {
        return Rp1NoteState::Invalid;
    }
    if le16_opt(desc, 10) != Some(Rp1BootInfoV1::SIZE as u16) {
        return Rp1NoteState::Invalid;
    }
    if le32_opt(desc, 16).unwrap_or(1) != 0 {
        return Rp1NoteState::Invalid;
    }

    let Some(firmware_version_kind) = le32_opt(desc, 76) else {
        return Rp1NoteState::Invalid;
    };
    if firmware_version_kind != RP1_VERSION_NON_PIO {
        return Rp1NoteState::Invalid;
    }

    let Some(owner_rp1) = le64_opt(desc, 48) else {
        return Rp1NoteState::Invalid;
    };
    let Some(owner_linux) = le64_opt(desc, 56) else {
        return Rp1NoteState::Invalid;
    };
    let Some(owner_disabled) = le64_opt(desc, 64) else {
        return Rp1NoteState::Invalid;
    };
    let Some(mailbox_flags) = le32_opt(desc, 72) else {
        return Rp1NoteState::Invalid;
    };

    Rp1NoteState::Valid(Rp1BootInfo {
        owner_rp1,
        owner_linux,
        owner_disabled,
        mailbox_flags,
        firmware_version_kind,
    })
}

fn section_header(elf: &[u8], shoff: usize, shentsize: usize, idx: usize) -> Option<&[u8]> {
    let off = shoff.checked_add(idx.checked_mul(shentsize)?)?;
    let end = off.checked_add(40)?;
    elf.get(off..end)
}

fn section_data<'a>(elf: &'a [u8], sh: &[u8]) -> Option<&'a [u8]> {
    let off = le32_opt(sh, 16)? as usize;
    let size = le32_opt(sh, 20)? as usize;
    let end = off.checked_add(size)?;
    elf.get(off..end)
}

fn section_name(names: &[u8], off: usize) -> Option<&[u8]> {
    let rest = names.get(off..)?;
    let end = rest.iter().position(|&byte| byte == 0)?;
    Some(&rest[..end])
}

fn align4(value: usize) -> Option<usize> {
    value.checked_add(3).map(|value| value & !3)
}

fn le16_opt(bytes: &[u8], off: usize) -> Option<u16> {
    let src = bytes.get(off..off.checked_add(2)?)?;
    Some(u16::from_le_bytes([src[0], src[1]]))
}

fn le32_opt(bytes: &[u8], off: usize) -> Option<u32> {
    let src = bytes.get(off..off.checked_add(4)?)?;
    Some(u32::from_le_bytes([src[0], src[1], src[2], src[3]]))
}

fn le64_opt(bytes: &[u8], off: usize) -> Option<u64> {
    let src = bytes.get(off..off.checked_add(8)?)?;
    Some(u64::from_le_bytes([
        src[0], src[1], src[2], src[3], src[4], src[5], src[6], src[7],
    ]))
}
