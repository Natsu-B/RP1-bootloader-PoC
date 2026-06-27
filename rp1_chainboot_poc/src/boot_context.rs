use dtb::{DeviceTree, DeviceTreeQueryExt, NodeQueryExt};

use crate::BootError;

pub struct FirmwareBootContext {
    pub boot_mode: FirmwareBootMode,
    pub boot_partition: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirmwareBootMode {
    SdOrEmmc,
    Network,
    Rpiboot,
    UsbMsd,
    Nvme,
    Http,
    Unknown(u32),
}

impl FirmwareBootContext {
    pub fn from_dtb(parser: &dtb::DtbParser) -> Result<Self, BootError> {
        let tree =
            DeviceTree::from_parser(parser).map_err(|_| BootError::BootModeDtbNodeInvalid)?;
        let bootloader = tree
            .find_node_by_path("/chosen/bootloader")
            .and_then(|id| tree.node(id))
            .ok_or(BootError::BootModeDtbNodeMissing)?;
        let boot_mode = parse_boot_mode_prop(
            bootloader
                .property("boot-mode")
                .map(|prop| prop.value.as_slice()),
        )?;
        let boot_partition = parse_optional_be_u32_prop(
            bootloader
                .property("partition")
                .map(|prop| prop.value.as_slice()),
        );

        Ok(Self {
            boot_mode,
            boot_partition,
        })
    }

    pub fn log(&self) {
        crate::logln!(
            "[BOOTCTX] boot_mode={} source={} partition={}",
            self.boot_mode.raw_value(),
            self.boot_mode.as_str(),
            OptionalU32(self.boot_partition)
        );
    }

    pub fn enforce_default_policy(&self) -> Result<(), BootError> {
        match self.boot_mode {
            FirmwareBootMode::Network => Ok(()),
            FirmwareBootMode::SdOrEmmc => {
                crate::logln!("[BOOTCTX] reject firmware boot source=sd-emmc");
                Err(BootError::FirmwareBootedFromSdOrEmmc)
            }
            FirmwareBootMode::UsbMsd => {
                crate::logln!("[BOOTCTX] reject firmware boot source=usb-msd");
                Err(BootError::FirmwareBootedFromUsbMsd)
            }
            FirmwareBootMode::Rpiboot
            | FirmwareBootMode::Nvme
            | FirmwareBootMode::Http
            | FirmwareBootMode::Unknown(_) => {
                crate::logln!(
                    "[BOOTCTX] reject unsupported firmware boot source={}",
                    self.boot_mode.as_str()
                );
                Err(BootError::FirmwareBootModeUnsupported)
            }
        }
    }
}

impl FirmwareBootMode {
    pub const fn from_raw(value: u32) -> Self {
        match value {
            1 => Self::SdOrEmmc,
            2 => Self::Network,
            3 => Self::Rpiboot,
            4 => Self::UsbMsd,
            6 => Self::Nvme,
            7 => Self::Http,
            other => Self::Unknown(other),
        }
    }

    pub const fn raw_value(self) -> u32 {
        match self {
            Self::SdOrEmmc => 1,
            Self::Network => 2,
            Self::Rpiboot => 3,
            Self::UsbMsd => 4,
            Self::Nvme => 6,
            Self::Http => 7,
            Self::Unknown(value) => value,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SdOrEmmc => "sd-emmc",
            Self::Network => "network",
            Self::Rpiboot => "rpiboot",
            Self::UsbMsd => "usb-msd",
            Self::Nvme => "nvme",
            Self::Http => "http",
            Self::Unknown(_) => "unknown",
        }
    }
}

fn parse_boot_mode_prop(bytes: Option<&[u8]>) -> Result<FirmwareBootMode, BootError> {
    let Some(bytes) = bytes else {
        return Err(BootError::BootModeDtbNodeMissing);
    };
    let Some(raw) = parse_be_u32_prefix(bytes) else {
        return Err(BootError::BootModeDtbNodeInvalid);
    };
    Ok(FirmwareBootMode::from_raw(raw))
}

fn parse_optional_be_u32_prop(bytes: Option<&[u8]>) -> Option<u32> {
    parse_be_u32_prefix(bytes?)
}

fn parse_be_u32_prefix(bytes: &[u8]) -> Option<u32> {
    let bytes = bytes.get(..4)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

struct OptionalU32(Option<u32>);

impl core::fmt::Display for OptionalU32 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            Some(value) => write!(f, "{}", value),
            None => f.write_str("missing"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_big_endian_boot_mode() {
        assert_eq!(
            parse_boot_mode_prop(Some(&[0, 0, 0, 2])).unwrap(),
            FirmwareBootMode::Network
        );
    }

    #[test]
    fn decodes_known_boot_modes() {
        assert_eq!(FirmwareBootMode::from_raw(1), FirmwareBootMode::SdOrEmmc);
        assert_eq!(FirmwareBootMode::from_raw(2), FirmwareBootMode::Network);
        assert_eq!(FirmwareBootMode::from_raw(4), FirmwareBootMode::UsbMsd);
    }

    #[test]
    fn missing_boot_mode_is_error() {
        assert_eq!(
            parse_boot_mode_prop(None),
            Err(BootError::BootModeDtbNodeMissing)
        );
        assert_eq!(
            parse_boot_mode_prop(Some(&[0, 0, 0])),
            Err(BootError::BootModeDtbNodeInvalid)
        );
    }
}
