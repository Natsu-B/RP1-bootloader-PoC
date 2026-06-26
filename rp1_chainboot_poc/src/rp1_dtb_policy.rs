use rp1_abi::owner::{
    DEV_DMA, DEV_GPIO, DEV_I2C0, DEV_I2C1, DEV_PIO0, DEV_PIO1, DEV_SPI0, DEV_TIMER, DEV_UART0,
    DEV_UART1, bit,
};

use crate::BootError;

pub const RP1_KNOWN_DEVICE_MASK: u64 = bit(DEV_GPIO)
    | bit(DEV_UART0)
    | bit(DEV_UART1)
    | bit(DEV_I2C0)
    | bit(DEV_I2C1)
    | bit(DEV_SPI0)
    | bit(DEV_PIO0)
    | bit(DEV_PIO1)
    | bit(DEV_DMA)
    | bit(DEV_TIMER);

#[derive(Clone, Copy)]
pub struct Rp1DtbPolicy {
    pub owner_rp1: u64,
    pub owner_linux: u64,
    pub owner_disabled: u64,
    pub source: Rp1DtbPolicySource,
}

#[derive(Clone, Copy)]
pub enum Rp1DtbPolicySource {
    Note,
    Config,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Rp1DeviceOwner {
    Rp1,
    Linux,
    Disabled,
    Unspecified,
}

impl Rp1DtbPolicy {
    pub fn from_note(note: &crate::rp1_note::Rp1BootInfo) -> Result<Self, BootError> {
        let policy = Self {
            owner_rp1: note.owner_rp1,
            owner_linux: note.owner_linux,
            owner_disabled: note.owner_disabled,
            source: Rp1DtbPolicySource::Note,
        };
        policy.validate()?;
        Ok(policy)
    }

    pub fn from_config(config: &crate::rp1_config::Rp1Config) -> Result<Self, BootError> {
        if !config.owner_table_seen {
            return Err(BootError::Rp1ConfigInvalid);
        }
        let policy = Self {
            owner_rp1: config.owner_rp1,
            owner_linux: config.owner_linux,
            owner_disabled: config.owner_disabled,
            source: Rp1DtbPolicySource::Config,
        };
        policy.validate()?;
        Ok(policy)
    }

    pub fn validate(&self) -> Result<(), BootError> {
        let overlap = (self.owner_rp1 & self.owner_linux)
            | (self.owner_rp1 & self.owner_disabled)
            | (self.owner_linux & self.owner_disabled);
        if overlap != 0 {
            return Err(BootError::Rp1DtbPolicyInvalid);
        }

        let specified = self.owner_rp1 | self.owner_linux | self.owner_disabled;
        if specified & !RP1_KNOWN_DEVICE_MASK != 0 {
            return Err(BootError::Rp1DtbPolicyInvalid);
        }
        if specified & RP1_KNOWN_DEVICE_MASK != RP1_KNOWN_DEVICE_MASK {
            return Err(BootError::Rp1DtbPolicyInvalid);
        }
        if self.owner_linux & (bit(DEV_PIO0) | bit(DEV_PIO1)) != 0 {
            return Err(BootError::Rp1DtbPolicyInvalid);
        }

        Ok(())
    }

    pub fn owner_of(&self, dev_bit: u8) -> Rp1DeviceOwner {
        let mask = bit(dev_bit);
        if self.owner_rp1 & mask != 0 {
            Rp1DeviceOwner::Rp1
        } else if self.owner_linux & mask != 0 {
            Rp1DeviceOwner::Linux
        } else if self.owner_disabled & mask != 0 {
            Rp1DeviceOwner::Disabled
        } else {
            Rp1DeviceOwner::Unspecified
        }
    }
}

impl Rp1DtbPolicySource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Config => "config",
        }
    }
}

impl Rp1DeviceOwner {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rp1 => "rp1",
            Self::Linux => "linux",
            Self::Disabled => "disabled",
            Self::Unspecified => "unspecified",
        }
    }

    pub const fn linux_status(self) -> Option<&'static str> {
        match self {
            Self::Rp1 | Self::Disabled => Some("disabled"),
            Self::Linux => Some("okay"),
            Self::Unspecified => None,
        }
    }
}

pub struct Rp1DeviceDtbNode {
    pub bit: u8,
    pub name: &'static str,
    pub fallback_paths: &'static [&'static str],
}

pub const RP1_DEVICE_DTB_NODES: &[Rp1DeviceDtbNode] = &[
    Rp1DeviceDtbNode {
        bit: DEV_GPIO,
        name: "gpio",
        fallback_paths: &[
            "/axi/pcie@1000120000/rp1/gpio@d0000",
            "/axi/pcie@120000/rp1/gpio@d0000",
            "/soc/rp1/gpio@d0000",
        ],
    },
    Rp1DeviceDtbNode {
        bit: DEV_UART0,
        name: "uart0",
        fallback_paths: &[
            "/axi/pcie@1000120000/rp1/serial@30000",
            "/axi/pcie@120000/rp1/uart@30000",
            "/soc/rp1/uart@30000",
        ],
    },
    Rp1DeviceDtbNode {
        bit: DEV_UART1,
        name: "uart1",
        fallback_paths: &[
            "/axi/pcie@1000120000/rp1/serial@34000",
            "/axi/pcie@120000/rp1/uart@34000",
            "/soc/rp1/uart@34000",
        ],
    },
    Rp1DeviceDtbNode {
        bit: DEV_I2C0,
        name: "i2c0",
        fallback_paths: &[
            "/axi/pcie@1000120000/rp1/i2c@70000",
            "/axi/pcie@120000/rp1/i2c@70000",
            "/soc/rp1/i2c@70000",
        ],
    },
    Rp1DeviceDtbNode {
        bit: DEV_I2C1,
        name: "i2c1",
        fallback_paths: &[
            "/axi/pcie@1000120000/rp1/i2c@74000",
            "/axi/pcie@120000/rp1/i2c@74000",
            "/soc/rp1/i2c@74000",
        ],
    },
    Rp1DeviceDtbNode {
        bit: DEV_SPI0,
        name: "spi0",
        fallback_paths: &[
            "/axi/pcie@1000120000/rp1/spi@50000",
            "/axi/pcie@120000/rp1/spi@50000",
            "/soc/rp1/spi@50000",
        ],
    },
    Rp1DeviceDtbNode {
        bit: DEV_PIO0,
        name: "pio0",
        fallback_paths: &[
            "/axi/pcie@1000120000/rp1/pio@178000",
            "/axi/pcie@120000/rp1/pio@178000",
            "/soc/rp1/pio@178000",
        ],
    },
    Rp1DeviceDtbNode {
        bit: DEV_PIO1,
        name: "pio1",
        fallback_paths: &[
            "/axi/pcie@1000120000/rp1/pio@178000",
            "/axi/pcie@120000/rp1/pio@178000",
            "/soc/rp1/pio@178000",
        ],
    },
    Rp1DeviceDtbNode {
        bit: DEV_DMA,
        name: "dma",
        fallback_paths: &[
            "/axi/pcie@1000120000/rp1/dma@188000",
            "/axi/pcie@120000/rp1/dma@188000",
            "/soc/rp1/dma@188000",
        ],
    },
    Rp1DeviceDtbNode {
        bit: DEV_TIMER,
        name: "timer",
        fallback_paths: &[],
    },
];
