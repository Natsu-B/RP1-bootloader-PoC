//! Opt-in, read-only offset observations, not an edge/scan or clock-drift proof.

const MAX_ATTEMPTS: u32 = 4;

#[derive(Clone, Copy, Debug, Default)]
struct RawRead {
    high_before: u32,
    low: u32,
    high_after: u32,
    attempts: u32,
}

impl RawRead {
    fn raw64(self) -> u64 {
        u64::from(self.high_before) << 32 | u64::from(self.low)
    }
}

// Offsets from the checked high-word address. Never spin through a bad link.
fn read_raw(mut read: impl FnMut(usize) -> u32) -> RawRead {
    let mut raw = RawRead::default();
    for attempt in 1..=MAX_ATTEMPTS {
        raw = RawRead {
            high_before: read(0),
            low: read(4),
            high_after: read(0),
            attempts: attempt,
        };
        if raw.high_before == raw.high_after {
            break;
        }
    }
    raw
}

#[derive(Clone, Copy, Debug, Default)]
struct Sample {
    host_before: u64,
    host_after: u64,
    cntfrq: u64,
    cntvoff_before: u64,
    cntvoff_after: u64,
    raw: RawRead,
    map_error: Option<&'static str>,
}

impl Sample {
    fn error(&self, previous: Option<&Self>) -> &'static str {
        if let Some(error) = self.map_error {
            return error;
        }
        if self.raw.high_before != self.raw.high_after {
            return "rollover-exhausted";
        }
        if self.raw.raw64() == u64::MAX {
            return "all-ones";
        }
        if self.host_after < self.host_before {
            return "host-order";
        }
        if self.cntfrq == 0 {
            return "cntfrq-zero";
        }
        if self.cntvoff_before != self.cntvoff_after {
            return "cntvoff-change";
        }
        if let Some(prev) = previous {
            if self.host_before < prev.host_after {
                return "host-order";
            }
            if self.cntfrq != prev.cntfrq {
                return "cntfrq-change";
            }
            if self.cntvoff_before != prev.cntvoff_after {
                return "cntvoff-change";
            }
            if prev.error(None) == "none" && self.raw.raw64() < prev.raw.raw64() {
                return "raw-backwards";
            }
        }
        "none"
    }
}

#[cfg(not(test))]
pub fn log(rp1: &arch_hal::soc::bcm2712::Rp1Config) {
    use arch_hal::cpu::{dsb_sy, isb};
    use arch_hal::soc::bcm2712::rp1::Rp1PeripheralMap;
    use arch_timer::systimer::{read_counter, read_counter_frequency};

    const SAMPLES: usize = 16;
    // RP1 local high/low=0x400ac024/0x400ac028, accessed only via admitted BAR1.
    let timer = Rp1PeripheralMap::from_config(rp1)
        .and_then(|map| map.mmio_base(0x000a_c024, 8))
        .map_err(|_| "bar1-range")
        .and_then(|base| {
            if base & 3 != 0 || base.checked_add(8).is_none() {
                Err("bar1-alignment-overflow")
            } else {
                Ok(base)
            }
        });
    let samples: [Sample; SAMPLES] = core::array::from_fn(|_| {
        let cntfrq = read_counter_frequency();
        let cntvoff_before = cntvoff();
        // DSB completes preceding accesses; ISB after CNTPCT prevents subsequent
        // device loads from being issued before the host-before counter read.
        dsb_sy();
        let host_before = read_counter();
        isb();
        let raw = match timer {
            Ok(base) => read_raw(|offset| {
                // SAFETY: checked, aligned BAR1 range of the already admitted
                // RP1. DSB completes each H/L/H read in order. No MMIO writes.
                let word = unsafe { core::ptr::read_volatile((base + offset) as *const u32) };
                dsb_sy();
                word
            }),
            Err(_) => RawRead::default(),
        };
        // ISB alone cannot complete a Device read before the counter sample.
        dsb_sy();
        let host_after = read_counter();
        Sample {
            host_before, host_after, cntfrq, cntvoff_before,
            cntvoff_after: cntvoff(), raw, map_error: timer.err(),
        }
    });

    // All samples are captured before the first UART/trace formatting call.
    crate::logln!("[RP1ANCHOR] version=1 counter=CNTPCT_EL0 samples={} raw_tick_hz=1000000 max_attempts={} readonly=1", SAMPLES, MAX_ATTEMPTS);
    for (i, sample) in samples.iter().enumerate() {
        let error = sample.error(i.checked_sub(1).map(|n| &samples[n]));
        crate::logln!("[RP1ANCHOR] sample={} valid={} error={} host_before={} host_after={} cntfrq={} cntvoff_before=0x{:016x} cntvoff_after=0x{:016x} raw64=0x{:016x} high_before=0x{:08x} low=0x{:08x} high_after=0x{:08x} attempts={}",
            i, u8::from(error == "none"), error, sample.host_before, sample.host_after,
            sample.cntfrq, sample.cntvoff_before, sample.cntvoff_after, sample.raw.raw64(),
            sample.raw.high_before, sample.raw.low, sample.raw.high_after, sample.raw.attempts);
    }
}

#[cfg(not(test))]
fn cntvoff() -> u64 {
    let value;
    // SAFETY: read-only EL2 metadata. Linux handoff later clears this offset;
    // the anchor uses physical CNTPCT, never a pre-handoff virtual timestamp.
    unsafe {
        core::arch::asm!("mrs {value}, CNTVOFF_EL2", value = out(reg) value,
            options(nostack, preserves_flags));
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_rollover_and_validation() {
        let words = [1, u32::MAX, 2, 2, 3, 2];
        let mut reads = 0;
        let raw = read_raw(|offset| {
            assert_eq!(offset, [0, 4, 0][reads % 3]);
            let word = words[reads]; reads += 1; word
        });
        assert_eq!(raw.raw64(), 0x0000_0002_0000_0003);
        assert_eq!((raw.attempts, reads), (2, 6));
        let good = Sample { host_before: 10, host_after: 20, cntfrq: 54_000_000,
            cntvoff_before: 7, cntvoff_after: 7, raw, map_error: None };
        assert_eq!(good.error(None), "none");
        let next = Sample { host_before: 21, host_after: 30, ..good };
        assert_eq!(next.error(Some(&good)), "none");

        reads = 0;
        let torn = read_raw(|_| { reads += 1; reads as u32 });
        assert_eq!(reads, 3 * MAX_ATTEMPTS as usize);
        assert_eq!(torn.attempts, MAX_ATTEMPTS);
        for (bad, expected) in [
            (Sample { raw: torn, ..good }, "rollover-exhausted"),
            (Sample { raw: read_raw(|_| u32::MAX), ..good }, "all-ones"),
            (Sample { host_after: 9, ..good }, "host-order"),
            (Sample { cntfrq: 0, ..good }, "cntfrq-zero"),
            (Sample { cntvoff_after: 8, ..good }, "cntvoff-change"),
            (Sample { raw: RawRead::default(), map_error: Some("bar1-range"), ..good }, "bar1-range"),
            (Sample { map_error: Some("bar1-alignment-overflow"), ..good }, "bar1-alignment-overflow"),
        ] { assert_eq!(bad.error(None), expected); }
        for (bad, expected) in [
            (Sample { host_before: 19, ..next }, "host-order"),
            (Sample { cntfrq: 1, ..next }, "cntfrq-change"),
            (Sample { cntvoff_before: 8, cntvoff_after: 8, ..next }, "cntvoff-change"),
            (Sample { raw: RawRead { low: 2, ..raw }, ..next }, "raw-backwards"),
        ] { assert_eq!(bad.error(Some(&good)), expected); }
        // A low32 wrap with a stable incremented high word is valid; a full
        // reset/wrap backwards is rejected instead of guessing an epoch.
        let before = Sample { raw: RawRead { high_before: 1, high_after: 1,
            low: u32::MAX, attempts: 1 }, ..good };
        assert_eq!(next.error(Some(&before)), "none");
        assert_eq!(Sample { raw: RawRead::default(), ..next }.error(Some(&good)), "raw-backwards");
    }
}
