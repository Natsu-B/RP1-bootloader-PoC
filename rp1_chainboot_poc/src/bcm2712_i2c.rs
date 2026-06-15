use crate::BootError;
use crate::rp1_bootstrap::Rp1I2cBus;

const I2C3_FALLBACK_BASE: usize = 0x10_7d50_c000;
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

const POLL_LIMIT: usize = 2_000_000;

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
        i2c.init();
        i2c
    }

    fn init(&self) {
        self.write32(C, C_CLEAR);
        self.write32(DIV, 2500);
        self.write32(CLKT, 0x40);
        self.write32(S, u32::MAX);
        self.write32(C, C_I2CEN);
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
            self.wait_for(S_TXD | S_ERR | S_CLKT)?;
            self.check_error()?;
            self.write32(FIFO, u32::from(bytes[pos]));
            pos += 1;
        }
        self.wait_done()
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
            self.wait_for(S_RXD | S_DONE | S_ERR | S_CLKT)?;
            self.check_error()?;
            while pos < buf.len() && (self.read32(S) & S_RXD) != 0 {
                buf[pos] = self.read32(FIFO) as u8;
                pos += 1;
            }
        }
        self.wait_done()
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
