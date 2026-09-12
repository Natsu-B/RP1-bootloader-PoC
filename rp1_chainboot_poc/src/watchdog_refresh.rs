//! One fixed WQL1 request. Diagnostic nonce is not randomness/authentication.
pub const fn request(raw_timer_word: u32) -> [u32; 8] {
    let folded = (raw_timer_word ^ (raw_timer_word >> 16)) & 0xffff;
    let nonce = if folded == 0 { 1 } else { folded };
    [u32::from_le_bytes(*b"WQL1"), 10, 1, 1, 0x00ff_ffff, 256, nonce,
     0x5744_5432 ^ 10 ^ 1 ^ 1 ^ 0x00ff_ffff ^ 256 ^ nonce]
}

#[cfg(test)]
mod tests {
    #[test]
    fn fixed_request_nonce_bounds_and_checksum() {
        for low in 0..=65535 {
            for high in [0, 1, 65535] {
                let timer = (high << 16) | low;
                let q = super::request(timer);
                assert_eq!(&q[..6], &[u32::from_le_bytes(*b"WQL1"),10,1,1,0x00ff_ffff,256]);
                assert!((1..=65535).contains(&q[6]));
                assert_eq!(q[7], 0x5744_5432 ^ q[1] ^ q[2] ^ q[3] ^ q[4] ^ q[5] ^ q[6]);
            }
        }
        assert_eq!(super::request(0)[6],1);
        assert_eq!(super::request(0xffff)[6],65535);
    }
}
