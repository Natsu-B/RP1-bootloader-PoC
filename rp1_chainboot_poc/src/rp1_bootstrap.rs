use crate::BootError;
use crate::bcm2712_aon::Rp1RunPin;
use crate::rp1_image::Rp1Image;

pub const RP1_I2C_ADDR: u8 = 0x43;
pub const RP1_CHUNK_SIZE: usize = 0x40;
pub const RP1_BOOT_MAGIC: u32 = 0xb007_c0de;
pub const RP1_SCRATCH_MAGIC: u32 = 0x4015_400c;
pub const RP1_SCRATCH_ENTRY: u32 = 0x4015_4010;
pub const RP1_SCRATCH_STACK: u32 = 0x4015_4018;
pub const RP1_BOOT_CTRL_A: u32 = 0x4001_0008;
pub const RP1_BOOT_COMMAND: u32 = 0x4015_4000;
pub const RP1_CHIP_ID: u32 = 0x4000_0000;
pub const RP1_RESETS_CLR: u32 = 0x4001_7004;
pub const RP1_RESETS_ALL_CLR_MASK_FOR_CHIP_ID: u32 = 0x0080_0000;
pub const RP1_PROBE_CHIP_ID_REQUIRED: bool = true;
pub const RP1_RESET_LOW_US: u64 = 50_000;
pub const RP1_RESET_HIGH_SETTLE_US: u64 = 10_000;
pub const RP1_RESET_CLEAR_RETRY_DELAY_US: u64 = 1_000;
pub const RP1_CHIP_ID_RETRY_DELAY_US: u64 = 1_000;

pub trait Rp1I2cBus {
    type Error;

    fn write(&mut self, addr: u8, bytes: &[u8]) -> Result<(), Self::Error>;
    fn write_read(&mut self, addr: u8, bytes: &[u8], read: &mut [u8]) -> Result<(), Self::Error>;
}

pub struct Rp1Bootstrap<I2C> {
    i2c: I2C,
    run: Rp1RunPin,
}

impl<I2C> Rp1Bootstrap<I2C>
where
    I2C: Rp1I2cBus,
{
    pub fn new(i2c: I2C, run: Rp1RunPin) -> Self {
        Self { i2c, run }
    }

    pub fn reset_into_bootrom(&mut self) -> Result<Option<u32>, BootError> {
        crate::logln!("[RP1BOOT] reset low");
        self.run.set_low()?;
        crate::logln!("[RP1BOOT] reset low delay {} us", RP1_RESET_LOW_US);
        crate::timer::delay_micros(RP1_RESET_LOW_US);

        crate::logln!("[RP1BOOT] reset high");
        self.run.set_high()?;
        crate::logln!(
            "[RP1BOOT] reset high settle {} us",
            RP1_RESET_HIGH_SETTLE_US
        );
        crate::timer::delay_micros(RP1_RESET_HIGH_SETTLE_US);

        let mut last = BootError::I2cWrite;
        let mut reset_clear_ok = false;
        for attempt in 0..100 {
            match self.write32(RP1_RESETS_CLR, RP1_RESETS_ALL_CLR_MASK_FOR_CHIP_ID) {
                Ok(()) => {
                    reset_clear_ok = true;
                    crate::logln!("[RP1BOOT] reset clear ok after {} retries", attempt);
                    break;
                }
                Err(err) => {
                    last = err;
                    crate::timer::delay_micros(RP1_RESET_CLEAR_RETRY_DELAY_US);
                }
            }
        }
        if !reset_clear_ok {
            crate::logln!("[RP1BOOT] reset clear write failed: {:?}", last);
            return Err(last);
        }
        crate::logln!("[RP1BOOT] reset clear for chip-id probe");
        crate::timer::delay_micros(RP1_RESET_CLEAR_RETRY_DELAY_US);

        let mut last = BootError::I2cNack;
        for attempt in 0..50 {
            match self.probe_chip_id() {
                Ok(chip_id) => {
                    crate::logln!("[RP1BOOT] i2c 0x43 ack ok after {} retries", attempt);
                    return Ok(Some(chip_id));
                }
                Err(err) => {
                    last = err;
                    crate::timer::delay_micros(RP1_CHIP_ID_RETRY_DELAY_US);
                }
            }
        }
        crate::logln!(
            "[RP1BOOT] chip-id ack/read failed after reset clear; check I2C repeated-start behavior"
        );
        if RP1_PROBE_CHIP_ID_REQUIRED {
            Err(last)
        } else {
            crate::logln!("[RP1BOOT] chip-id probe is optional in this build; continuing");
            Ok(None)
        }
    }

    pub fn probe_chip_id(&mut self) -> Result<u32, BootError> {
        let mut bytes = [0u8; 4];
        self.read_mem(RP1_CHIP_ID, &mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    pub fn write_mem(&mut self, addr: u32, data: &[u8]) -> Result<(), BootError> {
        if data.len() > RP1_CHUNK_SIZE {
            return Err(BootError::Rp1ChunkTooLarge);
        }
        let mut packet = [0u8; 4 + RP1_CHUNK_SIZE];
        packet[0..4].copy_from_slice(&addr.to_be_bytes());
        packet[4..4 + data.len()].copy_from_slice(data);
        self.i2c
            .write(RP1_I2C_ADDR, &packet[..4 + data.len()])
            .map_err(|_| BootError::I2cWrite)
    }

    pub fn read_mem(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), BootError> {
        self.i2c
            .write_read(RP1_I2C_ADDR, &addr.to_be_bytes(), buf)
            .map_err(|_| BootError::I2cRead)
    }

    pub fn write32(&mut self, addr: u32, value: u32) -> Result<(), BootError> {
        self.write_mem(addr, &value.to_le_bytes())
    }

    pub fn load_image(&mut self, image: &Rp1Image<'_>) -> Result<(), BootError> {
        let mut off = 0usize;
        while off < image.payload.len() {
            let n = core::cmp::min(RP1_CHUNK_SIZE, image.payload.len() - off);
            let dst = image
                .load_addr
                .checked_add(off as u32)
                .ok_or(BootError::AddressOverflow)?;
            self.write_mem(dst, &image.payload[off..off + n])?;
            off += n;
        }
        Ok(())
    }

    pub fn program_scratch(&mut self, entry: u32, stack: u32) -> Result<(), BootError> {
        let entry = entry | 1;
        self.write32(RP1_SCRATCH_MAGIC, RP1_BOOT_MAGIC)?;
        self.write32(RP1_SCRATCH_ENTRY, entry ^ RP1_BOOT_MAGIC)?;
        self.write32(RP1_SCRATCH_STACK, stack)?;
        Ok(())
    }

    pub fn start(&mut self) -> Result<(), BootError> {
        self.write32(RP1_BOOT_CTRL_A, 0x100)?;
        self.write32(RP1_BOOT_COMMAND, 0x8000_0000)?;
        Ok(())
    }

    pub fn load_and_start(&mut self, image: &Rp1Image<'_>) -> Result<(), BootError> {
        self.load_image(image)?;
        crate::logln!("[RP1BOOT] image loaded");
        self.program_scratch(image.entry, image.stack)?;
        crate::logln!("[RP1BOOT] scratch programmed");
        self.start()?;
        crate::logln!("[RP1BOOT] proc0 started");
        Ok(())
    }
}
