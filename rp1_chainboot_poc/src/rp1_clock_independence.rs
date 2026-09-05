use core::arch::asm;

use arch_hal::soc::bcm2712::Rp1Config;

const EXPECTED_BAR2_BASE: u64 = 0x1f00_4000_00;
const EXPECTED_BAR2_SIZE: u64 = 0x1_0000;
const RECORD_OFFSET: usize = 0xfd80;
const RECORD_WORDS: usize = 160;
const MAGIC: u32 = 0x5052_3143;
const VERSION: u32 = 1;
const PHASE_WAIT_GO: u32 = 2;
const PHASE_DONE: u32 = 5;
const COMPLETION_DONE: u32 = 0x454e_4f44;
const EXPECTED_FLAGS: u32 = 0x0000_ffff;
const CHECKSUM_SEED: u32 = 0x811c_9dc5;
const CHECKSUM_MUL: u32 = 0x9e37_79b1;
const NO_BAR_ACCESS_MS: u64 = 3_500;

const W_MAGIC: usize = 0;
const W_VERSION: usize = 1;
const W_SIZE: usize = 2;
const W_SEQUENCE: usize = 3;
const W_ACK: usize = 4;
const W_GO: usize = 5;
const W_PHASE: usize = 6;
const W_COMPLETION: usize = 7;
const W_FLAGS: usize = 8;
const W_ERROR: usize = 9;
const W_START_US: usize = 10;
const W_END_US: usize = 12;
const W_ELAPSED_US: usize = 14;
const W_PLL_SYS_PRIM_BEFORE: usize = 16;
const W_PLL_SYS_PRIM_AFTER: usize = 17;
const W_UART_BYTES: usize = 18;
const W_UART_TICKS: usize = 19;
const W_SPI_BYTES: usize = 20;
const W_PWM_LOW_STATUS: usize = 21;
const W_PWM_HIGH_STATUS: usize = 22;
const W_SPI_STATUS: usize = 23;
const W_ENTRY_SNAPSHOTS: usize = 24;
const W_COMPLETION_SNAPSHOTS: usize = 72;
const W_PWM_LOW_SNAPSHOT: usize = 120;
const W_PWM_HIGH_SNAPSHOT: usize = 132;
const W_SPI_SNAPSHOT: usize = 144;
const W_UART_CLOCK: usize = 154;
const W_TARGET_US: usize = 158;
const W_CHECKSUM: usize = 159;

const CLOCKS: [(u32, &str); 12] = [
    (0, "PLL_SYS_CORE"),
    (1, "PLL_AUDIO_CORE"),
    (2, "PLL_VIDEO_CORE"),
    (12, "CLK_SYS"),
    (13, "CLK_SLOW_SYS"),
    (14, "CLK_DMA"),
    (15, "CLK_UART"),
    (16, "CLK_ETH"),
    (17, "CLK_PWM0"),
    (18, "CLK_PWM1"),
    (21, "CLK_I2S"),
    (24, "CLK_PCIE_AUX"),
];

const _: () = assert!(RECORD_OFFSET + RECORD_WORDS * 4 == EXPECTED_BAR2_SIZE as usize);
const _: () = assert!(W_COMPLETION_SNAPSHOTS - W_ENTRY_SNAPSHOTS == CLOCKS.len() * 4);
const _: () = assert!(W_CHECKSUM + 1 == RECORD_WORDS);
const _: () = assert!(checksum_self_check());

pub(crate) fn run_after_full_init(rp1: &Rp1Config) {
    crate::logln!("RP1CLOCK feature=active protocol=fixed-tail-v1");
    let Some((base, size)) = rp1.shared_sram_addr else {
        crate::logln!("RP1CLOCK result=fail reason=bar2-missing");
        return;
    };
    if base != EXPECTED_BAR2_BASE || size != EXPECTED_BAR2_SIZE {
        crate::logln!(
            "RP1CLOCK result=fail reason=bar2-precondition base=0x{:x} size=0x{:x}",
            base,
            size
        );
        return;
    }
    let bar2 = base as usize;
    let Some(sequence) = wait_for_ack(bar2) else {
        crate::logln!("RP1CLOCK result=fail reason=ack-timeout");
        return;
    };

    crate::logln!(
        "RP1CLOCK handshake sequence={} ack={} action=go",
        sequence,
        sequence
    );
    unsafe { write_word(bar2, W_GO, sequence) };
    crate::logln!(
        "RP1CLOCK autonomous sequence={} no_bar_access_ms={}",
        sequence,
        NO_BAR_ACCESS_MS
    );
    crate::timer::delay_millis(NO_BAR_ACCESS_MS);

    // The complete result is fetched once after the autonomous no-BAR window.
    let words = unsafe { read_record_once(bar2) };
    log_and_validate(&words, sequence);
}

fn wait_for_ack(bar2: usize) -> Option<u32> {
    for _ in 0..4_000 {
        let magic = unsafe { read_word(bar2, W_MAGIC) };
        let version = unsafe { read_word(bar2, W_VERSION) };
        let size = unsafe { read_word(bar2, W_SIZE) };
        let sequence = unsafe { read_word(bar2, W_SEQUENCE) };
        let ack = unsafe { read_word(bar2, W_ACK) };
        let go = unsafe { read_word(bar2, W_GO) };
        let phase = unsafe { read_word(bar2, W_PHASE) };
        if magic == MAGIC
            && version == VERSION
            && size == RECORD_WORDS as u32
            && sequence != 0
            && ack == sequence
            && go == 0
            && phase == PHASE_WAIT_GO
        {
            return Some(sequence);
        }
        crate::timer::delay_micros(500);
    }
    None
}

fn log_and_validate(words: &[u32; RECORD_WORDS], sequence: u32) {
    let checksum_expected = checksum(words);
    let start = u64_at(words, W_START_US);
    let end = u64_at(words, W_END_US);
    let elapsed = u64_at(words, W_ELAPSED_US);
    let target = u64::from(words[W_TARGET_US]);

    crate::logln!(
        "RP1CLOCK header magic=0x{:08x} version={} size={} sequence={} ack={} go={} phase={} completion=0x{:08x} flags=0x{:08x} error={} checksum=0x{:08x} checksum_expected=0x{:08x}",
        words[W_MAGIC],
        words[W_VERSION],
        words[W_SIZE],
        words[W_SEQUENCE],
        words[W_ACK],
        words[W_GO],
        words[W_PHASE],
        words[W_COMPLETION],
        words[W_FLAGS],
        words[W_ERROR],
        words[W_CHECKSUM],
        checksum_expected
    );
    crate::logln!(
        "RP1CLOCK timing start={} end={} elapsed={} target={}",
        start,
        end,
        elapsed,
        target
    );
    crate::logln!(
        "RP1CLOCK io uart_bytes={} uart_ticks={} spi_bytes={} pwm_low_status={} pwm_high_status={} spi_status={}",
        words[W_UART_BYTES],
        words[W_UART_TICKS],
        words[W_SPI_BYTES],
        words[W_PWM_LOW_STATUS],
        words[W_PWM_HIGH_STATUS],
        words[W_SPI_STATUS]
    );
    crate::logln!(
        "RP1CLOCK pll_sys prim_before=0x{:08x} prim_after=0x{:08x}",
        words[W_PLL_SYS_PRIM_BEFORE],
        words[W_PLL_SYS_PRIM_AFTER]
    );
    crate::logln!(
        "RP1CLOCK uart_clock ctrl=0x{:08x} div_int=0x{:08x} div_frac=0x{:08x} sel=0x{:08x}",
        words[W_UART_CLOCK],
        words[W_UART_CLOCK + 1],
        words[W_UART_CLOCK + 2],
        words[W_UART_CLOCK + 3]
    );
    log_snapshots("entry", words, W_ENTRY_SNAPSHOTS);
    log_snapshots("completion", words, W_COMPLETION_SNAPSHOTS);
    log_io_snapshot("pwm_low", words, W_PWM_LOW_SNAPSHOT, 12);
    log_io_snapshot("pwm_high", words, W_PWM_HIGH_SNAPSHOT, 12);
    log_io_snapshot("spi", words, W_SPI_SNAPSHOT, 10);

    let mut valid = 0u32;
    if words[W_MAGIC] == MAGIC
        && words[W_VERSION] == VERSION
        && words[W_SIZE] == RECORD_WORDS as u32
    {
        valid |= 1 << 0;
    }
    if words[W_SEQUENCE] == sequence && words[W_ACK] == sequence && words[W_GO] == sequence {
        valid |= 1 << 1;
    }
    if words[W_PHASE] == PHASE_DONE && words[W_COMPLETION] == COMPLETION_DONE {
        valid |= 1 << 2;
    }
    if words[W_ERROR] == 0 {
        valid |= 1 << 3;
    }
    if words[W_FLAGS] == EXPECTED_FLAGS {
        valid |= 1 << 4;
    }
    if target == 2_200_000
        && start != 0
        && end >= start
        && elapsed == end - start
        && elapsed >= target
    {
        valid |= 1 << 5;
    }
    if clock_snapshots_valid(words) {
        valid |= 1 << 6;
    }
    if words[W_UART_BYTES] == 35 && words[W_UART_TICKS] == 10 {
        valid |= 1 << 7;
    }
    if pwm_snapshots_valid(words) {
        valid |= 1 << 8;
    }
    if spi_snapshot_valid(words) {
        valid |= 1 << 9;
    }
    if words[W_CHECKSUM] == checksum_expected {
        valid |= 1 << 10;
    }
    const EXPECTED_VALID: u32 = (1 << 11) - 1;
    crate::logln!(
        "RP1CLOCK validation mask=0x{:08x} expected=0x{:08x} flags_expected=0x{:08x}",
        valid,
        EXPECTED_VALID,
        EXPECTED_FLAGS
    );
    crate::logln!(
        "RP1CLOCK result={}",
        if valid == EXPECTED_VALID {
            "pass"
        } else {
            "fail"
        }
    );
}

fn log_snapshots(stage: &str, words: &[u32; RECORD_WORDS], base: usize) {
    for (index, &(id, name)) in CLOCKS.iter().enumerate() {
        let at = base + index * 4;
        crate::logln!(
            "RP1CLOCK snapshot stage={} index={} id={} name={} values=0x{:08x}/0x{:08x}/0x{:08x}/0x{:08x}",
            stage,
            index,
            id,
            name,
            words[at],
            words[at + 1],
            words[at + 2],
            words[at + 3]
        );
    }
}

fn log_io_snapshot(name: &str, words: &[u32; RECORD_WORDS], base: usize, len: usize) {
    crate::logln!(
        "RP1CLOCK io_snapshot name={} base={} words={}",
        name,
        base,
        len
    );
    for index in 0..len {
        crate::logln!(
            "RP1CLOCK io_snapshot name={} index={} value=0x{:08x}",
            name,
            index,
            words[base + index]
        );
    }
}

fn clock_snapshots_valid(words: &[u32; RECORD_WORDS]) -> bool {
    let mut absent_ok = true;
    for clock in [3usize, 4, 5, 6, 7, 10] {
        absent_ok &= words[W_ENTRY_SNAPSHOTS + clock * 4 + 2] == u32::MAX;
        absent_ok &= words[W_COMPLETION_SNAPSHOTS + clock * 4 + 2] == u32::MAX;
    }
    let pll = W_COMPLETION_SNAPSHOTS;
    let uart = W_COMPLETION_SNAPSHOTS + 6 * 4;
    absent_ok
        && words[pll..pll + 4] == [0x8000_0001, 0x0000_0004, 20, 0]
        && words[W_PLL_SYS_PRIM_AFTER] == 0x0007_7010
        && words[uart] == 0x1000_0840
        && words[uart + 1] == 1
        && words[uart + 3] & 1 != 0
        && words[W_UART_CLOCK] == 0x1000_0840
        && words[W_UART_CLOCK + 1] == 1
        && words[W_UART_CLOCK + 2] == u32::MAX
        && words[W_UART_CLOCK + 3] & 1 != 0
}

fn pwm_snapshots_valid(words: &[u32; RECORD_WORDS]) -> bool {
    let low = W_PWM_LOW_SNAPSHOT;
    let high = W_PWM_HIGH_SNAPSHOT;
    words[W_PWM_LOW_STATUS] == 1
        && words[W_PWM_HIGH_STATUS] == 1
        && words[low] == 0x1100_0840
        && words[low + 1] == 1
        && words[low + 2] == 0
        && words[low + 3] & 1 != 0
        && words[low + 10] == 5_000_000
        && words[low + 11] == 1_250_000
        && words[high] == 0x1100_0840
        && words[high + 1] == 1
        && words[high + 2] == 0
        && words[high + 3] & 1 != 0
        && words[high + 10] == 50_000
        && words[high + 11] == 37_500
}

fn spi_snapshot_valid(words: &[u32; RECORD_WORDS]) -> bool {
    let spi = W_SPI_SNAPSHOT;
    words[W_SPI_BYTES] == 20
        && words[W_SPI_STATUS] == 1
        && words[spi] != 0
        && words[spi + 1] == 0x0007_0100
        && words[spi + 2] == 1
        && words[spi + 3] == 2_000
        && words[spi + 9] & 0xffff == 20
        && words[spi + 9] >> 16 >= 20
}

const fn u64_at(words: &[u32; RECORD_WORDS], index: usize) -> u64 {
    words[index] as u64 | ((words[index + 1] as u64) << 32)
}

const fn checksum(words: &[u32; RECORD_WORDS]) -> u32 {
    let mut value = CHECKSUM_SEED;
    let mut index = 0;
    while index < W_CHECKSUM {
        if index != W_COMPLETION {
            value = (value ^ words[index])
                .rotate_left(5)
                .wrapping_mul(CHECKSUM_MUL);
        }
        index += 1;
    }
    value
}

const fn checksum_self_check() -> bool {
    let mut words = [0u32; RECORD_WORDS];
    if checksum(&words) != 0x665f_f975 {
        return false;
    }
    words[W_COMPLETION] = COMPLETION_DONE;
    if checksum(&words) != 0x665f_f975 {
        return false;
    }
    words[42] = 1;
    checksum(&words) == 0x9822_fead
}

unsafe fn read_record_once(bar2: usize) -> [u32; RECORD_WORDS] {
    let mut words = [0u32; RECORD_WORDS];
    for (index, word) in words.iter_mut().enumerate() {
        *word = unsafe { read_word(bar2, index) };
    }
    words
}

unsafe fn read_word(bar2: usize, index: usize) -> u32 {
    unsafe { core::ptr::read_volatile((bar2 + RECORD_OFFSET + index * 4) as *const u32) }
}

unsafe fn write_word(bar2: usize, index: usize, value: u32) {
    unsafe {
        core::ptr::write_volatile((bar2 + RECORD_OFFSET + index * 4) as *mut u32, value);
        asm!("dsb sy", options(nostack, preserves_flags));
    }
}
