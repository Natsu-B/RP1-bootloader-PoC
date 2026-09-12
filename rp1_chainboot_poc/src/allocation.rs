//! Bump allocation must align the absolute address, not the heap-relative offset.
pub(crate) fn aligned_range(base: usize, current: usize, capacity: usize,
                           size: usize, align: usize) -> Option<(usize, usize)> {
    if !align.is_power_of_two() || current > capacity { return None; }
    let address = base.checked_add(current)?.checked_add(align - 1)? & !(align - 1);
    let offset = address.checked_sub(base)?;
    let next = offset.checked_add(size)?;
    if next > capacity || base.checked_add(next).is_none() { return None; }
    Some((offset, next))
}

#[cfg(test)]
mod tests {
    use super::aligned_range;

    #[test]
    fn absolute_alignment_and_bounds() {
        for base in 0xb4a00..0xb4a40 {
            for current in [0, 1, 7, 64, 1023] {
                for power in 0..=20 {
                    let align = 1 << power;
                    let (offset, next) = aligned_range(base, current, 1 << 22, 37, align).unwrap();
                    assert_eq!((base + offset) % align, 0);
                    assert!(offset >= current && offset - current < align);
                    assert_eq!(next, offset + 37);
                }
            }
        }
        assert_eq!(aligned_range(0xb4a08, 0, 64, 16, 16), Some((8, 24)));
        assert_eq!(aligned_range(8, 0, 24, 16, 16), Some((8, 24)));
        for args in [(8, 0, 23, 16, 16), (0, 65, 64, 1, 1),
                     (usize::MAX, 0, 64, 1, 16), (usize::MAX, 1, 64, 1, 1),
                     (0, 1, usize::MAX, usize::MAX, 2), (1, 0, usize::MAX, usize::MAX, 1),
                     (0, 0, 64, 1, 0), (0, 0, 64, 1, 3)] {
            assert_eq!(aligned_range(args.0, args.1, args.2, args.3, args.4), None);
        }
    }
}
