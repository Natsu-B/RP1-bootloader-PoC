use crate::BootError;

const AON_GPIO_FALLBACK_BASE: usize = 0x10_7d51_7c00;
const GPIO2_BIT: u32 = 1 << 2;
const GIO_DATA: usize = 0x04;
const GIO_IODIR: usize = 0x08;

#[derive(Clone, Copy)]
pub struct Rp1RunPin {
    base: usize,
    pin_mask: u32,
    active_low: bool,
}

impl Rp1RunPin {
    pub fn from_dtb_or_fallback(_dtb: &dtb::DtbParser) -> Self {
        crate::logln!(
            "[RP1BOOT] using AON GPIO fallback for RP1_RUN base=0x{:x} pin=2",
            AON_GPIO_FALLBACK_BASE
        );
        Self {
            base: AON_GPIO_FALLBACK_BASE,
            pin_mask: GPIO2_BIT,
            active_low: false,
        }
    }

    pub fn set_low(&mut self) -> Result<(), BootError> {
        self.configure_output();
        self.write_level(false);
        Ok(())
    }

    pub fn set_high(&mut self) -> Result<(), BootError> {
        self.configure_output();
        self.write_level(true);
        Ok(())
    }

    fn configure_output(&self) {
        let iodir = self.read32(GIO_IODIR) & !self.pin_mask;
        self.write32(GIO_IODIR, iodir);
    }

    fn write_level(&self, high: bool) {
        let asserted_high = if self.active_low { !high } else { high };
        let mut data = self.read32(GIO_DATA);
        if asserted_high {
            data |= self.pin_mask;
        } else {
            data &= !self.pin_mask;
        }
        self.write32(GIO_DATA, data);
    }

    fn read32(&self, off: usize) -> u32 {
        // SAFETY: MMIO base is either DT-derived or the documented PoC fallback.
        unsafe { core::ptr::read_volatile((self.base + off) as *const u32) }
    }

    fn write32(&self, off: usize, value: u32) {
        // SAFETY: MMIO base is either DT-derived or the documented PoC fallback.
        unsafe { core::ptr::write_volatile((self.base + off) as *mut u32, value) }
    }
}
