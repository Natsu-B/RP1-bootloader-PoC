pub struct Rp1Config {
    pub force_boot: bool,
    pub linux_pio: bool,
}

impl Rp1Config {
    const fn default() -> Self {
        Self {
            force_boot: false,
            linux_pio: false,
        }
    }
}

pub fn parse_optional_config(bytes: Option<&[u8]>) -> Result<Rp1Config, ()> {
    let Some(bytes) = bytes else {
        return Ok(Rp1Config::default());
    };
    let text = core::str::from_utf8(bytes).map_err(|_| ())?;
    let mut config = Rp1Config::default();

    for raw_line in text.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(());
        };
        match (key.trim(), value.trim()) {
            ("force_boot", "true") => config.force_boot = true,
            ("force_boot", "false") => config.force_boot = false,
            ("linux_pio", "false") => config.linux_pio = false,
            ("linux_pio", "true") => return Err(()),
            _ => return Err(()),
        }
    }

    Ok(config)
}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(idx) => &line[..idx],
        None => line,
    }
}
