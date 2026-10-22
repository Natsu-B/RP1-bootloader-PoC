use core::fmt;

use sha2::{Digest, Sha256};

pub fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

pub struct Sha256Hex<'a>(pub &'a [u8; 32]);

impl fmt::Display for Sha256Hex<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

#[allow(dead_code)]
pub fn log_sha256(label: &str, digest: &[u8; 32]) {
    log_sha256_len(label, digest, 0);
}

pub fn log_sha256_len(label: &str, digest: &[u8; 32], len: usize) {
    slow_puts("[RP1HASH] ");
    slow_puts(label);
    slow_puts(" sha256=");
    let mut hex = [0u8; 64];
    hex_digest(digest, &mut hex);
    for chunk in hex.chunks(8) {
        // SAFETY: hex_digest writes only ASCII lowercase hex digits.
        slow_puts(unsafe { core::str::from_utf8_unchecked(chunk) });
    }
    slow_puts(" len=");
    let mut dec = [0u8; 20];
    let dec = decimal_usize(len, &mut dec);
    slow_puts(dec);
    slow_puts("\n");
}

fn slow_puts(s: &str) {
    crate::logging::puts(s);
    crate::timer::delay_micros(500);
}

fn hex_digest(digest: &[u8; 32], out: &mut [u8; 64]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for (index, byte) in digest.iter().copied().enumerate() {
        out[index * 2] = HEX[(byte >> 4) as usize];
        out[index * 2 + 1] = HEX[(byte & 0x0f) as usize];
    }
}

fn decimal_usize(mut value: usize, out: &mut [u8; 20]) -> &str {
    let mut pos = out.len();
    if value == 0 {
        pos -= 1;
        out[pos] = b'0';
    } else {
        while value != 0 {
            pos -= 1;
            out[pos] = b'0' + (value % 10) as u8;
            value /= 10;
        }
    }
    // SAFETY: decimal conversion writes only ASCII digits.
    unsafe { core::str::from_utf8_unchecked(&out[pos..]) }
}
