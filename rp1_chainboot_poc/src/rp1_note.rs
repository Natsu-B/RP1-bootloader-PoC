use rp1_abi::note::{
    Rp1BootInfoV1, RP1_NOTE_ABI_VERSION, RP1_NOTE_MAGIC, RP1_NOTE_NAME, RP1_NOTE_TYPE_BOOT_V1,
    RP1_VERSION_NON_PIO,
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
    pub memory_profile: Rp1MemoryProfile,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Rp1MemoryProfile {
    Legacy,
    PrivateLayoutV1,
    SharedSramV2,
}

impl Rp1MemoryProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::PrivateLayoutV1 => "private-layout-v1",
            Self::SharedSramV2 => "shared-sram-v2",
        }
    }
}

pub const RP1_MAILBOX_FLAG_ENABLE: u32 = 1 << 0;
pub const RP1_MAILBOX_FLAG_PRIVATE_LAYOUT_V1: u32 = 1 << 1;
pub const RP1_MAILBOX_FLAG_SHARED_SRAM_V2: u32 = 1 << 2;
pub const RP1_MAILBOX_FLAGS_SUPPORTED_MASK: u32 =
    RP1_MAILBOX_FLAG_ENABLE | RP1_MAILBOX_FLAG_PRIVATE_LAYOUT_V1 | RP1_MAILBOX_FLAG_SHARED_SRAM_V2;
pub const RP1_SHARED_SRAM_V2_LOAD_BASE: u32 = 0x2000_0000;
pub const RP1_PRIVATE_LAYOUT_V1_MAX_IMAGE_LEN: usize = 0xf800;
pub const RP1_PRIVATE_LAYOUT_V1_STACK_TOP: u32 =
    RP1_SHARED_SRAM_V2_LOAD_BASE + RP1_PRIVATE_LAYOUT_V1_MAX_IMAGE_LEN as u32;
pub const RP1_SHARED_SRAM_V2_MAX_IMAGE_LEN: usize = 0xf700;
pub const RP1_SHARED_SRAM_V2_STACK_TOP: u32 =
    RP1_SHARED_SRAM_V2_LOAD_BASE + RP1_SHARED_SRAM_V2_MAX_IMAGE_LEN as u32;

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
    if desc.len() != Rp1BootInfoV1::SIZE {
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
    let Some(header_flags) = le32_opt(desc, 12) else {
        return Rp1NoteState::Invalid;
    };
    if header_flags != 0 {
        return Rp1NoteState::Invalid;
    }
    if le32_opt(desc, 16).unwrap_or(1) != 0 {
        return Rp1NoteState::Invalid;
    }
    if desc
        .get(144..176)
        .is_none_or(|reserved| reserved.iter().any(|&b| b != 0))
    {
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
    if mailbox_flags & !RP1_MAILBOX_FLAGS_SUPPORTED_MASK != 0
        || mailbox_flags & (RP1_MAILBOX_FLAG_PRIVATE_LAYOUT_V1 | RP1_MAILBOX_FLAG_SHARED_SRAM_V2)
            == (RP1_MAILBOX_FLAG_PRIVATE_LAYOUT_V1 | RP1_MAILBOX_FLAG_SHARED_SRAM_V2)
    {
        return Rp1NoteState::Invalid;
    }
    let Some(entry) = le32_opt(desc, 20) else {
        return Rp1NoteState::Invalid;
    };
    let Some(stack_top) = le32_opt(desc, 24) else {
        return Rp1NoteState::Invalid;
    };
    let Some(load_base) = le32_opt(desc, 32) else {
        return Rp1NoteState::Invalid;
    };
    let Some(image_min_addr) = le32_opt(desc, 36) else {
        return Rp1NoteState::Invalid;
    };
    let Some(image_max_addr) = le32_opt(desc, 40) else {
        return Rp1NoteState::Invalid;
    };
    let memory_profile = match mailbox_flags
        & (RP1_MAILBOX_FLAG_PRIVATE_LAYOUT_V1 | RP1_MAILBOX_FLAG_SHARED_SRAM_V2)
    {
        0 => Rp1MemoryProfile::Legacy,
        RP1_MAILBOX_FLAG_PRIVATE_LAYOUT_V1 => Rp1MemoryProfile::PrivateLayoutV1,
        RP1_MAILBOX_FLAG_SHARED_SRAM_V2 => Rp1MemoryProfile::SharedSramV2,
        _ => return Rp1NoteState::Invalid,
    };
    if !matches!(memory_profile, Rp1MemoryProfile::Legacy)
        && (entry != 0
            || stack_top != 0
            || load_base != 0
            || image_min_addr != 0
            || image_max_addr != 0)
    {
        return Rp1NoteState::Invalid;
    }

    Rp1NoteState::Valid(Rp1BootInfo {
        owner_rp1,
        owner_linux,
        owner_disabled,
        mailbox_flags,
        firmware_version_kind,
        memory_profile,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(mailbox_flags: u32) -> [u8; Rp1BootInfoV1::SIZE] {
        let mut desc = [0; Rp1BootInfoV1::SIZE];
        desc[0..8].copy_from_slice(&RP1_NOTE_MAGIC);
        put_u16(&mut desc, 8, RP1_NOTE_ABI_VERSION);
        put_u16(&mut desc, 10, Rp1BootInfoV1::SIZE as u16);
        put_u32(&mut desc, 72, mailbox_flags);
        put_u32(&mut desc, 76, RP1_VERSION_NON_PIO);
        desc
    }

    #[test]
    fn default_note_is_legacy_compatible() {
        let parsed = parse_boot_info(&desc(0));
        assert!(matches!(
            parsed,
            Rp1NoteState::Valid(Rp1BootInfo {
                memory_profile: Rp1MemoryProfile::Legacy,
                ..
            })
        ));
    }

    #[test]
    fn note_flag_truth_table_classifies_layouts() {
        for flags in [0, RP1_MAILBOX_FLAG_ENABLE] {
            assert!(matches!(
                parse_boot_info(&desc(flags)),
                Rp1NoteState::Valid(Rp1BootInfo {
                    memory_profile: Rp1MemoryProfile::Legacy,
                    ..
                })
            ));
        }

        for flags in [
            RP1_MAILBOX_FLAG_PRIVATE_LAYOUT_V1,
            RP1_MAILBOX_FLAG_ENABLE | RP1_MAILBOX_FLAG_PRIVATE_LAYOUT_V1,
        ] {
            assert!(matches!(
                parse_boot_info(&desc(flags)),
                Rp1NoteState::Valid(Rp1BootInfo {
                    memory_profile: Rp1MemoryProfile::PrivateLayoutV1,
                    ..
                })
            ));
        }

        for flags in [
            RP1_MAILBOX_FLAG_SHARED_SRAM_V2,
            RP1_MAILBOX_FLAG_ENABLE | RP1_MAILBOX_FLAG_SHARED_SRAM_V2,
        ] {
            assert!(matches!(
                parse_boot_info(&desc(flags)),
                Rp1NoteState::Valid(Rp1BootInfo {
                    memory_profile: Rp1MemoryProfile::SharedSramV2,
                    ..
                })
            ));
        }

        for flags in [
            RP1_MAILBOX_FLAG_PRIVATE_LAYOUT_V1 | RP1_MAILBOX_FLAG_SHARED_SRAM_V2,
            RP1_MAILBOX_FLAG_ENABLE
                | RP1_MAILBOX_FLAG_PRIVATE_LAYOUT_V1
                | RP1_MAILBOX_FLAG_SHARED_SRAM_V2,
            8,
        ] {
            assert!(matches!(
                parse_boot_info(&desc(flags)),
                Rp1NoteState::Invalid
            ));
        }
    }

    #[test]
    fn hal_note_fixture_uses_mailbox_flags_at_desc_72() {
        for (flags, profile) in [
            (RP1_MAILBOX_FLAG_ENABLE, Rp1MemoryProfile::Legacy),
            (
                RP1_MAILBOX_FLAG_ENABLE | RP1_MAILBOX_FLAG_PRIVATE_LAYOUT_V1,
                Rp1MemoryProfile::PrivateLayoutV1,
            ),
            (
                RP1_MAILBOX_FLAG_ENABLE | RP1_MAILBOX_FLAG_SHARED_SRAM_V2,
                Rp1MemoryProfile::SharedSramV2,
            ),
        ] {
            let note = note_section(flags);
            let desc = &note[16..];
            assert_eq!(le32_opt(desc, 12), Some(0));
            assert_eq!(le32_opt(desc, 72), Some(flags));
            match parse_note_section(&note) {
                Rp1NoteState::Valid(info) => assert!(info.memory_profile == profile),
                Rp1NoteState::Missing | Rp1NoteState::Invalid => panic!("fixture rejected"),
            }
        }
    }

    #[test]
    fn header_flags_are_rejected_separately_from_mailbox_flags() {
        let mut wrong_header_flags = desc(RP1_MAILBOX_FLAG_SHARED_SRAM_V2);
        put_u32(&mut wrong_header_flags, 12, RP1_MAILBOX_FLAG_SHARED_SRAM_V2);
        assert!(matches!(
            parse_boot_info(&wrong_header_flags),
            Rp1NoteState::Invalid
        ));
    }

    #[test]
    fn v2_rejects_nonzero_legacy_image_contract_fields() {
        let mut wrong_entry = desc(RP1_MAILBOX_FLAG_SHARED_SRAM_V2);
        put_u32(&mut wrong_entry, 20, RP1_SHARED_SRAM_V2_LOAD_BASE | 1);
        assert!(matches!(
            parse_boot_info(&wrong_entry),
            Rp1NoteState::Invalid
        ));
    }

    #[test]
    fn reserved_desc_words_are_rejected() {
        let mut with_reserved = desc(0);
        put_u32(&mut with_reserved, 144, 1);
        assert!(matches!(
            parse_boot_info(&with_reserved),
            Rp1NoteState::Invalid
        ));
    }

    #[test]
    fn oversized_desc_tail_is_rejected() {
        let mut oversized = [0u8; Rp1BootInfoV1::SIZE + 4];
        oversized[..Rp1BootInfoV1::SIZE].copy_from_slice(&desc(0));
        assert!(matches!(parse_boot_info(&oversized), Rp1NoteState::Invalid));
    }

    fn note_section(mailbox_flags: u32) -> [u8; 192] {
        let mut note = [0u8; 192];
        put_u32(&mut note, 0, RP1_NOTE_NAME.len() as u32);
        put_u32(&mut note, 4, Rp1BootInfoV1::SIZE as u32);
        put_u32(&mut note, 8, RP1_NOTE_TYPE_BOOT_V1);
        note[12..16].copy_from_slice(RP1_NOTE_NAME.as_slice());
        note[16..].copy_from_slice(&desc(mailbox_flags));
        note
    }

    fn put_u16(out: &mut [u8], offset: usize, value: u16) {
        out[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(out: &mut [u8], offset: usize, value: u32) {
        out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
}
