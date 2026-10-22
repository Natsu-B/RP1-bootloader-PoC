//! Read-only normal DBI header observer; never changes selector or reset state.
pub const SELECTOR: u64 = 0x0010_8000;
pub const MONITOR2: u64 = SELECTOR + 0x1a4;
pub const DBI: u64 = 0x0010_9000;
const READY: u32 = 0x0003_0000;
const LEVELS: u32 = 0x001f_0000;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    NotReady(u32),
    Selector(u32),
    LevelsChanged(u32, u32),
    DeviceId(u32),
}

pub fn snapshot(mut read: impl FnMut(u64) -> u32) -> Result<[u32; 7], Error> {
    let before = read(MONITOR2);
    if before & READY != READY { return Err(Error::NotReady(before)); }
    let selector = read(SELECTOR);
    if selector != 0 { return Err(Error::Selector(selector)); }
    let mut header = [0; 7];
    for (i, word) in header.iter_mut().enumerate() { *word = read(DBI + i as u64 * 4); }
    let selector = read(SELECTOR);
    if selector != 0 { return Err(Error::Selector(selector)); }
    let after = read(MONITOR2);
    if after & READY != READY { return Err(Error::NotReady(after)); }
    if before & LEVELS != after & LEVELS { return Err(Error::LevelsChanged(before, after)); }
    if header[0] != 0x0001_1de4 { return Err(Error::DeviceId(header[0])); }
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    fn run(levels: [u32; 2], selectors: [u32; 2], id: u32) -> (Result<[u32; 7], Error>, Vec<u64>) {
        let mut calls = Vec::new();
        let (mut m, mut s) = (0, 0);
        let result = snapshot(|offset| {
            calls.push(offset);
            match offset {
                MONITOR2 => { let v = levels[m]; m += 1; v },
                SELECTOR => { let v = selectors[s]; s += 1; v },
                DBI => id,
                _ => { assert!(offset > DBI && offset <= DBI + 24 && offset % 4 == 0); offset as u32 },
            }
        });
        (result, calls)
    }

    #[test]
    fn reads_only_seven_header_words_between_reset_and_selector_guards() {
        let (result, calls) = run([READY; 2], [0; 2], 0x0001_1de4);
        assert_eq!(result.unwrap()[6], (DBI + 24) as u32);
        let mut expected = vec![MONITOR2, SELECTOR];
        expected.extend((0..7).map(|i| DBI + i * 4));
        expected.extend([SELECTOR, MONITOR2]);
        assert_eq!(calls, expected);
    }

    #[test]
    fn reset_or_unknown_selector_prevents_any_dbi_read() {
        for level in [0, 1 << 16, 1 << 17, u32::MAX & !(1 << 17)] {
            let (result, calls) = run([level, READY], [0; 2], 0x0001_1de4);
            assert_eq!(result, Err(Error::NotReady(level)));
            assert_eq!(calls, [MONITOR2]);
        }
        for selector in [1, 2, 0x8000_0000, u32::MAX] {
            let (result, calls) = run([READY; 2], [selector, 0], 0x0001_1de4);
            assert_eq!(result, Err(Error::Selector(selector)));
            assert_eq!(calls, [MONITOR2, SELECTOR]);
        }
    }

    #[test]
    fn rejects_changed_reset_selector_levels_or_device() {
        assert_eq!(run([READY, 0], [0; 2], 0x0001_1de4).0, Err(Error::NotReady(0)));
        assert_eq!(run([READY; 2], [0, 1], 0x0001_1de4).0, Err(Error::Selector(1)));
        assert_eq!(run([READY, READY | (1 << 18)], [0; 2], 0x0001_1de4).0,
                   Err(Error::LevelsChanged(READY, READY | (1 << 18))));
        for id in [0, u32::MAX, 0x1de4] {
            assert_eq!(run([READY; 2], [0; 2], id).0, Err(Error::DeviceId(id)));
        }
    }
}
