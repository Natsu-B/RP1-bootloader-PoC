use crate::BootError;
use crate::rp1_bootstrap::Rp1I2cBus;

const I2C3_FALLBACK_BASE: usize = 0x10_7d00_5600;
const AON_PINCTRL_BASE: usize = 0x10_7d51_0700;
const C: usize = 0x00;
const S: usize = 0x04;
const DLEN: usize = 0x08;
const A: usize = 0x0c;
const FIFO: usize = 0x10;
const DIV: usize = 0x14;
const CLKT: usize = 0x1c;

const C_I2CEN: u32 = 1 << 15;
const C_ST: u32 = 1 << 7;
const C_CLEAR: u32 = 0b11 << 4;
const C_READ: u32 = 1;
const S_CLKT: u32 = 1 << 9;
const S_ERR: u32 = 1 << 8;
const S_DONE: u32 = 1 << 1;
const S_TA: u32 = 1;
const S_TXD: u32 = 1 << 4;
const S_RXD: u32 = 1 << 5;

const POLL_LIMIT: usize = 100_000;
const AON_I2C3_MUX_FUNC: u32 = 4;
const AON_PULL_UP: u32 = 2;

pub struct Bcm2712I2c {
    base: usize,
}

impl Bcm2712I2c {
    pub fn from_dtb_or_fallback(_dtb: &dtb::DtbParser) -> Self {
        crate::logln!(
            "[RP1BOOT] using I2C fallback for bootstrap base=0x{:x}",
            I2C3_FALLBACK_BASE
        );
        let i2c = Self {
            base: I2C3_FALLBACK_BASE,
        };
        configure_aon_i2c3_pins();
        i2c.init();
        i2c
    }

    fn init(&self) {
        self.write32(C, C_CLEAR);
        self.write32(DIV, 2500);
        self.write32(CLKT, 0x40);
        self.write32(S, u32::MAX);
        self.write32(C, C_I2CEN);
        crate::logln!(
            "[RP1BOOT] i2c init C=0x{:08x} S=0x{:08x} DIV={}",
            self.read32(C),
            self.read32(S),
            self.read32(DIV)
        );
    }

    fn transfer_write(&self, addr: u8, bytes: &[u8]) -> Result<(), BootError> {
        if bytes.len() > u16::MAX as usize {
            return Err(BootError::I2cWrite);
        }
        self.write32(A, u32::from(addr));
        self.write32(DLEN, bytes.len() as u32);
        self.write32(S, u32::MAX);
        self.write32(C, C_I2CEN | C_CLEAR);

        let mut pos = 0usize;
        self.write32(C, C_I2CEN | C_ST);
        while pos < bytes.len() {
            if let Err(err) = self.wait_for(S_TXD | S_ERR | S_CLKT) {
                crate::logln!(
                    "[RP1BOOT] i2c write wait TXD failed pos={} len={} S=0x{:08x} C=0x{:08x}",
                    pos,
                    bytes.len(),
                    self.read32(S),
                    self.read32(C)
                );
                return Err(err);
            }
            if let Err(err) = self.check_error() {
                crate::logln!(
                    "[RP1BOOT] i2c write error pos={} len={} S=0x{:08x} C=0x{:08x}",
                    pos,
                    bytes.len(),
                    self.read32(S),
                    self.read32(C)
                );
                return Err(err);
            }
            self.write32(FIFO, u32::from(bytes[pos]));
            pos += 1;
        }
        if let Err(err) = self.wait_done() {
            crate::logln!(
                "[RP1BOOT] i2c write done failed len={} S=0x{:08x} C=0x{:08x}",
                bytes.len(),
                self.read32(S),
                self.read32(C)
            );
            return Err(err);
        }
        Ok(())
    }

    fn transfer_read(&self, addr: u8, buf: &mut [u8]) -> Result<(), BootError> {
        if buf.len() > u16::MAX as usize {
            return Err(BootError::I2cRead);
        }
        self.write32(A, u32::from(addr));
        self.write32(DLEN, buf.len() as u32);
        self.write32(S, u32::MAX);
        self.write32(C, C_I2CEN | C_CLEAR | C_READ | C_ST);

        let mut pos = 0usize;
        while pos < buf.len() {
            if let Err(err) = self.wait_for(S_RXD | S_DONE | S_ERR | S_CLKT) {
                crate::logln!(
                    "[RP1BOOT] i2c read wait RXD failed pos={} len={} S=0x{:08x} C=0x{:08x}",
                    pos,
                    buf.len(),
                    self.read32(S),
                    self.read32(C)
                );
                return Err(err);
            }
            if let Err(err) = self.check_error() {
                crate::logln!(
                    "[RP1BOOT] i2c read error pos={} len={} S=0x{:08x} C=0x{:08x}",
                    pos,
                    buf.len(),
                    self.read32(S),
                    self.read32(C)
                );
                return Err(err);
            }
            while pos < buf.len() && (self.read32(S) & S_RXD) != 0 {
                buf[pos] = self.read32(FIFO) as u8;
                pos += 1;
            }
        }
        if let Err(err) = self.wait_done() {
            crate::logln!(
                "[RP1BOOT] i2c read done failed len={} S=0x{:08x} C=0x{:08x}",
                buf.len(),
                self.read32(S),
                self.read32(C)
            );
            return Err(err);
        }
        Ok(())
    }

    fn wait_done(&self) -> Result<(), BootError> {
        for _ in 0..POLL_LIMIT {
            let s = self.read32(S);
            if (s & (S_ERR | S_CLKT)) != 0 {
                return Err(BootError::I2cNack);
            }
            if (s & S_DONE) != 0 && (s & S_TA) == 0 {
                self.write32(S, u32::MAX);
                return Ok(());
            }
        }
        Err(BootError::I2cTimeout)
    }

    fn wait_for(&self, mask: u32) -> Result<(), BootError> {
        for _ in 0..POLL_LIMIT {
            if (self.read32(S) & mask) != 0 {
                return Ok(());
            }
        }
        Err(BootError::I2cTimeout)
    }

    fn check_error(&self) -> Result<(), BootError> {
        let s = self.read32(S);
        if (s & S_CLKT) != 0 {
            Err(BootError::I2cTimeout)
        } else if (s & S_ERR) != 0 {
            Err(BootError::I2cNack)
        } else {
            Ok(())
        }
    }

    fn read32(&self, off: usize) -> u32 {
        // SAFETY: MMIO base is DT-derived or the documented bootstrap fallback.
        unsafe { core::ptr::read_volatile((self.base + off) as *const u32) }
    }

    fn write32(&self, off: usize, value: u32) {
        // SAFETY: MMIO base is DT-derived or the documented bootstrap fallback.
        unsafe { core::ptr::write_volatile((self.base + off) as *mut u32, value) }
    }
}

fn configure_aon_i2c3_pins() {
    let mux_before = read32_at(AON_PINCTRL_BASE, 0x0c);
    let pull14_before = read32_at(AON_PINCTRL_BASE, 0x14);
    let pull18_before = read32_at(AON_PINCTRL_BASE, 0x18);

    let mux = (mux_before & !((0xf << 0) | (0xf << 4)))
        | (AON_I2C3_MUX_FUNC << 0)
        | (AON_I2C3_MUX_FUNC << 4);
    write32_at(AON_PINCTRL_BASE, 0x0c, mux);

    let pull14 =
        (pull14_before & !((0x3 << 18) | (0x3 << 20))) | (AON_PULL_UP << 18) | (AON_PULL_UP << 20);
    write32_at(AON_PINCTRL_BASE, 0x14, pull14);

    let pull18 =
        (pull18_before & !((0x3 << 20) | (0x3 << 22))) | (AON_PULL_UP << 20) | (AON_PULL_UP << 22);
    write32_at(AON_PINCTRL_BASE, 0x18, pull18);

    crate::logln!(
        "[RP1BOOT] i2c3 aon pinmux mux 0x{:08x}->0x{:08x} pull14 0x{:08x}->0x{:08x} pull18 0x{:08x}->0x{:08x}",
        mux_before,
        read32_at(AON_PINCTRL_BASE, 0x0c),
        pull14_before,
        read32_at(AON_PINCTRL_BASE, 0x14),
        pull18_before,
        read32_at(AON_PINCTRL_BASE, 0x18)
    );
}

fn read32_at(base: usize, off: usize) -> u32 {
    // SAFETY: AON pinctrl is fixed BCM2712 MMIO used only to select RP1 bootstrap I2C pins.
    unsafe { core::ptr::read_volatile((base + off) as *const u32) }
}

fn write32_at(base: usize, off: usize, value: u32) {
    // SAFETY: AON pinctrl is fixed BCM2712 MMIO used only to select RP1 bootstrap I2C pins.
    unsafe { core::ptr::write_volatile((base + off) as *mut u32, value) }
}

impl Rp1I2cBus for Bcm2712I2c {
    type Error = BootError;

    fn write(&mut self, addr: u8, bytes: &[u8]) -> Result<(), Self::Error> {
        self.transfer_write(addr, bytes)
    }

    fn write_read(&mut self, addr: u8, bytes: &[u8], read: &mut [u8]) -> Result<(), Self::Error> {
        self.transfer_write(addr, bytes)?;
        self.transfer_read(addr, read)
    }
}
