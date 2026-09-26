//! Runtime R1 subset of HAL tools/check-freertos-runtime.py::validate.
//! External GPIO trace and same-boot Linux IRQ evidence remain separate gates.

pub const LINUX_DTB_SHA256: [u8; 32] = [
    0x46, 0xb7, 0x44, 0x9b, 0x31, 0x22, 0x45, 0x4d, 0xd9, 0xed, 0x05, 0x72, 0x50, 0x3f, 0x45, 0x1f,
    0x34, 0xf7, 0x3a, 0xd0, 0x5d, 0xa2, 0x6e, 0x25, 0xe8, 0x5b, 0xa9, 0x91, 0x87, 0x98, 0xf8, 0x43,
];

/// Same-boot console contract, not a physical frequency measurement. Mirror
/// standard PL011's rounded quotient for the explicitly selected 115200 baud.
pub fn console_clock_matches(firmware_hz: u32, linux_hz: u32, ibrd: u32, fbrd: u32) -> bool {
    firmware_hz != 0 && firmware_hz == linux_hz && (1..=0xffff).contains(&ibrd)
        && fbrd < 64 && u64::from(ibrd) * 64 + u64::from(fbrd)
            == (u64::from(linux_hz) * 4 + 57_600) / 115_200
}

pub fn dtb_envelope_valid(bytes: &[u8], address: usize) -> bool {
    address & 7 == 0 && bytes.len() >= 40 && bytes[..4] == [0xd0, 0x0d, 0xfe, 0xed]
        && u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize == bytes.len()
}

pub struct R1Admission {
    next: u32,
    valid: bool,
    first: [u32; 256],
    previous: [u32; 256],
}

impl R1Admission {
    pub const fn new() -> Self {
        Self { next: 0, valid: true, first: [0; 256], previous: [0; 256] }
    }

    pub fn sample(&mut self, number: u32, w: &[u32; 256]) -> bool {
        self.valid &= number == self.next && number < 31
            && w[0] == 0x3130_5452 && w[1] == 1 && w[2] != u32::MAX
            && w[3] == 0 && w[4] == 0 && w[192] != 0x3154_4652;
        if number >= 3 {
            self.valid &= w[2] == 5 && w[19..21] == [0x1357_9bdf, 0]
                && w[28] == 0 && w[29] & 3 == 2 && w[30] & 7 == 0
                && (0x2000_e000..=0x2000_f000).contains(&w[31])
                && w[21] & 0x700 == 0 && u64::from(w[22]) + 1 == u64::from(w[5] / 1000)
                && w[39] == 1 && w[17] > 0 && w[50] > 0
                && (1..4096).contains(&w[18]) && w[32..39].iter().all(|x| *x > 0)
                && w[70] == 0 && w[86] == 0 && w[65] == 0 && w[81] == 0
                && w[66] & 3 == 2 && w[82] & 3 == 2
                && w[67] != w[83] && w[67] & 7 == 0 && w[83] & 7 == 0
                && (0x2000_0000..0x2000_e000).contains(&w[67])
                && (0x2000_0000..0x2000_e000).contains(&w[83]);
            if number == 3 {
                self.first = *w;
            } else {
                self.valid &= [8, 9, 14, 15, 17, 49, 50, 64, 80].iter().all(|i|
                    (1..0x8000_0000).contains(&w[*i].wrapping_sub(self.previous[*i])));
            }
            self.previous = *w;
        }
        self.next += 1;
        self.valid
    }

    pub fn complete(&self) -> bool {
        let ticks = u64::from(self.previous[8].wrapping_sub(self.first[8]));
        let elapsed = u64::from(self.previous[11].wrapping_sub(self.first[11]));
        self.valid && self.next == 31 && ticks > 0
            && 950 * ticks <= elapsed && elapsed <= 1050 * ticks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(n: u32) -> [u32; 256] {
        let mut w = [0; 256];
        w[0] = 0x3130_5452; w[1] = 1; w[2] = 5; w[5] = 100_000_000;
        w[19] = 0x1357_9bdf; w[22] = 99_999; w[29] = 2; w[31] = 0x2000_e000;
        w[18] = 416; w[32..40].fill(1);
        w[66] = 2; w[82] = 2; w[67] = 0x2000_1000; w[83] = 0x2000_2000;
        for i in [8, 9, 14, 15, 17, 49, 50, 64, 80] { w[i] = (n + 1) * 1000; }
        w[11] = w[8] * 1000;
        w
    }

    fn admitted(records: &[[u32; 256]]) -> bool {
        let mut gate = R1Admission::new();
        for (n, w) in records.iter().enumerate() { gate.sample(n as u32, w); }
        gate.complete()
    }

    #[test]
    fn console_clock_contract() {
        assert!(console_clock_matches(44_236_800, 44_236_800, 24, 0));
        assert!(console_clock_matches(48_000_000, 48_000_000, 26, 3));
        for tuple in [(44_236_800,9_216_000,24,0), (44_236_800,44_236_800,5,0),
                      (0,0,0,0), (44_236_800,44_236_800,24,64),
                      (44_236_800,44_236_800,0x10000,0)] {
            assert!(!console_clock_matches(tuple.0,tuple.1,tuple.2,tuple.3));
        }
    }

    #[test]
    fn dtb_bounds() {
        let mut bytes = [0; 40];
        bytes[..4].copy_from_slice(&[0xd0, 0x0d, 0xfe, 0xed]);
        bytes[4..8].copy_from_slice(&40u32.to_be_bytes());
        assert!(dtb_envelope_valid(&bytes, 0x1000));
        assert!(!dtb_envelope_valid(&bytes, 0x1001));
        assert!(!dtb_envelope_valid(&bytes[..39], 0x1000));
        bytes[7] = 41; assert!(!dtb_envelope_valid(&bytes, 0x1000));
        bytes[7] = 40; bytes[0] = 0; assert!(!dtb_envelope_valid(&bytes, 0x1000));
        assert!(!dtb_envelope_valid(&[], 0x1000));
    }

    #[test]
    fn positive_and_fail_closed() {
        let good: Vec<_> = (0..31).map(record).collect();
        assert!(admitted(&good));
        assert!(!admitted(&good[..30]));
        for (word, value) in [(0,0), (1,0), (2,4), (3,1), (4,1), (192,0x3154_4652),
            (19,0), (20,1), (28,1), (29,0), (30,1), (31,0), (21,0x700), (22,0),
            (39,0), (17,0), (50,0), (18,4096), (32,0), (70,1), (86,1), (65,1),
            (81,1), (66,0), (82,0), (67,0), (83,0x2000_1000)] {
            let mut bad = good.clone(); bad[12][word] = value; assert!(!admitted(&bad), "word={word}");
        }
        for word in [8, 9, 14, 15, 17, 49, 50, 64, 80] {
            let mut bad = good.clone(); bad[12][word] = bad[11][word]; assert!(!admitted(&bad));
        }
        for mean in [949, 1051] {
            let mut bad = good.clone(); bad[30][11] = bad[3][11] + 27_000 * mean;
            assert!(!admitted(&bad));
        }
        let mut gate = R1Admission::new(); assert!(!gate.sample(1, &record(1)));
    }

    #[test]
    fn retained_formal_records() {
        // Optional real records, never a replacement for unchanged host validators.
        if let Ok(paths) = std::env::var("SCMI_R1_LOGS") {
            for path in paths.split(':') {
                let text = std::fs::read_to_string(path).unwrap();
                let mut records = vec![[0; 256]; 31];
                let mut rows = 0;
                for line in text.lines() {
                    let Some((_, row)) = line.split_once("[RTOS] ") else { continue; };
                    let parts: Vec<_> = row.split_whitespace().collect();
                    if parts.len() != 6 { continue; }
                    let n: usize = parts[0].parse().unwrap();
                    let offset: usize = parts[1].parse().unwrap();
                    for i in 0..4 { records[n][offset+i] = u32::from_str_radix(parts[2+i], 16).unwrap(); }
                    rows += 1;
                }
                assert_eq!(rows, 31 * 64); assert!(admitted(&records), "{path}");
            }
        }
    }
}
