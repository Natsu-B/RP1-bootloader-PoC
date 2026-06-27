use dtb::{DeviceTree, DeviceTreeQueryExt, NodeQueryExt};
use net::Ipv4Addr;

use crate::BootError;

pub struct FirmwareBootContext {
    pub boot_mode: FirmwareBootMode,
    pub boot_partition: Option<u32>,
    pub tftp_ip: Option<Ipv4Addr>,
    pub tftp_ip_source: Option<&'static str>,
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
        let (tftp_ip, tftp_ip_source) = find_tftp_ip(&tree);

        Ok(Self {
            boot_mode,
            boot_partition,
            tftp_ip,
            tftp_ip_source,
        })
    }

    pub fn log(&self) {
        crate::logln!(
            "[BOOTCTX] boot_mode={} source={} partition={}",
            self.boot_mode.raw_value(),
            self.boot_mode.as_str(),
            OptionalU32(self.boot_partition)
        );
        match (self.tftp_ip, self.tftp_ip_source) {
            (Some(ip), Some(source)) => crate::logln!(
                "[BOOTCTX] tftp_ip={}.{}.{}.{} source={}",
                ip[0],
                ip[1],
                ip[2],
                ip[3],
                source
            ),
            _ => crate::logln!("[BOOTCTX] tftp_ip=missing"),
        }
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

struct TftpIpCandidate {
    node_path: &'static str,
    property: &'static str,
    source: &'static str,
}

const TFTP_IP_CANDIDATES: &[TftpIpCandidate] = &[
    TftpIpCandidate {
        node_path: "/chosen/bootloader",
        property: "tftp",
        source: "/chosen/bootloader/tftp",
    },
    TftpIpCandidate {
        node_path: "/chosen/bootloader",
        property: "tftp-ip",
        source: "/chosen/bootloader/tftp-ip",
    },
    TftpIpCandidate {
        node_path: "/chosen/bootloader",
        property: "tftp-server",
        source: "/chosen/bootloader/tftp-server",
    },
    TftpIpCandidate {
        node_path: "/chosen/bootloader",
        property: "server-ip",
        source: "/chosen/bootloader/server-ip",
    },
    TftpIpCandidate {
        node_path: "/chosen",
        property: "tftp",
        source: "/chosen/tftp",
    },
    TftpIpCandidate {
        node_path: "/chosen",
        property: "tftp-ip",
        source: "/chosen/tftp-ip",
    },
    TftpIpCandidate {
        node_path: "/chosen",
        property: "tftp-server",
        source: "/chosen/tftp-server",
    },
    TftpIpCandidate {
        node_path: "/chosen",
        property: "server-ip",
        source: "/chosen/server-ip",
    },
];

fn find_tftp_ip(tree: &DeviceTree<'_, dtb::Borrowed>) -> (Option<Ipv4Addr>, Option<&'static str>) {
    for candidate in TFTP_IP_CANDIDATES {
        let Some(node) = tree
            .find_node_by_path(candidate.node_path)
            .and_then(|id| tree.node(id))
        else {
            continue;
        };
        let Some(bytes) = node
            .property(candidate.property)
            .map(|prop| prop.value.as_slice())
        else {
            continue;
        };
        if let Some(ip) = parse_tftp_ip_prop(bytes) {
            return (Some(ip), Some(candidate.source));
        }
    }
    (None, None)
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

fn parse_tftp_ip_prop(bytes: &[u8]) -> Option<Ipv4Addr> {
    parse_ascii_ipv4(bytes).or_else(|| {
        if bytes.len() == 4 {
            Some([bytes[0], bytes[1], bytes[2], bytes[3]])
        } else {
            None
        }
    })
}

fn parse_be_u32_prefix(bytes: &[u8]) -> Option<u32> {
    let bytes = bytes.get(..4)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn parse_ascii_ipv4(bytes: &[u8]) -> Option<Ipv4Addr> {
    let bytes = trim_ascii_nul_space(bytes);
    if bytes.is_empty() {
        return None;
    }

    let mut octets = [0u8; 4];
    let mut idx = 0usize;
    let mut value = 0u16;
    let mut saw_digit = false;

    for &b in bytes {
        match b {
            b'0'..=b'9' => {
                saw_digit = true;
                value = value.checked_mul(10)?.checked_add(u16::from(b - b'0'))?;
                if value > u16::from(u8::MAX) {
                    return None;
                }
            }
            b'.' => {
                if !saw_digit || idx >= 3 {
                    return None;
                }
                octets[idx] = value as u8;
                idx += 1;
                value = 0;
                saw_digit = false;
            }
            _ => return None,
        }
    }

    if !saw_digit || idx != 3 {
        return None;
    }
    octets[idx] = value as u8;
    Some(octets)
}

fn trim_ascii_nul_space(mut bytes: &[u8]) -> &[u8] {
    while let Some((&last, rest)) = bytes.split_last() {
        if last == 0 || last == b' ' || last == b'\n' || last == b'\r' || last == b'\t' {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
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

    #[test]
    fn parses_ascii_tftp_ip() {
        assert_eq!(
            parse_tftp_ip_prop(b"192.168.1.10\0"),
            Some([192, 168, 1, 10])
        );
    }

    #[test]
    fn parses_big_endian_u32_tftp_ip() {
        assert_eq!(
            parse_tftp_ip_prop(&[192, 168, 50, 1]),
            Some([192, 168, 50, 1])
        );
    }

    #[test]
    fn invalid_tftp_ip_is_none() {
        assert_eq!(parse_tftp_ip_prop(b"not-an-ip"), None);
        assert_eq!(parse_tftp_ip_prop(&[1, 2, 3]), None);
    }
}
