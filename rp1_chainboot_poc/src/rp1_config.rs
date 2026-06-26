use rp1_abi::owner::{
    DEV_DMA, DEV_GPIO, DEV_I2C0, DEV_I2C1, DEV_PIO0, DEV_PIO1, DEV_SPI0, DEV_TIMER, DEV_UART0,
    DEV_UART1, bit,
};

pub struct Rp1Config {
    pub force_boot: bool,
    pub linux_pio: bool,
    pub owner_table_seen: bool,
    pub owner_rp1: u64,
    pub owner_linux: u64,
    pub owner_disabled: u64,
}

impl Rp1Config {
    const fn default() -> Self {
        Self {
            force_boot: false,
            linux_pio: false,
            owner_table_seen: false,
            owner_rp1: 0,
            owner_linux: 0,
            owner_disabled: 0,
        }
    }
}

pub fn parse_optional_config(bytes: Option<&[u8]>) -> Result<Rp1Config, ()> {
    let Some(bytes) = bytes else {
        return Ok(Rp1Config::default());
    };
    let text = core::str::from_utf8(bytes).map_err(|_| ())?;
    let mut config = Rp1Config::default();
    let mut section = Section::Root;

    for raw_line in text.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if line == "[owner]" {
            section = Section::Owner;
            config.owner_table_seen = true;
            continue;
        }
        if line.starts_with('[') {
            return Err(());
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(());
        };
        let key = key.trim();
        let value = trim_value(value.trim());
        match section {
            Section::Root => match (key, value) {
                ("force_boot", "true") => config.force_boot = true,
                ("force_boot", "false") => config.force_boot = false,
                ("linux_pio", "false") => config.linux_pio = false,
                ("linux_pio", "true") => return Err(()),
                _ => return Err(()),
            },
            Section::Owner => {
                let bit = owner_key_bit(key).ok_or(())?;
                set_owner(&mut config, bit, value)?;
            }
        }
    }

    Ok(config)
}

#[derive(Clone, Copy)]
enum Section {
    Root,
    Owner,
}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(idx) => &line[..idx],
        None => line,
    }
}

fn trim_value(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 && bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"' {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

fn owner_key_bit(key: &str) -> Option<u8> {
    match key {
        "gpio" => Some(DEV_GPIO),
        "uart0" => Some(DEV_UART0),
        "uart1" => Some(DEV_UART1),
        "i2c0" => Some(DEV_I2C0),
        "i2c1" => Some(DEV_I2C1),
        "spi0" => Some(DEV_SPI0),
        "pio0" => Some(DEV_PIO0),
        "pio1" => Some(DEV_PIO1),
        "dma" => Some(DEV_DMA),
        "timer" => Some(DEV_TIMER),
        _ => None,
    }
}

fn set_owner(config: &mut Rp1Config, dev: u8, owner: &str) -> Result<(), ()> {
    let mask = bit(dev);
    if (config.owner_rp1 | config.owner_linux | config.owner_disabled) & mask != 0 {
        return Err(());
    }
    match owner {
        "rp1" => config.owner_rp1 |= mask,
        "linux" => config.owner_linux |= mask,
        "disabled" => config.owner_disabled |= mask,
        _ => return Err(()),
    }
    Ok(())
}
