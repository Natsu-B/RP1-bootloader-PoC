use core::arch::{asm, global_asm};

use arch_hal::soc::bcm2712::Rp1Config;
use rp1_abi::debug;

const EXPECTED_BAR1_BASE: usize = 0x1f00_0000_00;
const EXPECTED_BAR2_BASE: usize = 0x1f00_4000_00;
const EXPECTED_BAR1_SIZE: u64 = 0x40_0000;
const EXPECTED_BAR2_SIZE: u64 = 0x1_0000;
const EXPECTED_CHIP_ID: u32 = 0x2000_1927;
const CHIP_ID_OFFSET: usize = 0;
const RECORD_OFFSET: usize = (debug::MAILBOX_ADDR - 0x2000_0000) as usize;
const RECORD_WORDS: usize = 256;
const RECORD_BYTES: usize = RECORD_WORDS * 4;
#[cfg(not(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
)))]
const CHECKSUM_WORDS: usize = 142;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const CHECKSUM_WORDS: usize = 241;
const RECORD_MAGIC: u32 = 0x4d42_4e49;
#[cfg(not(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
)))]
const RECORD_VERSION: u32 = 2;
#[cfg(all(
    feature = "rp1-bar1-4k-protection-proof",
    not(feature = "rp1-iatu-second-spare-programming-proof"),
    not(feature = "rp1-bar1-interior-64k-hole-proof")
))]
const RECORD_VERSION: u32 = 3;
#[cfg(all(
    feature = "rp1-iatu-second-spare-programming-proof",
    not(feature = "rp1-iatu-64k-address-mask-characterization"),
    not(feature = "rp1-bar1-interior-64k-hole-proof")
))]
const RECORD_VERSION: u32 = 4;
#[cfg(all(
    feature = "rp1-iatu-64k-address-mask-characterization",
    not(feature = "rp1-bar1-interior-64k-hole-proof")
))]
const RECORD_VERSION: u32 = 5;
#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
const RECORD_VERSION: u32 = 6;
const CHECKSUM_SEED: u32 = 0x811c_9dc5;
const TRAFFIC_OPS: usize = 8192;
const ACK_TIMEOUT_US: u64 = 1_100_000;
const MODE_TIMEOUT_US: u64 = 300_000;
const SAMPLE_QUIET_US: u64 = 110_000;

const MODE_IDLE: u32 = 1;
const MODE_BAR2_READ: u32 = 2;
const MODE_BAR2_WRITE: u32 = 3;
const MODE_BAR1_READ: u32 = 4;
const MODE_BLOCK_BAR1: u32 = 5;
const MODE_DONE: u32 = 6;
#[cfg(feature = "rp1-bar1-4k-protection-proof")]
const MODE_REDIRECT_4K: u32 = 7;
#[cfg(feature = "rp1-bar1-4k-protection-proof")]
const MODE_HOLE_4K: u32 = 8;
#[cfg(feature = "rp1-bar1-64k-hole-proof")]
const MODE_HOLE_64K: u32 = 9;
#[cfg(feature = "rp1-iatu-second-spare-programming-proof")]
const MODE_PROGRAM_SECOND_SPARE: u32 = 10;
#[cfg(feature = "rp1-iatu-64k-address-mask-characterization")]
const MODE_CHARACTERIZE_ADDRESS_MASK: u32 = 11;
#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
const MODE_INTERIOR_HOLE_64K: u32 = 12;
const COMPLETION_IDLE: u32 = 0x4944_4c45;
const COMPLETION_DONE: u32 = 0x444f_4e45;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const COMPLETION_PRECONDITION: u32 = 0x5052_4543;
const PHASE_DONE: u32 = 5;
#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
const BLOCK_PHASE_IDLE: u32 = 0;
const BLOCK_PHASE_DISABLED: u32 = 3;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const BLOCK_PHASE_PRECONDITION_OK: u32 = 1;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const BLOCK_PHASE_PRECONDITION_FAIL: u32 = 2;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const BLOCK_PHASE_RESTORED: u32 = 4;
#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
const BLOCK_PHASE_RESTORING: u32 = 5;
#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
const BLOCK_PHASE_REJECTED: u32 = 6;
const FLAG_CTRL2_PRECONDITION_OK: u32 = 1 << 0;
const FLAG_CTRL2_WRITTEN: u32 = 1 << 1;
const FLAG_CTRL2_BLOCK_READBACK_OK: u32 = 1 << 2;
const FLAG_CTRL2_RESTORED: u32 = 1 << 3;
const FLAG_SELECTOR_RESTORED: u32 = 1 << 4;
const FLAG_SCRATCH_RESTORED: u32 = 1 << 5;
const HEALTH_PCIE_MONITOR_CAPTURED: u32 = 1 << 0;
const HEALTH_AXISHIM_CFG_UNCHANGED: u32 = 1 << 1;
const HEALTH_SAMPLED: u32 = 1 << 2;
const HEALTH_NO_OVERFLOW: u32 = 1 << 3;
const HEALTH_SCRATCH_RESTORED: u32 = 1 << 4;

const W_MAGIC: usize = 0;
const W_VERSION: usize = 1;
const W_RECORD_SIZE: usize = 2;
const W_SEQ: usize = 3;
const W_ACK: usize = 4;
const W_GO: usize = 5;
const W_MODE: usize = 6;
const W_PHASE: usize = 7;
const W_COMPLETION: usize = 8;
const W_COMPLETION_SEQ: usize = 9;
const W_FLAGS: usize = 10;
const W_RESULT: usize = 11;
const W_CHECKSUM: usize = 12;
const W_OVERFLOW_COUNT: usize = 13;
const W_STARTED_US_LO: usize = 14;
const W_STARTED_US_HI: usize = 15;
const W_ENDED_US_LO: usize = 16;
const W_ENDED_US_HI: usize = 17;
const W_ELAPSED_US: usize = 18;
const W_SAMPLE_COUNT: usize = 19;
const W_ARG0: usize = 20;
const W_ARG1: usize = 21;
const W_HEALTH_FLAGS: usize = 22;
const W_CONFIG_CHANGE_COUNT: usize = 23;
const W_MONITOR0_OR: usize = 24;
const W_MONITOR0_MAX: usize = 25;
const W_MONITOR1_OR: usize = 26;
const W_MONITOR1_MAX: usize = 27;
const W_MONITOR2_OR: usize = 28;
const W_MONITOR2_MAX: usize = 29;
const W_MONITOR2_BIT23_COUNT: usize = 30;
const W_MONITOR2_BIT22_COUNT: usize = 31;
const W_MONITOR2_BIT21_COUNT: usize = 32;
const W_MONITOR2_BIT23_FIRST_US: usize = 33;
const W_MONITOR2_BIT22_FIRST_US: usize = 34;
const W_MONITOR2_BIT21_FIRST_US: usize = 35;
const W_SCRATCH_RESTORE_OK: usize = 36;
const W_SCRATCH_CHANGE_COUNT: usize = 37;
const W_SCRATCH_LAST_CHANGE_US: usize = 38;
const W_BLOCK_PHASE: usize = 39;
const W_BLOCK_DISABLE_US: usize = 40;
const W_BLOCK_RESTORE_US: usize = 41;
const W_SELECTOR_SAVED: usize = 42;
const W_SELECTOR_RESTORE_READBACK: usize = 43;
const W_CTRL2_BEFORE: usize = 44;
const W_CTRL2_BLOCK_VALUE: usize = 45;
const W_CTRL2_BLOCK_READBACK: usize = 46;
const W_CTRL2_RESTORE_READBACK: usize = 47;
const W_PCIE_CFG_BEFORE: usize = 48;
const W_PCIE_CFG_AFTER: usize = 51;
const W_AXISHIM_CFG_BEFORE: usize = 54;
const W_AXISHIM_CFG_AFTER: usize = 66;
const W_AXISHIM_STATUS: usize = 78;
const AXISHIM_STATUS_WORDS: usize = 4;
const CHANNEL_COUNT: usize = 12;
const W_SCRATCH: usize = 126;
const W_SCRATCH_INITIAL: usize = 130;
const W_SCRATCH_LAST: usize = 134;
const W_SCRATCH_FINAL: usize = 138;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_BAR_ASSIGNMENT: usize = 142;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_BAR0: usize = W_BAR_ASSIGNMENT;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_BAR1: usize = W_BAR_ASSIGNMENT + 1;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_BAR2: usize = W_BAR_ASSIGNMENT + 2;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_BAR1_BUS_BASE: usize = W_BAR_ASSIGNMENT + 3;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_COMMAND: usize = W_BAR_ASSIGNMENT + 4;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_PROTECTION: usize = 147;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_TARGET_PAGE_OFFSET: usize = W_PROTECTION;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_PROTECT_BAR1_BUS_BASE: usize = W_PROTECTION + 1;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_DUMMY_LOCAL_BASE: usize = W_PROTECTION + 2;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_ORIGINAL_BAR1: usize = W_PROTECTION + 3;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const IATU_SNAPSHOT_WORDS: usize = 9;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_SPARE_BEFORE: usize = W_ORIGINAL_BAR1 + IATU_SNAPSHOT_WORDS;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_SPARE_PROGRAMMED: usize = W_SPARE_BEFORE + IATU_SNAPSHOT_WORDS * 2;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_SPARE_READBACK: usize = W_SPARE_PROGRAMMED + IATU_SNAPSHOT_WORDS * 2;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_SPARE_RESTORED: usize = W_SPARE_READBACK + IATU_SNAPSHOT_WORDS * 2;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_PROTECT_ENABLE_US: usize = W_SPARE_RESTORED + IATU_SNAPSHOT_WORDS * 2;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_PROTECT_DISABLE_US: usize = W_PROTECT_ENABLE_US + 1;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_PROTECT_RESTORE_US: usize = W_PROTECT_ENABLE_US + 2;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_TARGET_BEFORE: usize = W_PROTECT_ENABLE_US + 3;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_TARGET_DURING: usize = W_PROTECT_ENABLE_US + 4;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_TARGET_AFTER: usize = W_PROTECT_ENABLE_US + 5;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_CONTROL_BEFORE: usize = W_PROTECT_ENABLE_US + 6;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_CONTROL_DURING: usize = W_PROTECT_ENABLE_US + 7;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_CONTROL_AFTER: usize = W_PROTECT_ENABLE_US + 8;
#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
const W_PROTECT_FLAGS: usize = W_PROTECT_ENABLE_US + 9;

#[cfg(feature = "rp1-bar1-4k-protection-proof")]
const PROTECT_FLAG_DBI_BARS_VALID: u32 = 1 << 0;
#[cfg(feature = "rp1-bar1-4k-protection-proof")]
const PROTECT_FLAG_ORIGINAL_BAR1_VALID: u32 = 1 << 1;
#[cfg(feature = "rp1-bar1-4k-protection-proof")]
const PROTECT_FLAG_SPARES_UNUSED: u32 = 1 << 2;
#[cfg(feature = "rp1-bar1-4k-protection-proof")]
const PROTECT_FLAG_DUMMY_VALID: u32 = 1 << 3;
#[cfg(feature = "rp1-bar1-4k-protection-proof")]
const PROTECT_FLAG_PROGRAM_READBACK: u32 = 1 << 4;
#[cfg(feature = "rp1-bar1-4k-protection-proof")]
const PROTECT_FLAG_ACTIVE: u32 = 1 << 5;
#[cfg(feature = "rp1-bar1-4k-protection-proof")]
const PROTECT_FLAG_SPARE_DISABLED: u32 = 1 << 6;
#[cfg(feature = "rp1-bar1-4k-protection-proof")]
const PROTECT_FLAG_SPARE_RESTORED: u32 = 1 << 7;
#[cfg(feature = "rp1-bar1-4k-protection-proof")]
const PROTECT_FLAG_SELECTOR_RESTORED: u32 = 1 << 8;
#[cfg(feature = "rp1-bar1-4k-protection-proof")]
const PROTECT_FLAG_LOCAL_TARGET_STABLE: u32 = 1 << 9;
#[cfg(feature = "rp1-bar1-4k-protection-proof")]
const PROTECT_FLAG_LOCAL_CONTROL_STABLE: u32 = 1 << 10;
#[cfg(feature = "rp1-bar1-4k-protection-proof")]
const PROTECT_FLAG_ORIGINAL_RESTORED: u32 = 1 << 11;
#[cfg(feature = "rp1-bar1-4k-protection-proof")]
const REQUIRED_PROTECT_FLAGS: u32 = PROTECT_FLAG_DBI_BARS_VALID
    | PROTECT_FLAG_ORIGINAL_BAR1_VALID
    | PROTECT_FLAG_SPARES_UNUSED
    | PROTECT_FLAG_DUMMY_VALID
    | PROTECT_FLAG_PROGRAM_READBACK
    | PROTECT_FLAG_ACTIVE
    | PROTECT_FLAG_SPARE_DISABLED
    | PROTECT_FLAG_SPARE_RESTORED
    | PROTECT_FLAG_SELECTOR_RESTORED
    | PROTECT_FLAG_LOCAL_TARGET_STABLE
    | PROTECT_FLAG_LOCAL_CONTROL_STABLE
    | PROTECT_FLAG_ORIGINAL_RESTORED;
#[cfg(feature = "rp1-bar1-4k-protection-proof")]
const DUMMY_MAGIC: u32 = 0x344b_5052;
#[cfg(feature = "rp1-bar1-4k-protection-proof")]
const DUMMY_CANARY_XOR: u32 = 0xa5a5_5a5a;
#[cfg(feature = "rp1-bar1-4k-protection-proof")]
const CONTROL_OFFSET: usize = 0x0010_81a4;
#[cfg(feature = "rp1-bar1-4k-protection-proof")]
const CONTROL_LINK_MASK: u32 = 0x0019_0000;
#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
const INTERIOR_REQUIRED_FLAGS: u32 = (1 << 15) - 1;
#[cfg(all(
    feature = "rp1-bar1-interior-64k-hole-proof",
    not(feature = "rp1-bar1-second-interior-64k-page-proof")
))]
const INTERIOR_HOLE_OFFSET: u32 = 0x0003_0000;
#[cfg(feature = "rp1-bar1-second-interior-64k-page-proof")]
const INTERIOR_HOLE_OFFSET: u32 = 0x0004_0000;
#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
const INTERIOR_UART0_PID0_OFFSET: usize = 0x0003_0fe0;
#[cfg(feature = "rp1-bar1-hole-write-effect-proof")]
const INTERIOR_UART0_DR_OFFSET: usize = 0x0003_0000;
#[cfg(feature = "rp1-bar1-hole-write-effect-proof")]
const INTERIOR_UART0_FR_OFFSET: usize = 0x0003_0018;
#[cfg(feature = "rp1-bar1-hole-write-effect-proof")]
const UART_FR_BUSY: u32 = 1 << 3;
#[cfg(feature = "rp1-bar1-hole-write-effect-proof")]
const UART_FR_TXFE: u32 = 1 << 7;
#[cfg(feature = "rp1-bar1-hole-write-effect-proof")]
const UART_IDLE_TIMEOUT_US: u64 = 100_000;
#[cfg(feature = "rp1-bar1-hole-write-effect-proof")]
const FRAME_BEFORE: [u8; 8] = [0xF1, 0x16, 0xB1, 0x55, 0xB2, 0x16, 0xF2, 0x7E];
#[cfg(feature = "rp1-bar1-hole-write-effect-proof")]
const FRAME_DURING: [u8; 8] = [0xF1, 0x16, 0xD1, 0x55, 0xD2, 0x16, 0xF2, 0x7E];
#[cfg(feature = "rp1-bar1-hole-write-effect-proof")]
const FRAME_AFTER: [u8; 8] = [0xF1, 0x16, 0xA1, 0x55, 0xA2, 0x16, 0xF2, 0x7E];
#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
const INTERIOR_UART4_PID0_OFFSET: usize = 0x0004_0fe0;
#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
const INTERIOR_TARGET_UART4: bool = cfg!(feature = "rp1-bar1-second-interior-64k-page-proof");
#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
const INTERIOR_TARGET_PID0_OFFSET: usize = if INTERIOR_TARGET_UART4 {
    INTERIOR_UART4_PID0_OFFSET
} else {
    INTERIOR_UART0_PID0_OFFSET
};
#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
const INTERIOR_POSITIVE_VERDICT: &str = if INTERIOR_TARGET_UART4 {
    "RP1_BAR1_SECOND_INTERIOR_64K_PAGE_PROVEN"
} else {
    "RP1_BAR1_INTERIOR_64K_HOLE_PROVEN"
};
#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
const INTERIOR_NEGATIVE_VERDICT: &str = if INTERIOR_TARGET_UART4 {
    "RP1_BAR1_SECOND_INTERIOR_64K_PAGE_REJECTED"
} else {
    "RP1_BAR1_INTERIOR_64K_HOLE_REJECTED"
};
#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
const INTERIOR_PRE_GO_PROBES: usize = 64;
#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
const INTERIOR_POST_RESTORE_PROBES: usize = 64;
#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
const INTERIOR_ORIGINAL_BAR1: [u32; IATU_SNAPSHOT_WORDS] = [
    0x23,
    0,
    0xc000_0100,
    0,
    0,
    0x0000_ffff,
    0x4000_0000,
    0xc0,
    0,
];
#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
const INTERIOR_A3_UNUSED: [u32; IATU_SNAPSHOT_WORDS] = [0xa3, 0, 0, 0, 0, 0x0000_ffff, 0, 0, 0];
#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
const INTERIOR_E3_UNUSED: [u32; IATU_SNAPSHOT_WORDS] = [0xe3, 0, 0, 0, 0, 0x0000_ffff, 0, 0, 0];
#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
const INTERIOR_A3_UPPER: [u32; IATU_SNAPSHOT_WORDS] = [
    0xa3,
    0,
    0x8000_0000,
    INTERIOR_HOLE_OFFSET + 0x0001_0000,
    0,
    0x003f_ffff,
    0x4000_0000 + INTERIOR_HOLE_OFFSET + 0x0001_0000,
    0xc0,
    0,
];
#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
const INTERIOR_E3_LOWER: [u32; IATU_SNAPSHOT_WORDS] = [
    0xe3,
    0,
    0x8000_0000,
    0,
    0,
    INTERIOR_HOLE_OFFSET - 1,
    0x4000_0000,
    0xc0,
    0,
];
#[cfg(feature = "rp1-iatu-second-spare-programming-proof")]
const SECOND_SPARE_REQUIRED_FLAGS: u32 = 0x7ff;
#[cfg(feature = "rp1-iatu-64k-address-mask-characterization")]
const ADDRESS_MASK_REQUIRED_FLAGS: u32 = 0x7e3f;
#[cfg(feature = "rp1-iatu-64k-address-mask-characterization")]
const ADDRESS_MASK_EXPECTED_FLAGS: u32 = 0x01c0;
#[cfg(feature = "rp1-iatu-64k-address-mask-characterization")]
const ADDRESS_MASK_ALL_FLAGS: u32 = ADDRESS_MASK_REQUIRED_FLAGS | ADDRESS_MASK_EXPECTED_FLAGS;
#[cfg(feature = "rp1-iatu-64k-address-mask-characterization")]
const ADDRESS_MASK_BASE_CHALLENGE: u32 = 0x0001_ffff;
#[cfg(feature = "rp1-iatu-64k-address-mask-characterization")]
const ADDRESS_MASK_BASE_EXPECTED: u32 = 0x0001_0000;
#[cfg(feature = "rp1-iatu-64k-address-mask-characterization")]
const ADDRESS_MASK_LIMIT_CHALLENGE: u32 = 0x003f_0000;
#[cfg(feature = "rp1-iatu-64k-address-mask-characterization")]
const ADDRESS_MASK_LIMIT_EXPECTED: u32 = 0x003f_ffff;
#[cfg(feature = "rp1-iatu-64k-address-mask-characterization")]
const ADDRESS_MASK_TARGET_CHALLENGE: u32 = 0x4001_ffff;
#[cfg(feature = "rp1-iatu-64k-address-mask-characterization")]
const ADDRESS_MASK_TARGET_EXPECTED: u32 = 0x4001_0000;
#[cfg(feature = "rp1-iatu-second-spare-programming-proof")]
const SECOND_SPARE_UNUSED: [u32; IATU_SNAPSHOT_WORDS] = [0xE3, 0, 0, 0, 0, 0x0000_FFFF, 0, 0, 0];
#[cfg(feature = "rp1-iatu-second-spare-programming-proof")]
const SECOND_SPARE_PROGRAMMED: [u32; IATU_SNAPSHOT_WORDS] = [
    0xE3,
    0,
    0,
    0x0001_0000,
    0,
    0x003F_FFFF,
    0x4001_0000,
    0xC0,
    0,
];
#[cfg(all(
    feature = "rp1-iatu-second-spare-programming-proof",
    not(feature = "rp1-iatu-64k-address-mask-characterization")
))]
const _: () = {
    assert!(RECORD_VERSION == 4);
    assert!(CHECKSUM_WORDS == 241);
    assert!(MODE_PROGRAM_SECOND_SPARE == 10);
    assert!(SECOND_SPARE_REQUIRED_FLAGS == 0x7ff);
    assert!(SECOND_SPARE_PROGRAMMED[2] == 0);
    assert!(SECOND_SPARE_PROGRAMMED[3] & 0xffff == 0);
    assert!(SECOND_SPARE_PROGRAMMED[5] & 0xffff == 0xffff);
    assert!(SECOND_SPARE_PROGRAMMED[6] & 0xffff == 0);
};
#[cfg(feature = "rp1-iatu-64k-address-mask-characterization")]
const _: () = {
    assert!(RECORD_VERSION == 5);
    assert!(CHECKSUM_WORDS == 241);
    assert!(MODE_CHARACTERIZE_ADDRESS_MASK == 11);
    assert!(ADDRESS_MASK_REQUIRED_FLAGS == 0x7e3f);
    assert!(ADDRESS_MASK_EXPECTED_FLAGS == 0x01c0);
    assert!(ADDRESS_MASK_ALL_FLAGS == 0x7fff);
    assert!(ADDRESS_MASK_BASE_EXPECTED == ADDRESS_MASK_BASE_CHALLENGE & 0xffff_0000);
    assert!(ADDRESS_MASK_LIMIT_EXPECTED == ADDRESS_MASK_LIMIT_CHALLENGE | 0x0000_ffff);
    assert!(ADDRESS_MASK_TARGET_EXPECTED == ADDRESS_MASK_TARGET_CHALLENGE & 0xffff_0000);
};
#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
const _: () = {
    assert!(RECORD_VERSION == 6);
    assert!(CHECKSUM_WORDS == 241);
    assert!(MODE_INTERIOR_HOLE_64K == 12);
    assert!(INTERIOR_REQUIRED_FLAGS == 0x7fff);
    assert!(INTERIOR_HOLE_OFFSET & 0x0000_ffff == 0);
    assert!(INTERIOR_ORIGINAL_BAR1[2] == 0xc000_0100);
    assert!(INTERIOR_A3_UPPER[0] == 0xa3);
    assert!(INTERIOR_A3_UPPER[3] == INTERIOR_HOLE_OFFSET + 0x0001_0000);
    assert!(INTERIOR_A3_UPPER[6] == 0x4000_0000 + INTERIOR_HOLE_OFFSET + 0x0001_0000);
    assert!(INTERIOR_E3_LOWER[0] == 0xe3);
    assert!(INTERIOR_E3_LOWER[5] == INTERIOR_HOLE_OFFSET - 1);
    assert!(INTERIOR_TARGET_PID0_OFFSET as u32 == INTERIOR_HOLE_OFFSET + 0x0000_0fe0);
};
#[cfg(all(
    feature = "rp1-bar1-hole-write-effect-proof",
    not(feature = "rp1-bar1-second-interior-64k-page-proof")
))]
const _: () = {
    assert!(INTERIOR_HOLE_OFFSET == 0x0003_0000);
    assert!(INTERIOR_UART0_DR_OFFSET == 0x0003_0000);
    assert!(FRAME_BEFORE[3] == 0x55);
    assert!(FRAME_DURING[3] == 0x55);
    assert!(FRAME_AFTER[3] == 0x55);
};
#[cfg(feature = "rp1-bar1-second-interior-64k-page-proof")]
const _: () = {
    assert!(INTERIOR_HOLE_OFFSET == 0x0004_0000);
    assert!(INTERIOR_TARGET_UART4);
    assert!(INTERIOR_TARGET_PID0_OFFSET == 0x0004_0fe0);
    assert!(INTERIOR_A3_UPPER[3] == 0x0005_0000);
    assert!(INTERIOR_A3_UPPER[6] == 0x4005_0000);
    assert!(INTERIOR_E3_LOWER[5] == 0x0003_ffff);
};
#[cfg(all(
    feature = "rp1-iatu-second-spare-programming-proof",
    feature = "rp1-bar1-4k-protection-proof"
))]
compile_error!(
    "rp1-iatu-second-spare-programming-proof must not be combined with active 4K protection modes"
);
#[cfg(all(
    feature = "rp1-bar1-interior-64k-hole-proof",
    any(
        feature = "rp1-iatu-second-spare-programming-proof",
        feature = "rp1-bar1-64k-hole-proof"
    )
))]
compile_error!("rp1-bar1-interior-64k-hole-proof is a distinct terminal iATU mode");
#[cfg(all(
    feature = "rp1-bar1-hole-write-effect-proof",
    feature = "rp1-bar1-second-interior-64k-page-proof"
))]
compile_error!(
    "rp1-bar1-hole-write-effect-proof is pinned to BAR1 page 0x30000 / UART0 DR 0x30000"
);
#[cfg(all(
    feature = "rp1-iatu-second-spare-programming-proof",
    feature = "rp1-bar1-64k-hole-proof"
))]
compile_error!("rp1-iatu-second-spare-programming-proof must not include active 64K hole modes");

global_asm!(
    r#"
    .section .text.rp1_bar1_abort_vector, "ax"
    .balign 2048
    .global __rp1_bar1_abort_vector
__rp1_bar1_abort_vector:
    b __rp1_bar1_unexpected_exception
    .balign 0x80
    b __rp1_bar1_unexpected_exception
    .balign 0x80
    b __rp1_bar1_unexpected_exception
    .balign 0x80
    b __rp1_bar1_unexpected_exception
    .balign 0x80
    b __rp1_bar1_sync_current_spx
    .balign 0x80
    b __rp1_bar1_unexpected_exception
    .balign 0x80
    b __rp1_bar1_unexpected_exception
    .balign 0x80
    b __rp1_bar1_unexpected_exception
    .balign 0x80
    b __rp1_bar1_unexpected_exception
    .balign 0x80
    b __rp1_bar1_unexpected_exception
    .balign 0x80
    b __rp1_bar1_unexpected_exception
    .balign 0x80
    b __rp1_bar1_unexpected_exception
    .balign 0x80
    b __rp1_bar1_unexpected_exception
    .balign 0x80
    b __rp1_bar1_unexpected_exception
    .balign 0x80
    b __rp1_bar1_unexpected_exception
    .balign 0x80
    b __rp1_bar1_unexpected_exception

__rp1_bar1_sync_current_spx:
    mrs x0, esr_el2
    mrs x1, far_el2
    mrs x2, elr_el2
    adrp x3, __rp1_bar1_abort_esr
    add x3, x3, :lo12:__rp1_bar1_abort_esr
    str x0, [x3]
    adrp x3, __rp1_bar1_abort_far
    add x3, x3, :lo12:__rp1_bar1_abort_far
    str x1, [x3]
    adrp x3, __rp1_bar1_abort_elr
    add x3, x3, :lo12:__rp1_bar1_abort_elr
    str x2, [x3]
    lsr x3, x0, #26
    cmp x3, #0x25
    b.ne __rp1_bar1_unexpected_exception
    adrp x3, __rp1_bar1_expected_far
    add x3, x3, :lo12:__rp1_bar1_expected_far
    ldr x3, [x3]
    cmp x1, x3
    b.ne __rp1_bar1_unexpected_exception
    adrp x3, __rp1_bar1_expected_elr
    add x3, x3, :lo12:__rp1_bar1_expected_elr
    ldr x3, [x3]
    cmp x2, x3
    b.ne __rp1_bar1_unexpected_exception
    adrp x3, __rp1_bar1_abort_armed
    add x3, x3, :lo12:__rp1_bar1_abort_armed
    ldr x3, [x3]
    cmp x3, #1
    b.ne __rp1_bar1_unexpected_exception
    adrp x3, __rp1_bar1_abort_mode
    add x3, x3, :lo12:__rp1_bar1_abort_mode
    ldr x3, [x3]
    cmp x3, #5
    b.eq 4f
    cmp x3, #8
    b.eq 4f
    cmp x3, #9
    b.eq 4f
    cmp x3, #12
    b.ne __rp1_bar1_unexpected_exception
4:
    adrp x3, __rp1_bar1_abort_phase
    add x3, x3, :lo12:__rp1_bar1_abort_phase
    ldr x3, [x3]
    cmp x3, #3
    b.ne __rp1_bar1_unexpected_exception
    adrp x3, __rp1_bar1_resume_elr
    add x3, x3, :lo12:__rp1_bar1_resume_elr
    ldr x3, [x3]
    msr elr_el2, x3
    mov x0, #1
    adrp x1, __rp1_bar1_abort_hit
    add x1, x1, :lo12:__rp1_bar1_abort_hit
    str x0, [x1]
    eret

__rp1_bar1_unexpected_exception:
    mov x0, #1
    adrp x1, __rp1_bar1_unexpected_hit
    add x1, x1, :lo12:__rp1_bar1_unexpected_hit
    str x0, [x1]
1:
    wfe
    b 1b

    .section .text.rp1_bar1_abort_probe, "ax"
    .global __rp1_bar1_abort_probe_load
__rp1_bar1_abort_probe_load:
    adrp x1, __rp1_bar1_abort_hit
    add x1, x1, :lo12:__rp1_bar1_abort_hit
    str xzr, [x1]
    adrp x1, __rp1_bar1_unexpected_hit
    add x1, x1, :lo12:__rp1_bar1_unexpected_hit
    str xzr, [x1]
    adrp x1, __rp1_bar1_expected_elr
    add x1, x1, :lo12:__rp1_bar1_expected_elr
    adr x2, 2f
    str x2, [x1]
    adrp x1, __rp1_bar1_resume_elr
    add x1, x1, :lo12:__rp1_bar1_resume_elr
    adr x2, 3f
    str x2, [x1]
    adrp x1, __rp1_bar1_expected_far
    add x1, x1, :lo12:__rp1_bar1_expected_far
    str x0, [x1]
2:
    ldr w0, [x0]
    ret
3:
    mov w0, #0xffffffff
    ret
    "#
);

unsafe extern "C" {
    static __rp1_bar1_abort_vector: u8;
    fn __rp1_bar1_abort_probe_load(addr: *const u32) -> u32;
}

#[unsafe(no_mangle)]
static mut __rp1_bar1_expected_elr: u64 = 0;
#[unsafe(no_mangle)]
static mut __rp1_bar1_resume_elr: u64 = 0;
#[unsafe(no_mangle)]
static mut __rp1_bar1_expected_far: u64 = 0;
#[unsafe(no_mangle)]
static mut __rp1_bar1_abort_hit: u64 = 0;
#[unsafe(no_mangle)]
static mut __rp1_bar1_unexpected_hit: u64 = 0;
#[unsafe(no_mangle)]
static mut __rp1_bar1_abort_esr: u64 = 0;
#[unsafe(no_mangle)]
static mut __rp1_bar1_abort_far: u64 = 0;
#[unsafe(no_mangle)]
static mut __rp1_bar1_abort_elr: u64 = 0;
#[unsafe(no_mangle)]
static mut __rp1_bar1_abort_armed: u64 = 0;
#[unsafe(no_mangle)]
static mut __rp1_bar1_abort_mode: u64 = 0;
#[unsafe(no_mangle)]
static mut __rp1_bar1_abort_phase: u64 = 0;

pub(crate) fn run_after_full_init(rp1: &Rp1Config) {
    crate::logln!("[RP1INBOUND] feature active; fixed protocol only");
    log_endpoint_link_audit(rp1, "entry");
    let Some((bar1_base, bar1_size)) = rp1.peripheral_addr else {
        crate::logln!("[RP1INBOUND] skip: BAR1 missing");
        return;
    };
    let Some((bar2_base, bar2_size)) = rp1.shared_sram_addr else {
        crate::logln!("[RP1INBOUND] skip: BAR2 missing");
        return;
    };
    if bar1_base != EXPECTED_BAR1_BASE as u64
        || bar2_base != EXPECTED_BAR2_BASE as u64
        || bar1_size != EXPECTED_BAR1_SIZE
        || bar2_size != EXPECTED_BAR2_SIZE
    {
        crate::logln!(
            "[RP1INBOUND] skip: exact BAR precondition failed bar1=0x{:x}/0x{:x} bar2=0x{:x}/0x{:x}",
            bar1_base,
            bar1_size,
            bar2_base,
            bar2_size
        );
        return;
    }
    let bar1 = bar1_base as usize;
    let bar2 = bar2_base as usize;
    let chip_id = unsafe { read32(bar1 + CHIP_ID_OFFSET) };
    crate::logln!("[RP1INBOUND] preprobe CHIP_ID=0x{:08x}", chip_id);
    if chip_id != EXPECTED_CHIP_ID {
        crate::logln!("[RP1INBOUND] skip: CHIP_ID mismatch");
        return;
    }
    if !validate_current_el() {
        crate::logln!("[RP1INBOUND] skip: CurrentEL is not EL2");
        return;
    }
    if !validate_record_header(bar2) {
        crate::logln!("[RP1INBOUND] skip: record header/layout mismatch");
        dump_record_summary("bad-header", bar2);
        return;
    }

    let mut seq = unsafe { read_record_word(bar2, W_SEQ) }.wrapping_add(1);
    #[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
    {
        crate::log_rp1_pcie_raw_diag("pre-mode12");
        let _ = run_interior_64k_hole_mode(bar1, bar2, &mut seq);
        crate::log_rp1_pcie_raw_diag("post-mode12-pre-done");
        let done_seq = seq.wrapping_add(1);
        if send_mode_and_wait_ack(bar2, done_seq, MODE_DONE, "DONE") {
            unsafe { write_record_word(bar2, W_GO, done_seq) };
            let _ = wait_rp1_done(bar2, done_seq, "DONE");
        }
        dump_record_summary("DONE", bar2);
        log_endpoint_link_audit(rp1, "done");
        crate::logln!(
            "[RP1INBOUND] mode12 interior 64K hole sequence complete; halting before GDB/Linux"
        );
        return;
    }

    #[cfg(feature = "rp1-iatu-64k-address-mask-characterization")]
    {
        let _ = run_address_mask_characterization_mode(bar1, bar2, &mut seq);
        let done_seq = seq.wrapping_add(1);
        if send_mode_and_wait_ack(bar2, done_seq, MODE_DONE, "DONE") {
            unsafe { write_record_word(bar2, W_GO, done_seq) };
            let _ = wait_rp1_done(bar2, done_seq, "DONE");
        }
        dump_record_summary("DONE", bar2);
        log_endpoint_link_audit(rp1, "done");
        crate::logln!(
            "[RP1INBOUND] mode11 characterization sequence complete; halting before GDB/Linux"
        );
        return;
    }

    #[cfg(all(
        feature = "rp1-iatu-second-spare-programming-proof",
        not(feature = "rp1-iatu-64k-address-mask-characterization")
    ))]
    {
        let _ = run_second_spare_programming_mode(bar1, bar2, &mut seq);
        let done_seq = seq.wrapping_add(1);
        if send_mode_and_wait_ack(bar2, done_seq, MODE_DONE, "DONE") {
            unsafe { write_record_word(bar2, W_GO, done_seq) };
            let _ = wait_rp1_done(bar2, done_seq, "DONE");
        }
        dump_record_summary("DONE", bar2);
        log_endpoint_link_audit(rp1, "done");
        crate::logln!("[RP1INBOUND] mode10 proof sequence complete; halting before GDB/Linux");
        return;
    }

    #[cfg(not(feature = "rp1-iatu-second-spare-programming-proof"))]
    {
        #[cfg(feature = "rp1-axishim-focused-sampling-proof")]
        for &(mode, name) in &[
            (MODE_IDLE, "IDLE"),
            (MODE_BAR2_READ, "BAR2_READ"),
            (MODE_BAR2_WRITE, "BAR2_WRITE"),
            (MODE_BAR1_READ, "BAR1_READ"),
        ] {
            for channel in 0..12 {
                if !run_focused_mode(bar1, bar2, &mut seq, mode, name, channel) {
                    crate::logln!(
                        "[RP1INBOUND] stop: focused mode {} channel {} failed",
                        name,
                        channel
                    );
                    if send_mode_and_wait_ack(bar2, seq, MODE_DONE, "DONE") {
                        unsafe { write_record_word(bar2, W_GO, seq) };
                    }
                    return;
                }
            }
        }

        #[cfg(not(feature = "rp1-axishim-focused-sampling-proof"))]
        for &(mode, name) in &[
            (MODE_IDLE, "IDLE"),
            (MODE_BAR2_READ, "BAR2_READ"),
            (MODE_BAR2_WRITE, "BAR2_WRITE"),
            (MODE_BAR1_READ, "BAR1_READ"),
        ] {
            if !run_mode(bar1, bar2, &mut seq, mode, name) {
                crate::logln!("[RP1INBOUND] stop: mode {} failed", name);
                if send_mode_and_wait_ack(bar2, seq, MODE_DONE, "DONE") {
                    unsafe { write_record_word(bar2, W_GO, seq) };
                }
                return;
            }
        }

        #[cfg(all(
            not(feature = "rp1-bar1-4k-protection-proof"),
            not(feature = "rp1-axishim-focused-sampling-proof")
        ))]
        if monitor_health_allows_block(bar2) {
            let _ = run_block_mode(bar1, bar2, &mut seq);
        } else {
            crate::logln!(
                "[RP1INBOUND] skip BLOCK_BAR1: monitor health/config/checksum gate failed"
            );
        }
        #[cfg(all(
            feature = "rp1-bar1-4k-protection-proof",
            not(feature = "rp1-bar1-64k-hole-proof")
        ))]
        if monitor_health_allows_block(bar2) {
            match run_redirect_4k_mode(bar1, bar2, &mut seq) {
                RedirectOutcome::Proven => {}
                RedirectOutcome::NoEffectClean => {
                    crate::logln!(
                        "[RP1PROTECT] Primary had no redirect effect with clean restore; trying bounded Fallback"
                    );
                    let _ = run_hole_4k_mode(bar1, bar2, &mut seq);
                }
                RedirectOutcome::ProgramGranularityRejectedClean => {
                    crate::logln!(
                        "[RP1PROTECT] Primary 4K tuple was hardware-rounded with exact restore; trying bounded Fallback readback gate"
                    );
                    let _ = run_hole_4k_mode(bar1, bar2, &mut seq);
                }
                RedirectOutcome::Failed => {
                    crate::logln!("[RP1PROTECT] Primary failed; Fallback safety gate closed");
                }
            }
        } else {
            crate::logln!("[RP1PROTECT] skip: monitor health/config/checksum gate failed");
        }
        #[cfg(feature = "rp1-bar1-64k-hole-proof")]
        if monitor_health_allows_block(bar2) {
            let _ = run_hole_64k_mode(bar1, bar2, &mut seq);
        } else {
            crate::logln!("[RP1PROTECT] skip 64K hole: monitor health/config/checksum gate failed");
        }
        let done_seq = seq.wrapping_add(1);
        if send_mode_and_wait_ack(bar2, done_seq, MODE_DONE, "DONE") {
            unsafe { write_record_word(bar2, W_GO, done_seq) };
            let _ = wait_rp1_done(bar2, done_seq, "DONE");
        }
        dump_record_summary("DONE", bar2);
        log_endpoint_link_audit(rp1, "done");
        crate::logln!("[RP1INBOUND] proof sequence complete; halting before GDB/Linux");
    }
}

fn validate_record_header(bar2: usize) -> bool {
    unsafe {
        read_record_word(bar2, W_MAGIC) == RECORD_MAGIC
            && read_record_word(bar2, W_VERSION) == RECORD_VERSION
            && read_record_word(bar2, W_RECORD_SIZE) as usize == RECORD_BYTES
    }
}

fn validate_current_el() -> bool {
    let current_el: u64;
    unsafe {
        asm!("mrs {}, CurrentEL", out(reg) current_el, options(nomem, nostack, preserves_flags));
    }
    crate::logln!("[RP1INBOUND] CurrentEL=0x{:x}", current_el);
    current_el == 0x8
}

fn run_mode(bar1: usize, bar2: usize, seq: &mut u32, mode: u32, name: &'static str) -> bool {
    crate::logln!("[RP1INBOUND:{}] begin seq={}", name, *seq);
    if !send_mode_and_wait_ack(bar2, *seq, mode, name) {
        return false;
    }
    unsafe { write_record_word(bar2, W_GO, *seq) };
    let go_us = now_us();
    if mode != MODE_IDLE {
        busy_wait_us(1_000);
    }
    let (ops, xor) = match mode {
        MODE_IDLE => (0, 0),
        MODE_BAR2_READ => run_bar2_read_traffic(bar2),
        MODE_BAR2_WRITE => run_bar2_write_traffic(bar2),
        MODE_BAR1_READ => run_bar1_read_traffic(bar1),
        _ => (0, 0),
    };
    wait_until_sample_window_clear(go_us);
    unsafe {
        write_record_word(bar2, W_ARG0, ops);
        write_record_word(bar2, W_ARG1, xor);
    }
    let ok = wait_rp1_done(bar2, *seq, name);
    dump_record_summary(name, bar2);
    *seq = seq.wrapping_add(1);
    ok
}

#[cfg(feature = "rp1-axishim-focused-sampling-proof")]
fn run_focused_mode(
    bar1: usize,
    bar2: usize,
    seq: &mut u32,
    mode: u32,
    name: &'static str,
    channel: u32,
) -> bool {
    crate::logln!(
        "[RP1INBOUND:{}] focused channel={} begin seq={}",
        name,
        channel,
        *seq
    );
    unsafe { write_record_word(bar2, W_ARG0, channel) };
    if !send_mode_and_wait_ack(bar2, *seq, mode, name) {
        return false;
    }
    unsafe {
        write_record_word(bar2, W_ARG0, channel);
        write_record_word(bar2, W_GO, *seq);
    }
    let go_us = now_us();
    if mode != MODE_IDLE {
        busy_wait_us(1_000);
    }
    let (ops, xor) = match mode {
        MODE_IDLE => (0, 0),
        MODE_BAR2_READ => run_bar2_read_traffic(bar2),
        MODE_BAR2_WRITE => run_bar2_write_traffic(bar2),
        MODE_BAR1_READ => run_bar1_read_traffic(bar1),
        _ => (0, 0),
    };
    wait_until_sample_window_clear(go_us);
    let ok = wait_rp1_done(bar2, *seq, name);
    crate::logln!(
        "[RP1INBOUND:{}] focused channel={} ops={} xor=0x{:08x}",
        name,
        channel,
        ops,
        xor
    );
    dump_record_summary(name, bar2);
    *seq = seq.wrapping_add(1);
    ok
}

fn run_block_mode(bar1: usize, bar2: usize, seq: &mut u32) -> bool {
    crate::logln!("[RP1INBOUND:BLOCK_BAR1] begin seq={}", *seq);
    let Some(vector) = install_abort_vector() else {
        crate::logln!("[RP1INBOUND:BLOCK_BAR1] skip: vector install precondition failed");
        return false;
    };
    let enabled_probe = unsafe { bar1_abort_probe(bar1 as *const u32) };
    if enabled_probe.expected_abort || enabled_probe.value != EXPECTED_CHIP_ID {
        crate::logln!(
            "[RP1INBOUND:BLOCK_BAR1] skip: enabled preprobe value=0x{:08x} expected_abort={}",
            enabled_probe.value,
            enabled_probe.expected_abort
        );
        restore_abort_vector(vector);
        return false;
    }
    if !send_mode_and_wait_ack(bar2, *seq, MODE_BLOCK_BAR1, "BLOCK_BAR1") {
        restore_abort_vector(vector);
        return false;
    }
    unsafe { write_record_word(bar2, W_GO, *seq) };
    unsafe { arm_expected_abort(MODE_BLOCK_BAR1) };
    let mut counts = BlockProbeCounts::default();
    let mut xor = 0u32;
    for _ in 0..TRAFFIC_OPS {
        let block_phase = unsafe { read_record_word(bar2, W_BLOCK_PHASE) };
        unsafe { set_expected_abort_phase(block_phase) };
        let probe = unsafe { bar1_abort_probe(bar1 as *const u32) };
        xor ^= probe.value;
        counts.record(block_phase, &probe);
        let canary = unsafe { read_record_word(bar2, W_MAGIC) };
        if canary != RECORD_MAGIC {
            counts.canary_bad = counts.canary_bad.wrapping_add(1);
        }
        xor ^= canary;
        busy_wait_us(12);
    }
    unsafe {
        write_record_word(bar2, W_ARG0, TRAFFIC_OPS as u32);
        write_record_word(bar2, W_ARG1, xor);
    }
    let ok = wait_rp1_done(bar2, *seq, "BLOCK_BAR1");
    unsafe {
        set_expected_abort_phase(read_record_word(bar2, W_BLOCK_PHASE));
    }
    let post_probe = unsafe { bar1_abort_probe(bar1 as *const u32) };
    unsafe { disarm_expected_abort() };
    restore_abort_vector(vector);
    crate::logln!(
        "[RP1INBOUND:BLOCK_BAR1] before valid/allff/abort={}/{}/{} during={}/{}/{} after={}/{}/{} canary_bad={} xor=0x{:08x} post_value=0x{:08x} post_abort={}",
        counts.before.valid,
        counts.before.all_ff,
        counts.before.abort,
        counts.during.valid,
        counts.during.all_ff,
        counts.during.abort,
        counts.after.valid,
        counts.after.all_ff,
        counts.after.abort,
        counts.canary_bad,
        xor,
        post_probe.value,
        post_probe.expected_abort
    );
    dump_record_summary("BLOCK_BAR1", bar2);
    *seq = seq.wrapping_add(1);
    ok && post_probe.value == EXPECTED_CHIP_ID && !post_probe.expected_abort
}

#[cfg(feature = "rp1-iatu-64k-address-mask-characterization")]
fn run_address_mask_characterization_mode(bar1: usize, bar2: usize, seq: &mut u32) -> bool {
    const NAME: &str = "CHARACTERIZE_ADDRESS_MASK";
    crate::logln!("[RP1PROTECT:{}] begin seq={}", NAME, *seq);
    log_phase13_live_observation("pre-mode11", bar1, bar2);
    if !send_mode_and_wait_ack(bar2, *seq, MODE_CHARACTERIZE_ADDRESS_MASK, NAME) {
        return false;
    }
    unsafe { write_record_word(bar2, W_GO, *seq) };
    let completion_ok = wait_rp1_done(bar2, *seq, NAME);
    let post_chip_id = unsafe { read32(bar1 + CHIP_ID_OFFSET) };
    log_phase13_live_observation("post-mode11", bar1, bar2);
    let ok = completion_ok
        && post_chip_id == EXPECTED_CHIP_ID
        && address_mask_characterization_record_gate(bar2, *seq);
    let result = unsafe { read_record_word(bar2, W_RESULT) };
    dump_record_summary(NAME, bar2);
    dump_protection_summary(NAME, bar2);
    crate::logln!(
        "[RP1PHASE13] post-command CHIP_ID=0x{:08x} stable={} result={} verdict={}",
        post_chip_id,
        post_chip_id == EXPECTED_CHIP_ID,
        result,
        if ok {
            if result == 1 {
                "RP1_IATU_64K_ADDRESS_MASK_HW_CHARACTERIZED"
            } else {
                "RP1_IATU_LOW_BIT_MASK_CHARACTERIZED_DIFFERENT_FROM_64K_HYPOTHESIS"
            }
        } else {
            "RP1_IATU_ADDRESS_MASK_CHARACTERIZATION_REJECTED"
        }
    );
    *seq = seq.wrapping_add(1);
    ok
}

#[cfg(feature = "rp1-iatu-64k-address-mask-characterization")]
fn log_phase13_live_observation(label: &'static str, bar1: usize, bar2: usize) {
    unsafe {
        crate::logln!(
            "[RP1PHASE13:{}] live chip_id=0x{:08x} bar2_magic=0x{:08x} bar2_version={} bar2_size={} seq={} ack={} go={} mode={} phase={} completion=0x{:08x} completion_seq={} flags=0x{:08x} protect_flags=0x{:08x}",
            label,
            read32(bar1 + CHIP_ID_OFFSET),
            read_record_word(bar2, W_MAGIC),
            read_record_word(bar2, W_VERSION),
            read_record_word(bar2, W_RECORD_SIZE),
            read_record_word(bar2, W_SEQ),
            read_record_word(bar2, W_ACK),
            read_record_word(bar2, W_GO),
            read_record_word(bar2, W_MODE),
            read_record_word(bar2, W_PHASE),
            read_record_word(bar2, W_COMPLETION),
            read_record_word(bar2, W_COMPLETION_SEQ),
            read_record_word(bar2, W_FLAGS),
            read_record_word(bar2, W_PROTECT_FLAGS),
        );
    }
    crate::log_rp1_pcie_raw_diag(label);
}

#[cfg(feature = "rp1-iatu-64k-address-mask-characterization")]
fn address_mask_characterization_record_gate(bar2: usize, seq: u32) -> bool {
    let bar1_before = read_iatu_snapshot_words(bar2, W_ORIGINAL_BAR1);
    let bar2_before = read_iatu_snapshot_words(bar2, W_SPARE_BEFORE);
    let a3_before = read_iatu_snapshot_words(bar2, W_SPARE_BEFORE + IATU_SNAPSHOT_WORDS);
    let e3_before = read_iatu_snapshot_words(bar2, W_SPARE_PROGRAMMED);
    let anchor_readback = read_iatu_snapshot_words(bar2, W_SPARE_PROGRAMMED + IATU_SNAPSHOT_WORDS);
    let bar1_after = read_iatu_snapshot_words(bar2, W_SPARE_READBACK);
    let bar2_after = read_iatu_snapshot_words(bar2, W_SPARE_READBACK + IATU_SNAPSHOT_WORDS);
    let a3_after = read_iatu_snapshot_words(bar2, W_SPARE_RESTORED);
    let e3_after = read_iatu_snapshot_words(bar2, W_SPARE_RESTORED + IATU_SNAPSHOT_WORDS);
    let flags = unsafe { read_record_word(bar2, W_FLAGS) };
    let protect_flags = unsafe { read_record_word(bar2, W_PROTECT_FLAGS) };
    let result = unsafe { read_record_word(bar2, W_RESULT) };
    let base_readback = unsafe { read_record_word(bar2, W_PROTECT_BAR1_BUS_BASE) };
    let limit_readback = unsafe { read_record_word(bar2, W_TARGET_BEFORE) };
    let target_readback = unsafe { read_record_word(bar2, W_TARGET_AFTER) };
    let observed_expected_flags = u32::from(base_readback == ADDRESS_MASK_BASE_EXPECTED) << 6
        | u32::from(limit_readback == ADDRESS_MASK_LIMIT_EXPECTED) << 7
        | u32::from(target_readback == ADDRESS_MASK_TARGET_EXPECTED) << 8;
    let started = unsafe {
        u64::from(read_record_word(bar2, W_STARTED_US_LO))
            | (u64::from(read_record_word(bar2, W_STARTED_US_HI)) << 32)
    };
    let ended = unsafe {
        u64::from(read_record_word(bar2, W_ENDED_US_LO))
            | (u64::from(read_record_word(bar2, W_ENDED_US_HI)) << 32)
    };
    let elapsed = unsafe { read_record_word(bar2, W_ELAPSED_US) };
    let enable_us = unsafe { read_record_word(bar2, W_PROTECT_ENABLE_US) };
    let disable_us = unsafe { read_record_word(bar2, W_PROTECT_DISABLE_US) };
    let restore_us = unsafe { read_record_word(bar2, W_PROTECT_RESTORE_US) };
    let timing_ok = started != 0
        && ended >= started
        && elapsed != 0
        && ended - started <= u64::from(elapsed)
        && enable_us != 0
        && enable_us <= disable_us
        && disable_us <= restore_us
        && restore_us <= elapsed;
    let result_ok = match result {
        1 => observed_expected_flags == ADDRESS_MASK_EXPECTED_FLAGS,
        2 => observed_expected_flags != ADDRESS_MASK_EXPECTED_FLAGS,
        _ => false,
    };
    let command = unsafe { read_record_word(bar2, W_COMMAND) };
    let ok = validate_record_header(bar2)
        && validate_checksum(bar2)
        && unsafe { read_record_word(bar2, W_ACK) } == seq
        && unsafe { read_record_word(bar2, W_GO) } == seq
        && unsafe { read_record_word(bar2, W_MODE) } == MODE_CHARACTERIZE_ADDRESS_MASK
        && unsafe { read_record_word(bar2, W_PHASE) } == PHASE_DONE
        && unsafe { read_record_word(bar2, W_COMPLETION) } == COMPLETION_DONE
        && unsafe { read_record_word(bar2, W_COMPLETION_SEQ) } == seq
        && unsafe { read_record_word(bar2, W_BLOCK_PHASE) } == BLOCK_PHASE_RESTORED
        && flags == protect_flags
        && flags & ADDRESS_MASK_REQUIRED_FLAGS == ADDRESS_MASK_REQUIRED_FLAGS
        && flags & !ADDRESS_MASK_ALL_FLAGS == 0
        && flags & ADDRESS_MASK_EXPECTED_FLAGS == observed_expected_flags
        && result_ok
        && unsafe { read_record_word(bar2, W_TARGET_PAGE_OFFSET) } == ADDRESS_MASK_BASE_CHALLENGE
        && unsafe { read_record_word(bar2, W_DUMMY_LOCAL_BASE) } == ADDRESS_MASK_LIMIT_CHALLENGE
        && unsafe { read_record_word(bar2, W_TARGET_DURING) } == ADDRESS_MASK_TARGET_CHALLENGE
        && unsafe { read_record_word(bar2, W_CTRL2_BEFORE) } == 0
        && unsafe { read_record_word(bar2, W_CTRL2_BLOCK_VALUE) } == 0
        && unsafe { read_record_word(bar2, W_CTRL2_BLOCK_READBACK) } == 0
        && unsafe { read_record_word(bar2, W_CTRL2_RESTORE_READBACK) } == 0
        && unsafe { read_record_word(bar2, W_SELECTOR_RESTORE_READBACK) }
            == unsafe { read_record_word(bar2, W_SELECTOR_SAVED) }
        && unsafe { read_record_word(bar2, W_CONTROL_BEFORE) } == EXPECTED_CHIP_ID
        && unsafe { read_record_word(bar2, W_CONTROL_DURING) } == EXPECTED_CHIP_ID
        && unsafe { read_record_word(bar2, W_CONTROL_AFTER) } == EXPECTED_CHIP_ID
        && unsafe { read_record_word(bar2, W_BAR0) } == 0x0080_0000
        && unsafe { read_record_word(bar2, W_BAR1) } == 0
        && unsafe { read_record_word(bar2, W_BAR2) } == 0x0040_0000
        && unsafe { read_record_word(bar2, W_BAR1_BUS_BASE) } == 0
        && command & 0x2 != 0
        && bar1_before[0] == 0x23
        && bar2_before[0] == 0x63
        && a3_before[0] == 0xa3
        && bar1_after == bar1_before
        && bar2_after == bar2_before
        && a3_after == a3_before
        && e3_before == SECOND_SPARE_UNUSED
        && anchor_readback == SECOND_SPARE_PROGRAMMED
        && e3_after == e3_before
        && timing_ok;
    crate::logln!(
        "[RP1PHASE13] gate ok={} result={} flags=0x{:08x} expected_observed=0x{:03x} base=0x{:08x}->0x{:08x} limit=0x{:08x}->0x{:08x} target=0x{:08x}->0x{:08x} timing={}/{}/{}/{}",
        ok,
        result,
        flags,
        observed_expected_flags,
        ADDRESS_MASK_BASE_CHALLENGE,
        base_readback,
        ADDRESS_MASK_LIMIT_CHALLENGE,
        limit_readback,
        ADDRESS_MASK_TARGET_CHALLENGE,
        target_readback,
        enable_us,
        disable_us,
        restore_us,
        elapsed,
    );
    ok
}

#[cfg(all(
    feature = "rp1-iatu-second-spare-programming-proof",
    not(feature = "rp1-iatu-64k-address-mask-characterization")
))]
fn run_second_spare_programming_mode(bar1: usize, bar2: usize, seq: &mut u32) -> bool {
    const NAME: &str = "PROGRAM_SECOND_SPARE";
    crate::logln!("[RP1PROTECT:{}] begin seq={}", NAME, *seq);
    log_phase12_live_observation("pre-mode10", bar1, bar2);
    if !send_mode_and_wait_ack(bar2, *seq, MODE_PROGRAM_SECOND_SPARE, NAME) {
        return false;
    }
    unsafe { write_record_word(bar2, W_GO, *seq) };
    let completion_ok = wait_rp1_done(bar2, *seq, NAME);
    let post_chip_id = unsafe { read32(bar1 + CHIP_ID_OFFSET) };
    log_phase12_live_observation("post-mode10-pre-done", bar1, bar2);
    let ok =
        completion_ok && post_chip_id == EXPECTED_CHIP_ID && second_spare_record_gate(bar2, *seq);
    dump_record_summary(NAME, bar2);
    dump_protection_summary(NAME, bar2);
    crate::logln!(
        "[RP1PROTECT:{}] post-command CHIP_ID=0x{:08x} stable={}",
        NAME,
        post_chip_id,
        post_chip_id == EXPECTED_CHIP_ID
    );
    crate::logln!(
        "[RP1PROTECT] verdict={}",
        if ok {
            "RP1_IATU_SECOND_SPARE_PROGRAMMING_PROVEN"
        } else {
            "RP1_IATU_SECOND_SPARE_PROGRAMMING_REJECTED"
        }
    );
    *seq = seq.wrapping_add(1);
    ok
}

#[cfg(all(
    feature = "rp1-iatu-second-spare-programming-proof",
    not(feature = "rp1-iatu-64k-address-mask-characterization")
))]
fn log_phase12_live_observation(label: &'static str, bar1: usize, bar2: usize) {
    unsafe {
        let chip_id = read32(bar1 + CHIP_ID_OFFSET);
        crate::logln!(
            "[RP1PHASE12:{}] live chip_id=0x{:08x} bar2_magic=0x{:08x} bar2_version={} bar2_size={} seq={} ack={} go={} mode={} phase={} completion=0x{:08x} completion_seq={} flags=0x{:08x} protect_flags=0x{:08x}",
            label,
            chip_id,
            read_record_word(bar2, W_MAGIC),
            read_record_word(bar2, W_VERSION),
            read_record_word(bar2, W_RECORD_SIZE),
            read_record_word(bar2, W_SEQ),
            read_record_word(bar2, W_ACK),
            read_record_word(bar2, W_GO),
            read_record_word(bar2, W_MODE),
            read_record_word(bar2, W_PHASE),
            read_record_word(bar2, W_COMPLETION),
            read_record_word(bar2, W_COMPLETION_SEQ),
            read_record_word(bar2, W_FLAGS),
            read_record_word(bar2, W_PROTECT_FLAGS),
        );
    }
    crate::log_rp1_pcie_raw_diag(label);
}

#[cfg(all(
    feature = "rp1-iatu-second-spare-programming-proof",
    not(feature = "rp1-iatu-64k-address-mask-characterization")
))]
fn second_spare_record_gate(bar2: usize, seq: u32) -> bool {
    let bar1_before = read_iatu_snapshot_words(bar2, W_ORIGINAL_BAR1);
    let bar2_before = read_iatu_snapshot_words(bar2, W_SPARE_BEFORE);
    let a3_before = read_iatu_snapshot_words(bar2, W_SPARE_BEFORE + IATU_SNAPSHOT_WORDS);
    let e3_before = read_iatu_snapshot_words(bar2, W_SPARE_PROGRAMMED);
    let bar1_after = read_iatu_snapshot_words(bar2, W_SPARE_PROGRAMMED + IATU_SNAPSHOT_WORDS);
    let e3_readback = read_iatu_snapshot_words(bar2, W_SPARE_READBACK);
    let bar2_after = read_iatu_snapshot_words(bar2, W_SPARE_READBACK + IATU_SNAPSHOT_WORDS);
    let e3_restored = read_iatu_snapshot_words(bar2, W_SPARE_RESTORED);
    let a3_after = read_iatu_snapshot_words(bar2, W_SPARE_RESTORED + IATU_SNAPSHOT_WORDS);
    let command = unsafe { read_record_word(bar2, W_COMMAND) };
    let ok = validate_record_header(bar2)
        && validate_checksum(bar2)
        && unsafe { read_record_word(bar2, W_ACK) } == seq
        && unsafe { read_record_word(bar2, W_GO) } == seq
        && unsafe { read_record_word(bar2, W_MODE) } == MODE_PROGRAM_SECOND_SPARE
        && unsafe { read_record_word(bar2, W_PHASE) } == PHASE_DONE
        && unsafe { read_record_word(bar2, W_COMPLETION) } == COMPLETION_DONE
        && unsafe { read_record_word(bar2, W_COMPLETION_SEQ) } == seq
        && unsafe { read_record_word(bar2, W_RESULT) } == COMPLETION_DONE
        && unsafe { read_record_word(bar2, W_BLOCK_PHASE) } == BLOCK_PHASE_RESTORED
        && unsafe { read_record_word(bar2, W_FLAGS) } == SECOND_SPARE_REQUIRED_FLAGS
        && unsafe { read_record_word(bar2, W_PROTECT_FLAGS) } == SECOND_SPARE_REQUIRED_FLAGS
        && unsafe { read_record_word(bar2, W_CTRL2_BEFORE) } == 0
        && unsafe { read_record_word(bar2, W_CTRL2_BLOCK_VALUE) } == 0
        && unsafe { read_record_word(bar2, W_CTRL2_BLOCK_READBACK) } == 0
        && unsafe { read_record_word(bar2, W_CTRL2_RESTORE_READBACK) } == 0
        && unsafe { read_record_word(bar2, W_SELECTOR_RESTORE_READBACK) }
            == unsafe { read_record_word(bar2, W_SELECTOR_SAVED) }
        && unsafe { read_record_word(bar2, W_TARGET_BEFORE) } == EXPECTED_CHIP_ID
        && unsafe { read_record_word(bar2, W_TARGET_DURING) } == EXPECTED_CHIP_ID
        && unsafe { read_record_word(bar2, W_TARGET_AFTER) } == EXPECTED_CHIP_ID
        && unsafe { read_record_word(bar2, W_BAR0) } == 0x0080_0000
        && unsafe { read_record_word(bar2, W_BAR1) } == 0
        && unsafe { read_record_word(bar2, W_BAR2) } == 0x0040_0000
        && unsafe { read_record_word(bar2, W_BAR1_BUS_BASE) } == 0
        && command & 0x2 != 0
        && unsafe { read_record_word(bar2, W_CONTROL_BEFORE) } == command
        && unsafe { read_record_word(bar2, W_CONTROL_DURING) } == 0
        && unsafe { read_record_word(bar2, W_CONTROL_AFTER) } == command
        && bar1_before[0] == 0x23
        && bar2_before[0] == 0x63
        && a3_before[0] == 0xA3
        && bar1_after == bar1_before
        && bar2_after == bar2_before
        && a3_after == a3_before
        && e3_before == SECOND_SPARE_UNUSED
        && e3_readback == SECOND_SPARE_PROGRAMMED
        && e3_restored == e3_before;
    crate::logln!(
        "[RP1PROTECT] second_spare_gate ok={} flags=0x{:08x} protect_flags=0x{:08x} command=0x{:08x}",
        ok,
        unsafe { read_record_word(bar2, W_FLAGS) },
        unsafe { read_record_word(bar2, W_PROTECT_FLAGS) },
        command
    );
    ok
}

#[cfg(feature = "rp1-bar1-4k-protection-proof")]
enum RedirectOutcome {
    Proven,
    NoEffectClean,
    ProgramGranularityRejectedClean,
    Failed,
}

#[cfg(feature = "rp1-bar1-4k-protection-proof")]
fn run_redirect_4k_mode(bar1: usize, bar2: usize, seq: &mut u32) -> RedirectOutcome {
    const NAME: &str = "REDIRECT_4K";
    crate::logln!("[RP1PROTECT:{}] begin seq={}", NAME, *seq);

    let target_pre = unsafe { read32(bar1 + CHIP_ID_OFFSET) };
    let control_pre = unsafe { read32(bar1 + CONTROL_OFFSET) };
    let canary_pre = unsafe { read_record_word(bar2, W_MAGIC) };
    if target_pre != EXPECTED_CHIP_ID
        || !control_value_valid(control_pre)
        || canary_pre != RECORD_MAGIC
    {
        crate::logln!(
            "[RP1PROTECT:{}] skip: host precondition target=0x{:08x} control=0x{:08x} canary=0x{:08x}",
            NAME,
            target_pre,
            control_pre,
            canary_pre
        );
        return RedirectOutcome::Failed;
    }
    if !send_mode_and_wait_ack(bar2, *seq, MODE_REDIRECT_4K, NAME) {
        return RedirectOutcome::Failed;
    }

    if !validate_dummy_page_from_bar2(bar2, *seq) {
        crate::logln!(
            "[RP1PROTECT:{}] host dummy gate failed; forcing protocol reject",
            NAME
        );
        unsafe {
            write_record_word(bar2, W_MODE, 0);
            write_record_word(bar2, W_GO, *seq);
        }
        let _ = wait_rp1_done(bar2, *seq, NAME);
        dump_record_summary(NAME, bar2);
        dump_protection_summary(NAME, bar2);
        *seq = seq.wrapping_add(1);
        return RedirectOutcome::Failed;
    }

    unsafe { write_record_word(bar2, W_GO, *seq) };
    let mut counts = ProtectionProbeCounts::default();
    for _ in 0..TRAFFIC_OPS {
        let phase_before = unsafe { read_record_word(bar2, W_BLOCK_PHASE) };
        let event_us = unsafe { read_record_word(bar2, W_PROTECT_ENABLE_US) };
        let target = Bar1Probe {
            value: unsafe { read32(bar1 + CHIP_ID_OFFSET) },
            expected_abort: false,
        };
        let control = unsafe { read32(bar1 + CONTROL_OFFSET) };
        let canary = unsafe { read_record_word(bar2, W_MAGIC) };
        let phase_after = unsafe { read_record_word(bar2, W_BLOCK_PHASE) };
        counts.record(
            phase_before,
            phase_after,
            event_us,
            &target,
            control,
            canary,
        );
        busy_wait_us(12);
    }
    unsafe {
        write_record_word(bar2, W_ARG0, TRAFFIC_OPS as u32);
        write_record_word(bar2, W_ARG1, counts.xor);
    }
    let completion_ok = wait_rp1_done(bar2, *seq, NAME);
    let post_target = unsafe { read32(bar1 + CHIP_ID_OFFSET) };
    let post_control = unsafe { read32(bar1 + CONTROL_OFFSET) };
    let post_canary = unsafe { read_record_word(bar2, W_MAGIC) };
    let restored =
        protection_restore_gate(bar2, completion_ok, post_target, post_control, post_canary);
    let program_granularity_rejected_clean = redirect_program_granularity_rejected_clean(
        bar2,
        *seq,
        completion_ok,
        post_target,
        post_control,
        post_canary,
    );

    log_protection_probe_counts(NAME, &counts, post_target, false);
    dump_record_summary(NAME, bar2);
    dump_protection_summary(NAME, bar2);
    *seq = seq.wrapping_add(1);

    let during = &counts.during;
    let redirect_proven = restored
        && during.stable != 0
        && during.target.dummy == during.stable
        && during.target.expected == 0
        && during.target.all_ff == 0
        && during.target.other == 0
        && during.target.abort == 0
        && during.control.valid == during.stable
        && counts.canary_bad == 0;
    if redirect_proven {
        crate::logln!("[RP1PROTECT] verdict=RP1_BAR1_4K_REDIRECT_PROVEN");
        return RedirectOutcome::Proven;
    }

    if program_granularity_rejected_clean {
        crate::logln!("[RP1PROTECT] observation=RP1_BAR1_4K_PROGRAM_GRANULARITY_REJECTED_CLEAN");
        return RedirectOutcome::ProgramGranularityRejectedClean;
    }

    let clean_no_effect = restored
        && during.stable != 0
        && during.target.expected == during.stable
        && during.target.dummy == 0
        && during.target.all_ff == 0
        && during.target.other == 0
        && during.target.abort == 0
        && during.control.valid == during.stable
        && counts.canary_bad == 0;
    if clean_no_effect {
        crate::logln!("[RP1PROTECT] verdict=RP1_BAR1_4K_OVERLAY_NO_EFFECT_CLEAN");
        RedirectOutcome::NoEffectClean
    } else {
        crate::logln!("[RP1PROTECT] verdict=RP1_BAR1_4K_OVERLAY_PRIORITY_UNRESOLVED");
        RedirectOutcome::Failed
    }
}

#[cfg(feature = "rp1-bar1-4k-protection-proof")]
fn run_hole_4k_mode(bar1: usize, bar2: usize, seq: &mut u32) -> bool {
    run_hole_mode(
        bar1,
        bar2,
        seq,
        MODE_HOLE_4K,
        0x1000,
        "HOLE_4K",
        "RP1_BAR1_4K_HOLE_PROVEN",
        "RP1_BAR1_ADDRESS_MATCH_REPLACEMENT_FAILED",
    )
}

#[cfg(feature = "rp1-bar1-64k-hole-proof")]
fn run_hole_64k_mode(bar1: usize, bar2: usize, seq: &mut u32) -> bool {
    run_hole_mode(
        bar1,
        bar2,
        seq,
        MODE_HOLE_64K,
        0x1_0000,
        "HOLE_64K",
        "RP1_BAR1_64K_HOLE_PROVEN",
        "RP1_BAR1_64K_ADDRESS_MATCH_REPLACEMENT_FAILED",
    )
}

#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
fn run_interior_64k_hole_mode(bar1: usize, bar2: usize, seq: &mut u32) -> bool {
    const NAME: &str = "INTERIOR_HOLE_64K";
    crate::logln!("[RP1PROTECT:{}] begin seq={}", NAME, *seq);
    let Some(vector) = install_abort_vector() else {
        crate::logln!(
            "[RP1PROTECT:{}] skip: vector install precondition failed",
            NAME
        );
        return false;
    };
    if !send_mode_and_wait_ack(bar2, *seq, MODE_INTERIOR_HOLE_64K, NAME) {
        restore_abort_vector(vector);
        return false;
    }

    let mut counts = InteriorProbeCounts::default();
    unsafe { arm_expected_abort(MODE_INTERIOR_HOLE_64K) };
    let Some(anchors) = capture_interior_anchors(bar1) else {
        crate::logln!("[RP1PHASE14] skip: invalid pre-GO UART anchors");
        unsafe { disarm_expected_abort() };
        restore_abort_vector(vector);
        return false;
    };
    crate::logln!(
        "[RP1PHASE14:anchors] phase={} uart0=0x{:08x} uart4=0x{:08x}",
        unsafe { read_record_word(bar2, W_BLOCK_PHASE) },
        anchors.uart0,
        anchors.uart4,
    );
    for _ in 0..INTERIOR_PRE_GO_PROBES {
        let phase_before = unsafe { read_record_word(bar2, W_BLOCK_PHASE) };
        unsafe { set_expected_abort_phase(phase_before) };
        record_interior_probe(bar1, bar2, &mut counts, phase_before, false, &anchors);
    }
    if counts.before.samples != INTERIOR_PRE_GO_PROBES as u32
        || !counts.before.gate_expected()
        || counts.boundary != 0
        || counts.unprobed != 0
    {
        crate::logln!("[RP1PHASE14] skip: unstable pre-GO anchor cohort");
        log_interior_probe_counts(&counts);
        unsafe { disarm_expected_abort() };
        restore_abort_vector(vector);
        return false;
    }
    #[cfg(feature = "rp1-bar1-hole-write-effect-proof")]
    let mut write_sequence_gate = InteriorWriteSequenceGate::default();
    #[cfg(feature = "rp1-bar1-hole-write-effect-proof")]
    if !write_sequence_gate.before(bar1, bar2) {
        unsafe { disarm_expected_abort() };
        restore_abort_vector(vector);
        return false;
    }
    unsafe { write_record_word(bar2, W_GO, *seq) };
    for _ in 0..TRAFFIC_OPS {
        let phase_before = unsafe { read_record_word(bar2, W_BLOCK_PHASE) };
        unsafe { set_expected_abort_phase(phase_before) };
        #[cfg(feature = "rp1-bar1-hole-write-effect-proof")]
        if !write_sequence_gate.during_done && phase_before == BLOCK_PHASE_DISABLED {
            write_sequence_gate.during(bar1, bar2);
        }
        record_interior_probe(bar1, bar2, &mut counts, phase_before, false, &anchors);
        busy_wait_us(12);
    }

    let completion_ok = wait_rp1_done(bar2, *seq, NAME);
    let phase = unsafe { read_record_word(bar2, W_BLOCK_PHASE) };
    if completion_ok && phase == BLOCK_PHASE_RESTORED {
        for _ in 0..INTERIOR_POST_RESTORE_PROBES {
            let phase_before = unsafe { read_record_word(bar2, W_BLOCK_PHASE) };
            unsafe { set_expected_abort_phase(phase_before) };
            record_interior_probe(bar1, bar2, &mut counts, phase_before, true, &anchors);
        }
        #[cfg(feature = "rp1-bar1-hole-write-effect-proof")]
        write_sequence_gate.after(bar1, bar2);
    } else {
        unsafe { set_expected_abort_phase(phase) };
        record_interior_probe(bar1, bar2, &mut counts, phase, false, &anchors);
    }
    unsafe {
        write_record_word(bar2, W_ARG0, TRAFFIC_OPS as u32);
        write_record_word(bar2, W_ARG1, counts.xor);
    }
    unsafe { disarm_expected_abort() };
    restore_abort_vector(vector);

    let record_ok = interior_hole_record_gate(bar2, *seq);
    let probes_ok = counts.gate();
    #[cfg(feature = "rp1-bar1-hole-write-effect-proof")]
    let write_sequence_ok = write_sequence_gate.gate();
    #[cfg(not(feature = "rp1-bar1-hole-write-effect-proof"))]
    let write_sequence_ok = true;
    dump_record_summary(NAME, bar2);
    dump_protection_summary(NAME, bar2);
    log_interior_probe_counts(&counts);
    let ok = completion_ok && record_ok && probes_ok && write_sequence_ok;
    crate::logln!(
        "[RP1PHASE14] gate ok={} completion={} record={} probes={} flags=0x{:08x} result={} verdict={}",
        ok,
        completion_ok,
        record_ok,
        probes_ok,
        unsafe { read_record_word(bar2, W_FLAGS) },
        unsafe { read_record_word(bar2, W_RESULT) },
        if ok {
            INTERIOR_POSITIVE_VERDICT
        } else {
            INTERIOR_NEGATIVE_VERDICT
        }
    );
    *seq = seq.wrapping_add(1);
    ok
}

#[cfg(feature = "rp1-bar1-hole-write-effect-proof")]
struct InteriorWriteSequenceGate {
    before_done: bool,
    during_done: bool,
    after_done: bool,
    ok: bool,
    sequence: u32,
}

#[cfg(feature = "rp1-bar1-hole-write-effect-proof")]
impl Default for InteriorWriteSequenceGate {
    fn default() -> Self {
        Self {
            before_done: false,
            during_done: false,
            after_done: false,
            ok: true,
            sequence: 0,
        }
    }
}

#[cfg(feature = "rp1-bar1-hole-write-effect-proof")]
impl InteriorWriteSequenceGate {
    fn before(&mut self, bar1: usize, bar2: usize) -> bool {
        let phase = unsafe { read_record_word(bar2, W_BLOCK_PHASE) };
        if phase != BLOCK_PHASE_IDLE || !uart0_wait_idle(bar1, "BEFORE/pre") {
            crate::logln!("[RP1PHASE16] BEFORE rejected phase={}", phase);
            return false;
        }
        let ok = self.send_frame(bar1, bar2, "BEFORE", BLOCK_PHASE_IDLE, &FRAME_BEFORE)
            && uart0_wait_idle(bar1, "BEFORE/post");
        self.before_done = ok;
        self.ok &= ok;
        ok
    }

    fn during(&mut self, bar1: usize, bar2: usize) {
        self.during_done = true;
        let ok = self.send_frame(bar1, bar2, "DURING", BLOCK_PHASE_DISABLED, &FRAME_DURING);
        self.ok &= ok;
    }

    fn after(&mut self, bar1: usize, bar2: usize) {
        let phase = unsafe { read_record_word(bar2, W_BLOCK_PHASE) };
        let ok = phase == BLOCK_PHASE_RESTORED
            && uart0_wait_idle(bar1, "AFTER/pre")
            && self.send_frame(bar1, bar2, "AFTER", BLOCK_PHASE_RESTORED, &FRAME_AFTER)
            && uart0_wait_idle(bar1, "AFTER/post");
        self.after_done = ok;
        self.ok &= ok;
    }

    fn send_frame(
        &mut self,
        bar1: usize,
        bar2: usize,
        name: &'static str,
        expected_phase: u32,
        frame: &[u8; 8],
    ) -> bool {
        self.sequence = self.sequence.wrapping_add(1);
        let start = now_us();
        let phase_before = unsafe { read_record_word(bar2, W_BLOCK_PHASE) };
        for byte in frame {
            unsafe { write32(bar1 + INTERIOR_UART0_DR_OFFSET, u32::from(*byte)) };
        }
        let drain = unsafe { read32(bar1 + CHIP_ID_OFFSET) };
        let end = now_us();
        let phase_after = unsafe { read_record_word(bar2, W_BLOCK_PHASE) };
        let ok = phase_before == expected_phase
            && phase_after == expected_phase
            && drain == EXPECTED_CHIP_ID
            && frame[3] == 0x55;
        crate::logln!(
            "[RP1PHASE16:{}] seq={} ok={} phase={}/{} addr=0x{:x} width=32 central=0x{:02x} frame={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} drain=0x{:08x} timing_us={}/{}",
            name,
            self.sequence,
            ok,
            phase_before,
            phase_after,
            bar1 + INTERIOR_UART0_DR_OFFSET,
            frame[3],
            frame[0],
            frame[1],
            frame[2],
            frame[3],
            frame[4],
            frame[5],
            frame[6],
            frame[7],
            drain,
            start,
            end.wrapping_sub(start),
        );
        ok
    }

    fn gate(&self) -> bool {
        let ok = self.ok && self.before_done && self.during_done && self.after_done;
        crate::logln!(
            "[RP1PHASE16] internal_sequence_gate ok={} before={} during={} after={} seq={}",
            ok,
            self.before_done,
            self.during_done,
            self.after_done,
            self.sequence,
        );
        ok
    }
}

#[cfg(feature = "rp1-bar1-hole-write-effect-proof")]
fn uart0_wait_idle(bar1: usize, label: &'static str) -> bool {
    let start = now_us();
    loop {
        let fr = unsafe { read32(bar1 + INTERIOR_UART0_FR_OFFSET) };
        if fr & (UART_FR_TXFE | UART_FR_BUSY) == UART_FR_TXFE {
            crate::logln!(
                "[RP1PHASE16:{}] uart0_idle ok=true FR=0x{:08x} timing_us={}",
                label,
                fr,
                now_us().wrapping_sub(start),
            );
            return true;
        }
        if now_us().wrapping_sub(start) > UART_IDLE_TIMEOUT_US {
            crate::logln!(
                "[RP1PHASE16:{}] uart0_idle ok=false FR=0x{:08x} timing_us={}",
                label,
                fr,
                now_us().wrapping_sub(start),
            );
            return false;
        }
    }
}

#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
fn capture_interior_anchors(bar1: usize) -> Option<InteriorAnchors> {
    let uart0 = unsafe { bar1_abort_probe((bar1 + INTERIOR_UART0_PID0_OFFSET) as *const u32) };
    let uart4 = unsafe { bar1_abort_probe((bar1 + INTERIOR_UART4_PID0_OFFSET) as *const u32) };
    if uart0.expected_abort
        || uart4.expected_abort
        || uart0.value == u32::MAX
        || uart4.value == u32::MAX
    {
        None
    } else {
        Some(InteriorAnchors {
            uart0: uart0.value,
            uart4: uart4.value,
        })
    }
}

#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
fn record_interior_probe(
    bar1: usize,
    bar2: usize,
    counts: &mut InteriorProbeCounts,
    phase_before: u32,
    post_completion: bool,
    anchors: &InteriorAnchors,
) {
    let uart0 = unsafe { bar1_abort_probe((bar1 + INTERIOR_UART0_PID0_OFFSET) as *const u32) };
    let chip = unsafe { bar1_abort_probe((bar1 + CHIP_ID_OFFSET) as *const u32) };
    let uart4 = unsafe { bar1_abort_probe((bar1 + INTERIOR_UART4_PID0_OFFSET) as *const u32) };
    let monitor2 = unsafe { bar1_abort_probe((bar1 + CONTROL_OFFSET) as *const u32) };
    let bar2_valid = unsafe {
        read_record_word(bar2, W_MAGIC) == RECORD_MAGIC
            && read_record_word(bar2, W_VERSION) == RECORD_VERSION
            && read_record_word(bar2, W_RECORD_SIZE) as usize == RECORD_BYTES
    };
    let phase_after = unsafe { read_record_word(bar2, W_BLOCK_PHASE) };
    counts.record(
        phase_before,
        phase_after,
        post_completion,
        &uart0,
        &chip,
        &uart4,
        &monitor2,
        bar2_valid,
        anchors,
    );
}

#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
fn interior_hole_record_gate(bar2: usize, seq: u32) -> bool {
    let flags = unsafe { read_record_word(bar2, W_FLAGS) };
    let protect_flags = unsafe { read_record_word(bar2, W_PROTECT_FLAGS) };
    let started = unsafe {
        u64::from(read_record_word(bar2, W_STARTED_US_LO))
            | (u64::from(read_record_word(bar2, W_STARTED_US_HI)) << 32)
    };
    let ended = unsafe {
        u64::from(read_record_word(bar2, W_ENDED_US_LO))
            | (u64::from(read_record_word(bar2, W_ENDED_US_HI)) << 32)
    };
    let elapsed = unsafe { read_record_word(bar2, W_ELAPSED_US) };
    let enable_us = unsafe { read_record_word(bar2, W_PROTECT_ENABLE_US) };
    let disable_us = unsafe { read_record_word(bar2, W_PROTECT_DISABLE_US) };
    let restore_us = unsafe { read_record_word(bar2, W_PROTECT_RESTORE_US) };
    let timing_ok = started != 0
        && ended >= started
        && elapsed != 0
        && enable_us != 0
        && enable_us <= disable_us
        && disable_us <= restore_us
        && restore_us <= elapsed;
    let ok = validate_record_header(bar2)
        && validate_checksum(bar2)
        && unsafe { read_record_word(bar2, W_ACK) } == seq
        && unsafe { read_record_word(bar2, W_GO) } == seq
        && unsafe { read_record_word(bar2, W_MODE) } == MODE_INTERIOR_HOLE_64K
        && unsafe { read_record_word(bar2, W_PHASE) } == PHASE_DONE
        && unsafe { read_record_word(bar2, W_COMPLETION) } == COMPLETION_DONE
        && unsafe { read_record_word(bar2, W_COMPLETION_SEQ) } == seq
        && unsafe { read_record_word(bar2, W_RESULT) } == 1
        && unsafe { read_record_word(bar2, W_BLOCK_PHASE) } == BLOCK_PHASE_RESTORED
        && flags == INTERIOR_REQUIRED_FLAGS
        && protect_flags == INTERIOR_REQUIRED_FLAGS
        && unsafe { read_record_word(bar2, W_TARGET_PAGE_OFFSET) } == INTERIOR_HOLE_OFFSET
        && unsafe { read_record_word(bar2, W_PROTECT_BAR1_BUS_BASE) } == 0
        && unsafe { read_record_word(bar2, W_DUMMY_LOCAL_BASE) }
            == 0x4000_0000u32.wrapping_add(INTERIOR_TARGET_PID0_OFFSET as u32)
        && unsafe { read_record_word(bar2, W_CTRL2_BEFORE) } == 0xc000_0100
        && unsafe { read_record_word(bar2, W_CTRL2_BLOCK_VALUE) } == 0x4000_0100
        && unsafe { read_record_word(bar2, W_CTRL2_BLOCK_READBACK) } == 0x4000_0100
        && unsafe { read_record_word(bar2, W_CTRL2_RESTORE_READBACK) } == 0xc000_0100
        && unsafe { read_record_word(bar2, W_SELECTOR_RESTORE_READBACK) }
            == unsafe { read_record_word(bar2, W_SELECTOR_SAVED) }
        && unsafe { read_record_word(bar2, W_TARGET_BEFORE) } == EXPECTED_CHIP_ID
        && unsafe { read_record_word(bar2, W_TARGET_DURING) } == EXPECTED_CHIP_ID
        && unsafe { read_record_word(bar2, W_TARGET_AFTER) } == EXPECTED_CHIP_ID
        && control_value_valid(unsafe { read_record_word(bar2, W_CONTROL_BEFORE) })
        && control_value_valid(unsafe { read_record_word(bar2, W_CONTROL_DURING) })
        && control_value_valid(unsafe { read_record_word(bar2, W_CONTROL_AFTER) })
        && unsafe { read_record_word(bar2, W_BAR0) } == 0x0080_0000
        && unsafe { read_record_word(bar2, W_BAR1) } == 0
        && unsafe { read_record_word(bar2, W_BAR2) } == 0x0040_0000
        && unsafe { read_record_word(bar2, W_BAR1_BUS_BASE) } == 0
        && unsafe { read_record_word(bar2, W_COMMAND) } & 0x2 != 0
        && read_iatu_snapshot_words(bar2, W_ORIGINAL_BAR1) == INTERIOR_ORIGINAL_BAR1
        && read_iatu_snapshot_words(bar2, W_SPARE_BEFORE) == INTERIOR_A3_UNUSED
        && read_iatu_snapshot_words(bar2, W_SPARE_BEFORE + IATU_SNAPSHOT_WORDS)
            == INTERIOR_E3_UNUSED
        && read_iatu_snapshot_words(bar2, W_SPARE_PROGRAMMED) == INTERIOR_A3_UPPER
        && read_iatu_snapshot_words(bar2, W_SPARE_PROGRAMMED + IATU_SNAPSHOT_WORDS)
            == INTERIOR_E3_LOWER
        && read_iatu_snapshot_words(bar2, W_SPARE_READBACK) == INTERIOR_A3_UPPER
        && read_iatu_snapshot_words(bar2, W_SPARE_READBACK + IATU_SNAPSHOT_WORDS)
            == INTERIOR_E3_LOWER
        && read_iatu_snapshot_words(bar2, W_SPARE_RESTORED) == INTERIOR_A3_UNUSED
        && read_iatu_snapshot_words(bar2, W_SPARE_RESTORED + IATU_SNAPSHOT_WORDS)
            == INTERIOR_E3_UNUSED
        && timing_ok;
    crate::logln!(
        "[RP1PHASE14] record_gate ok={} flags=0x{:08x} protect_flags=0x{:08x} timing={}/{}/{} elapsed={}",
        ok,
        flags,
        protect_flags,
        enable_us,
        disable_us,
        restore_us,
        elapsed,
    );
    ok
}

#[cfg(feature = "rp1-bar1-4k-protection-proof")]
fn run_hole_mode(
    bar1: usize,
    bar2: usize,
    seq: &mut u32,
    mode: u32,
    hole_offset: u32,
    name: &'static str,
    success_verdict: &'static str,
    failure_verdict: &'static str,
) -> bool {
    crate::logln!("[RP1PROTECT:{}] begin seq={}", name, *seq);
    let Some(vector) = install_abort_vector() else {
        crate::logln!(
            "[RP1PROTECT:{}] skip: vector install precondition failed",
            name
        );
        return false;
    };
    let target_pre = unsafe { bar1_abort_probe((bar1 + CHIP_ID_OFFSET) as *const u32) };
    let control_pre = unsafe { read32(bar1 + CONTROL_OFFSET) };
    let canary_pre = unsafe { read_record_word(bar2, W_MAGIC) };
    if target_pre.expected_abort
        || target_pre.value != EXPECTED_CHIP_ID
        || !control_value_valid(control_pre)
        || canary_pre != RECORD_MAGIC
    {
        crate::logln!(
            "[RP1PROTECT:{}] skip: host precondition target=0x{:08x}/abort={} control=0x{:08x} canary=0x{:08x}",
            name,
            target_pre.value,
            target_pre.expected_abort,
            control_pre,
            canary_pre
        );
        restore_abort_vector(vector);
        return false;
    }
    if !send_mode_and_wait_ack(bar2, *seq, mode, name) {
        restore_abort_vector(vector);
        return false;
    }
    if !validate_dummy_page_from_bar2(bar2, *seq) {
        crate::logln!(
            "[RP1PROTECT:{}] host dummy gate failed; forcing protocol reject",
            name
        );
        unsafe {
            write_record_word(bar2, W_MODE, 0);
            write_record_word(bar2, W_GO, *seq);
        }
        let _ = wait_rp1_done(bar2, *seq, name);
        restore_abort_vector(vector);
        dump_record_summary(name, bar2);
        dump_protection_summary(name, bar2);
        *seq = seq.wrapping_add(1);
        return false;
    }

    unsafe {
        arm_expected_abort(mode);
        write_record_word(bar2, W_GO, *seq);
    }
    let mut counts = ProtectionProbeCounts::default();
    for _ in 0..TRAFFIC_OPS {
        let phase_before = unsafe { read_record_word(bar2, W_BLOCK_PHASE) };
        unsafe { set_expected_abort_phase(phase_before) };
        let event_us = unsafe { read_record_word(bar2, W_PROTECT_DISABLE_US) };
        let control = unsafe { read32(bar1 + CONTROL_OFFSET) };
        let canary = unsafe { read_record_word(bar2, W_MAGIC) };
        if phase_before != BLOCK_PHASE_DISABLED {
            let phase_after = unsafe { read_record_word(bar2, W_BLOCK_PHASE) };
            counts.record_unprobed(phase_before, phase_after, event_us, control, canary);
            busy_wait_us(12);
            continue;
        }
        let target = unsafe { bar1_abort_probe((bar1 + CHIP_ID_OFFSET) as *const u32) };
        let phase_after = unsafe { read_record_word(bar2, W_BLOCK_PHASE) };
        counts.record(
            phase_before,
            phase_after,
            event_us,
            &target,
            control,
            canary,
        );
        busy_wait_us(12);
    }
    unsafe {
        write_record_word(bar2, W_ARG0, TRAFFIC_OPS as u32);
        write_record_word(bar2, W_ARG1, counts.xor);
    }
    let completion_ok = wait_rp1_done(bar2, *seq, name);
    let phase = unsafe { read_record_word(bar2, W_BLOCK_PHASE) };
    let post_target = if phase == BLOCK_PHASE_RESTORED {
        unsafe {
            set_expected_abort_phase(phase);
            bar1_abort_probe((bar1 + CHIP_ID_OFFSET) as *const u32)
        }
    } else {
        Bar1Probe {
            value: 0,
            expected_abort: true,
        }
    };
    unsafe { disarm_expected_abort() };
    restore_abort_vector(vector);
    let post_control = unsafe { read32(bar1 + CONTROL_OFFSET) };
    let post_canary = unsafe { read_record_word(bar2, W_MAGIC) };
    let restored = phase == BLOCK_PHASE_RESTORED
        && !post_target.expected_abort
        && protection_restore_gate(
            bar2,
            completion_ok,
            post_target.value,
            post_control,
            post_canary,
        );

    log_protection_probe_counts(name, &counts, post_target.value, post_target.expected_abort);
    dump_record_summary(name, bar2);
    dump_protection_summary(name, bar2);
    *seq = seq.wrapping_add(1);

    let during = &counts.during;
    let hole_proven = restored
        && hole_snapshot_gate(bar2, hole_offset)
        && during.stable != 0
        && during.target.expected == 0
        && during.target.dummy == 0
        && during.target.other == 0
        && during.target.all_ff.wrapping_add(during.target.abort) == during.stable
        && during.target.all_ff.wrapping_add(during.target.abort) != 0
        && during.control.valid == during.stable
        && counts.canary_bad == 0;
    if hole_proven {
        crate::logln!("[RP1PROTECT] verdict={}", success_verdict);
    } else {
        crate::logln!("[RP1PROTECT] verdict={}", failure_verdict);
    }
    hole_proven
}

#[cfg(feature = "rp1-bar1-4k-protection-proof")]
fn validate_dummy_page_from_bar2(bar2: usize, seq: u32) -> bool {
    const SRAM_BASE: usize = 0x2000_0000;
    const DUMMY_LIMIT: usize = 0x2000_f800;
    let dummy_base = unsafe { read_record_word(bar2, W_DUMMY_LOCAL_BASE) } as usize;
    let target_offset = unsafe { read_record_word(bar2, W_TARGET_PAGE_OFFSET) };
    let Some(dummy_end) = dummy_base.checked_add(0x1000) else {
        return false;
    };
    if target_offset != 0
        || dummy_base < SRAM_BASE
        || dummy_base & 0xfff != 0
        || dummy_end > DUMMY_LIMIT
        || dummy_end.wrapping_sub(SRAM_BASE) > EXPECTED_BAR2_SIZE as usize
    {
        crate::logln!(
            "[RP1PROTECT] dummy bounds invalid target_offset=0x{:08x} base=0x{:08x} end=0x{:08x}",
            target_offset,
            dummy_base,
            dummy_end
        );
        return false;
    }

    let host_page = bar2 + dummy_base - SRAM_BASE;
    let mut checksum = CHECKSUM_SEED;
    let mut mismatches = 0u32;
    let mut first_bad_index = u32::MAX;
    let mut first_bad_actual = 0u32;
    let mut stored_checksum = 0u32;
    for index in 0..1024usize {
        let actual = unsafe { read32(host_page + index * 4) };
        let expected = match index {
            0 => DUMMY_MAGIC,
            1 => seq,
            2 => !seq,
            3 => 0,
            _ => {
                (dummy_base as u32)
                    .wrapping_add((index as u32).wrapping_mul(4))
                    .rotate_left(7)
                    ^ DUMMY_CANARY_XOR
            }
        };
        if index == 3 {
            stored_checksum = actual;
        } else if actual != expected {
            mismatches = mismatches.wrapping_add(1);
            if first_bad_index == u32::MAX {
                first_bad_index = index as u32;
                first_bad_actual = actual;
            }
        }
        checksum = (checksum ^ expected)
            .rotate_left(5)
            .wrapping_mul(0x9e37_79b1);
    }
    let ok = mismatches == 0 && stored_checksum == checksum;
    crate::logln!(
        "[RP1PROTECT] dummy BAR2 check ok={} base=0x{:08x} mismatches={} first_bad={}/0x{:08x} checksum=0x{:08x}/0x{:08x}",
        ok,
        dummy_base,
        mismatches,
        first_bad_index,
        first_bad_actual,
        stored_checksum,
        checksum
    );
    ok
}

#[cfg(feature = "rp1-bar1-4k-protection-proof")]
fn control_value_valid(value: u32) -> bool {
    value != u32::MAX && value & CONTROL_LINK_MASK == CONTROL_LINK_MASK
}

#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
fn read_iatu_snapshot_words(bar2: usize, offset: usize) -> [u32; IATU_SNAPSHOT_WORDS] {
    let mut words = [0u32; IATU_SNAPSHOT_WORDS];
    let mut index = 0;
    while index < IATU_SNAPSHOT_WORDS {
        words[index] = unsafe { read_record_word(bar2, offset + index) };
        index += 1;
    }
    words
}

#[cfg(feature = "rp1-bar1-4k-protection-proof")]
fn hole_snapshot_gate(bar2: usize, hole_offset: u32) -> bool {
    if hole_offset != 0x1000 && hole_offset != 0x1_0000 {
        return false;
    }
    let original = [
        0x23,
        0,
        0xC000_0100,
        0,
        0,
        0x0000_FFFF,
        0x4000_0000,
        0xC0,
        0,
    ];
    let spare0 = [0xA3, 0, 0, 0, 0, 0x0000_FFFF, 0, 0, 0];
    let spare1 = [0xE3, 0, 0, 0, 0, 0x0000_FFFF, 0, 0, 0];
    let expected = [
        0xA3,
        0,
        0x8000_0000,
        hole_offset,
        0,
        0x003F_FFFF,
        0x4000_0000u32.wrapping_add(hole_offset),
        0xC0,
        0,
    ];
    let ok = read_iatu_snapshot_words(bar2, W_ORIGINAL_BAR1) == original
        && read_iatu_snapshot_words(bar2, W_SPARE_BEFORE) == spare0
        && read_iatu_snapshot_words(bar2, W_SPARE_BEFORE + IATU_SNAPSHOT_WORDS) == spare1
        && read_iatu_snapshot_words(bar2, W_SPARE_PROGRAMMED) == expected
        && read_iatu_snapshot_words(bar2, W_SPARE_PROGRAMMED + IATU_SNAPSHOT_WORDS) == spare1
        && read_iatu_snapshot_words(bar2, W_SPARE_READBACK) == expected
        && read_iatu_snapshot_words(bar2, W_SPARE_READBACK + IATU_SNAPSHOT_WORDS) == spare1
        && read_iatu_snapshot_words(bar2, W_SPARE_RESTORED) == spare0
        && read_iatu_snapshot_words(bar2, W_SPARE_RESTORED + IATU_SNAPSHOT_WORDS) == spare1;
    crate::logln!(
        "[RP1PROTECT] hole_snapshot_gate offset=0x{:08x} ok={}",
        hole_offset,
        ok
    );
    ok
}

#[cfg(feature = "rp1-bar1-4k-protection-proof")]
fn redirect_program_granularity_rejected_clean(
    bar2: usize,
    seq: u32,
    completion_ok: bool,
    post_target: u32,
    post_control: u32,
    post_canary: u32,
) -> bool {
    const SAFE_REJECT_FLAGS: u32 = PROTECT_FLAG_DBI_BARS_VALID
        | PROTECT_FLAG_ORIGINAL_BAR1_VALID
        | PROTECT_FLAG_SPARES_UNUSED
        | PROTECT_FLAG_DUMMY_VALID
        | PROTECT_FLAG_SPARE_DISABLED
        | PROTECT_FLAG_SPARE_RESTORED
        | PROTECT_FLAG_SELECTOR_RESTORED
        | PROTECT_FLAG_ORIGINAL_RESTORED;
    let flags = unsafe { read_record_word(bar2, W_PROTECT_FLAGS) };
    let dummy = unsafe { read_record_word(bar2, W_DUMMY_LOCAL_BASE) };
    let programmed0 = [0xA3, 0, 0x8000_0000, 0, 0, 0x0000_0FFF, dummy, 0xC0, 0];
    let rounded0 = [
        0xA3,
        0,
        0x8000_0000,
        0,
        0,
        0x0000_FFFF,
        dummy & !0xFFFF,
        0xC0,
        0,
    ];
    let spare0 = [0xA3, 0, 0, 0, 0, 0x0000_FFFF, 0, 0, 0];
    let spare1 = [0xE3, 0, 0, 0, 0, 0x0000_FFFF, 0, 0, 0];
    let enable_us = unsafe { read_record_word(bar2, W_PROTECT_ENABLE_US) };
    let disable_us = unsafe { read_record_word(bar2, W_PROTECT_DISABLE_US) };
    let restore_us = unsafe { read_record_word(bar2, W_PROTECT_RESTORE_US) };
    let ok = !completion_ok
        && validate_checksum(bar2)
        && unsafe { read_record_word(bar2, W_ACK) } == seq
        && unsafe { read_record_word(bar2, W_GO) } == seq
        && unsafe { read_record_word(bar2, W_MODE) } == MODE_REDIRECT_4K
        && unsafe { read_record_word(bar2, W_COMPLETION) } == COMPLETION_PRECONDITION
        && unsafe { read_record_word(bar2, W_COMPLETION_SEQ) } == seq
        && unsafe { read_record_word(bar2, W_BLOCK_PHASE) } == BLOCK_PHASE_PRECONDITION_FAIL
        && flags == SAFE_REJECT_FLAGS
        && enable_us == 0
        && disable_us > 0
        && disable_us <= restore_us
        && unsafe { read_record_word(bar2, W_TARGET_BEFORE) } == EXPECTED_CHIP_ID
        && unsafe { read_record_word(bar2, W_TARGET_DURING) } == 0
        && unsafe { read_record_word(bar2, W_TARGET_AFTER) } == EXPECTED_CHIP_ID
        && control_value_valid(unsafe { read_record_word(bar2, W_CONTROL_BEFORE) })
        && unsafe { read_record_word(bar2, W_CONTROL_DURING) } == 0
        && control_value_valid(unsafe { read_record_word(bar2, W_CONTROL_AFTER) })
        && post_target == EXPECTED_CHIP_ID
        && control_value_valid(post_control)
        && post_canary == RECORD_MAGIC
        && read_iatu_snapshot_words(bar2, W_SPARE_BEFORE) == spare0
        && read_iatu_snapshot_words(bar2, W_SPARE_BEFORE + IATU_SNAPSHOT_WORDS) == spare1
        && read_iatu_snapshot_words(bar2, W_SPARE_PROGRAMMED) == programmed0
        && read_iatu_snapshot_words(bar2, W_SPARE_PROGRAMMED + IATU_SNAPSHOT_WORDS) == spare1
        && read_iatu_snapshot_words(bar2, W_SPARE_READBACK) == rounded0
        && read_iatu_snapshot_words(bar2, W_SPARE_READBACK + IATU_SNAPSHOT_WORDS) == spare1
        && read_iatu_snapshot_words(bar2, W_SPARE_RESTORED) == spare0
        && read_iatu_snapshot_words(bar2, W_SPARE_RESTORED + IATU_SNAPSHOT_WORDS) == spare1;
    crate::logln!(
        "[RP1PROTECT] rounded_program_gate ok={} flags=0x{:08x} enable/disable/restore={}/{}/{} dummy=0x{:08x}",
        ok,
        flags,
        enable_us,
        disable_us,
        restore_us,
        dummy
    );
    ok
}

#[cfg(feature = "rp1-bar1-4k-protection-proof")]
fn protection_restore_gate(
    bar2: usize,
    completion_ok: bool,
    post_target: u32,
    post_control: u32,
    post_canary: u32,
) -> bool {
    let flags = unsafe { read_record_word(bar2, W_PROTECT_FLAGS) };
    let phase = unsafe { read_record_word(bar2, W_BLOCK_PHASE) };
    let ok = completion_ok
        && phase == BLOCK_PHASE_RESTORED
        && flags & REQUIRED_PROTECT_FLAGS == REQUIRED_PROTECT_FLAGS
        && post_target == EXPECTED_CHIP_ID
        && control_value_valid(post_control)
        && post_canary == RECORD_MAGIC;
    crate::logln!(
        "[RP1PROTECT] restore_gate ok={} completion={} phase={} flags=0x{:08x}/0x{:08x} target=0x{:08x} control=0x{:08x} bar2=0x{:08x}",
        ok,
        completion_ok,
        phase,
        flags,
        REQUIRED_PROTECT_FLAGS,
        post_target,
        post_control,
        post_canary
    );
    ok
}

fn run_bar2_read_traffic(bar2: usize) -> (u32, u32) {
    let scratch = bar2 + RECORD_OFFSET + W_SCRATCH * 4;
    let mut xor = 0u32;
    for i in 0..TRAFFIC_OPS {
        let off = (i & 0x3) * 4;
        xor ^= unsafe { read32(scratch + off) };
    }
    crate::logln!(
        "[RP1INBOUND:BAR2_READ] ops={} xor=0x{:08x}",
        TRAFFIC_OPS,
        xor
    );
    (TRAFFIC_OPS as u32, xor)
}

fn run_bar2_write_traffic(bar2: usize) -> (u32, u32) {
    let scratch = bar2 + RECORD_OFFSET + W_SCRATCH * 4;
    let mut saved = [0u32; 4];
    for (index, slot) in saved.iter_mut().enumerate() {
        *slot = unsafe { read32(scratch + index * 4) };
    }
    let mut xor = 0u32;
    for i in 0..TRAFFIC_OPS {
        let index = i & 0x3;
        let value = 0x4950_4d31u32 ^ i as u32 ^ ((index as u32) << 24);
        unsafe { write32(scratch + index * 4, value) };
        xor ^= value;
    }
    for (index, value) in saved.iter().copied().enumerate() {
        unsafe { write32(scratch + index * 4, value) };
    }
    crate::logln!(
        "[RP1INBOUND:BAR2_WRITE] ops={} xor=0x{:08x} restored=4",
        TRAFFIC_OPS,
        xor
    );
    (TRAFFIC_OPS as u32, xor)
}

fn run_bar1_read_traffic(bar1: usize) -> (u32, u32) {
    let mut xor = 0u32;
    for _ in 0..TRAFFIC_OPS {
        xor ^= unsafe { read32(bar1 + CHIP_ID_OFFSET) };
    }
    crate::logln!(
        "[RP1INBOUND:BAR1_READ] ops={} xor=0x{:08x}",
        TRAFFIC_OPS,
        xor
    );
    (TRAFFIC_OPS as u32, xor)
}

fn wait_until_sample_window_clear(go_us: u64) {
    while now_us().wrapping_sub(go_us) < SAMPLE_QUIET_US {
        core::hint::spin_loop();
    }
}

fn wait_rp1_done(bar2: usize, seq: u32, name: &'static str) -> bool {
    let start = now_us();
    while now_us().wrapping_sub(start) <= MODE_TIMEOUT_US {
        let completion_seq = unsafe { read_record_word(bar2, W_COMPLETION_SEQ) };
        let completion = unsafe { read_record_word(bar2, W_COMPLETION) };
        let phase = unsafe { read_record_word(bar2, W_PHASE) };
        if completion_seq == seq {
            crate::logln!(
                "[RP1INBOUND:{}] RP1 completion seq={} completion=0x{:08x} phase={}",
                name,
                seq,
                completion,
                phase
            );
            return phase == PHASE_DONE
                && validate_checksum(bar2)
                && matches!(completion, COMPLETION_IDLE | COMPLETION_DONE);
        }
        crate::timer::delay_micros(500);
    }
    crate::logln!("[RP1INBOUND:{}] timeout waiting RP1 seq={}", name, seq);
    false
}

fn validate_checksum(bar2: usize) -> bool {
    let expected = checksum_record(bar2);
    let actual = unsafe { read_record_word(bar2, W_CHECKSUM) };
    let ok = expected == actual;
    crate::logln!(
        "[RP1INBOUND] checksum {} expected=0x{:08x} actual=0x{:08x}",
        if ok { "ok" } else { "bad" },
        expected,
        actual
    );
    ok
}

fn checksum_record(bar2: usize) -> u32 {
    let mut checksum = CHECKSUM_SEED;
    for index in 0..CHECKSUM_WORDS {
        let word = if matches!(index, W_COMPLETION_SEQ | W_CHECKSUM | W_ARG0 | W_ARG1) {
            0
        } else {
            unsafe { read_record_word(bar2, index) }
        };
        checksum = (checksum ^ word).rotate_left(5).wrapping_mul(0x9e37_79b1);
    }
    checksum
}

fn monitor_health_allows_block(bar2: usize) -> bool {
    let health = unsafe { read_record_word(bar2, W_HEALTH_FLAGS) };
    let config_change_count = unsafe { read_record_word(bar2, W_CONFIG_CHANGE_COUNT) };
    let overflow = unsafe { read_record_word(bar2, W_OVERFLOW_COUNT) };
    let monitor2_bit_counts = unsafe {
        read_record_word(bar2, W_MONITOR2_BIT23_COUNT)
            .wrapping_add(read_record_word(bar2, W_MONITOR2_BIT22_COUNT))
            .wrapping_add(read_record_word(bar2, W_MONITOR2_BIT21_COUNT))
    };
    let mut axishim_status_counts = 0u32;
    for channel in 0..CHANNEL_COUNT {
        let status = W_AXISHIM_STATUS + channel * AXISHIM_STATUS_WORDS;
        axishim_status_counts =
            axishim_status_counts.wrapping_add(unsafe { read_record_word(bar2, status + 2) });
    }
    let checksum_ok = validate_checksum(bar2);
    let required = HEALTH_PCIE_MONITOR_CAPTURED
        | HEALTH_AXISHIM_CFG_UNCHANGED
        | HEALTH_SAMPLED
        | HEALTH_NO_OVERFLOW
        | HEALTH_SCRATCH_RESTORED;
    crate::logln!(
        "[RP1INBOUND:BLOCK_BAR1] health_gate health=0x{:08x} required=0x{:08x} axishim_config_changes={} overflow={} monitor2_bit_counts={} axishim_status_counts={} checksum_ok={}",
        health,
        required,
        config_change_count,
        overflow,
        monitor2_bit_counts,
        axishim_status_counts,
        checksum_ok
    );
    health & required == required
        && config_change_count == 0
        && overflow == 0
        && monitor2_bit_counts != 0
        && checksum_ok
}

#[derive(Default)]
struct ProbeClassCounts {
    valid: u32,
    all_ff: u32,
    abort: u32,
}

#[derive(Default)]
struct BlockProbeCounts {
    before: ProbeClassCounts,
    during: ProbeClassCounts,
    after: ProbeClassCounts,
    canary_bad: u32,
}

impl BlockProbeCounts {
    fn record(&mut self, block_phase: u32, probe: &Bar1Probe) {
        let counts = if block_phase < BLOCK_PHASE_DISABLED {
            &mut self.before
        } else if block_phase == BLOCK_PHASE_DISABLED {
            &mut self.during
        } else {
            &mut self.after
        };
        if probe.expected_abort {
            counts.abort = counts.abort.wrapping_add(1);
        } else if probe.value == u32::MAX {
            counts.all_ff = counts.all_ff.wrapping_add(1);
        } else if probe.value == EXPECTED_CHIP_ID {
            counts.valid = counts.valid.wrapping_add(1);
        }
    }
}

#[cfg(feature = "rp1-bar1-4k-protection-proof")]
#[derive(Default)]
struct ProtectionTargetCounts {
    expected: u32,
    dummy: u32,
    all_ff: u32,
    other: u32,
    abort: u32,
}

#[cfg(feature = "rp1-bar1-4k-protection-proof")]
impl ProtectionTargetCounts {
    fn record(&mut self, probe: &Bar1Probe) {
        if probe.expected_abort {
            self.abort = self.abort.wrapping_add(1);
        } else if probe.value == DUMMY_MAGIC {
            self.dummy = self.dummy.wrapping_add(1);
        } else if probe.value == EXPECTED_CHIP_ID {
            self.expected = self.expected.wrapping_add(1);
        } else if probe.value == u32::MAX {
            self.all_ff = self.all_ff.wrapping_add(1);
        } else {
            self.other = self.other.wrapping_add(1);
        }
    }
}

#[cfg(feature = "rp1-bar1-4k-protection-proof")]
#[derive(Default)]
struct ProtectionControlCounts {
    valid: u32,
    all_ff: u32,
    other: u32,
}

#[cfg(feature = "rp1-bar1-4k-protection-proof")]
impl ProtectionControlCounts {
    fn record(&mut self, value: u32) {
        if control_value_valid(value) {
            self.valid = self.valid.wrapping_add(1);
        } else if value == u32::MAX {
            self.all_ff = self.all_ff.wrapping_add(1);
        } else {
            self.other = self.other.wrapping_add(1);
        }
    }
}

#[cfg(feature = "rp1-bar1-4k-protection-proof")]
#[derive(Default)]
struct ProtectionPhaseCounts {
    stable: u32,
    target: ProtectionTargetCounts,
    control: ProtectionControlCounts,
}

#[cfg(feature = "rp1-bar1-4k-protection-proof")]
impl ProtectionPhaseCounts {
    fn record(&mut self, target: &Bar1Probe, control: u32) {
        self.stable = self.stable.wrapping_add(1);
        self.target.record(target);
        self.control.record(control);
    }
}

#[cfg(feature = "rp1-bar1-4k-protection-proof")]
#[derive(Default)]
struct ProtectionProbeCounts {
    before: ProtectionPhaseCounts,
    during: ProtectionPhaseCounts,
    after: ProtectionPhaseCounts,
    boundary: u32,
    unprobed: u32,
    canary_bad: u32,
    event_us_or: u32,
    xor: u32,
}

#[cfg(feature = "rp1-bar1-4k-protection-proof")]
impl ProtectionProbeCounts {
    fn record_unprobed(
        &mut self,
        phase_before: u32,
        phase_after: u32,
        event_us: u32,
        control: u32,
        canary: u32,
    ) {
        self.xor ^= phase_before ^ phase_after ^ event_us ^ control ^ canary;
        self.event_us_or |= event_us;
        if canary != RECORD_MAGIC {
            self.canary_bad = self.canary_bad.wrapping_add(1);
        }
        if phase_before != phase_after {
            self.boundary = self.boundary.wrapping_add(1);
        } else {
            self.unprobed = self.unprobed.wrapping_add(1);
        }
    }

    fn record(
        &mut self,
        phase_before: u32,
        phase_after: u32,
        event_us: u32,
        target: &Bar1Probe,
        control: u32,
        canary: u32,
    ) {
        self.xor ^= phase_before ^ phase_after ^ event_us ^ target.value ^ control ^ canary;
        self.event_us_or |= event_us;
        if canary != RECORD_MAGIC {
            self.canary_bad = self.canary_bad.wrapping_add(1);
        }
        if phase_before != phase_after {
            self.boundary = self.boundary.wrapping_add(1);
            return;
        }
        let counts = if phase_before < BLOCK_PHASE_DISABLED {
            &mut self.before
        } else if phase_before == BLOCK_PHASE_DISABLED {
            &mut self.during
        } else {
            &mut self.after
        };
        counts.record(target, control);
    }
}

#[cfg(feature = "rp1-bar1-4k-protection-proof")]
fn log_protection_probe_counts(
    name: &'static str,
    counts: &ProtectionProbeCounts,
    post_value: u32,
    post_abort: bool,
) {
    for (phase, values) in [
        ("before", &counts.before),
        ("during", &counts.during),
        ("after", &counts.after),
    ] {
        crate::logln!(
            "[RP1PROTECT:{}] {} stable={} target expected/dummy/allff/other/abort={}/{}/{}/{}/{} control valid/allff/other={}/{}/{}",
            name,
            phase,
            values.stable,
            values.target.expected,
            values.target.dummy,
            values.target.all_ff,
            values.target.other,
            values.target.abort,
            values.control.valid,
            values.control.all_ff,
            values.control.other
        );
    }
    crate::logln!(
        "[RP1PROTECT:{}] boundary={} bar2_bad={} unprobed={} event_us_or={} xor=0x{:08x} post=0x{:08x}/abort={}",
        name,
        counts.boundary,
        counts.canary_bad,
        counts.unprobed,
        counts.event_us_or,
        counts.xor,
        post_value,
        post_abort
    );
}

#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
#[derive(Clone, Copy)]
struct InteriorAnchors {
    uart0: u32,
    uart4: u32,
}

#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
#[derive(Default)]
struct InteriorValueCounts {
    expected: u32,
    all_ff: u32,
    other: u32,
    abort: u32,
    raw_seen: u32,
    raw_first: u32,
    raw_last: u32,
    raw_or: u32,
    raw_and: u32,
    raw_xor: u32,
}

#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
impl InteriorValueCounts {
    fn record_raw(&mut self, value: u32) {
        if self.raw_seen == 0 {
            self.raw_first = value;
            self.raw_and = value;
        } else {
            self.raw_and &= value;
        }
        self.raw_seen = self.raw_seen.wrapping_add(1);
        self.raw_last = value;
        self.raw_or |= value;
        self.raw_xor ^= value;
    }

    fn record_probe(&mut self, probe: &Bar1Probe, expected: u32) {
        if probe.expected_abort {
            self.abort = self.abort.wrapping_add(1);
        } else {
            self.record_raw(probe.value);
            if probe.value == u32::MAX {
                self.all_ff = self.all_ff.wrapping_add(1);
            } else if probe.value == expected {
                self.expected = self.expected.wrapping_add(1);
            } else {
                self.other = self.other.wrapping_add(1);
            }
        }
    }

    fn record_monitor2(&mut self, probe: &Bar1Probe) {
        if probe.expected_abort {
            self.abort = self.abort.wrapping_add(1);
        } else {
            self.record_raw(probe.value);
            if probe.value == u32::MAX {
                self.all_ff = self.all_ff.wrapping_add(1);
            } else if control_value_valid(probe.value) {
                self.expected = self.expected.wrapping_add(1);
            } else {
                self.other = self.other.wrapping_add(1);
            }
        }
    }

    fn record_bool(&mut self, ok: bool) {
        self.record_raw(u32::from(ok));
        if ok {
            self.expected = self.expected.wrapping_add(1);
        } else {
            self.other = self.other.wrapping_add(1);
        }
    }

    fn all_expected(&self, samples: u32) -> bool {
        samples != 0
            && self.expected == samples
            && self.all_ff == 0
            && self.other == 0
            && self.abort == 0
    }

    fn all_all_ff(&self, samples: u32) -> bool {
        samples != 0
            && self.expected == 0
            && self.all_ff == samples
            && self.other == 0
            && self.abort == 0
    }
}

#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
#[derive(Default)]
struct InteriorPhaseCounts {
    samples: u32,
    uart0: InteriorValueCounts,
    chip: InteriorValueCounts,
    uart4: InteriorValueCounts,
    monitor2: InteriorValueCounts,
    bar2: InteriorValueCounts,
}

#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
impl InteriorPhaseCounts {
    fn record(
        &mut self,
        uart0: &Bar1Probe,
        chip: &Bar1Probe,
        uart4: &Bar1Probe,
        monitor2: &Bar1Probe,
        bar2_valid: bool,
        anchors: &InteriorAnchors,
    ) {
        self.samples = self.samples.wrapping_add(1);
        self.uart0.record_probe(uart0, anchors.uart0);
        self.chip.record_probe(chip, EXPECTED_CHIP_ID);
        self.uart4.record_probe(uart4, anchors.uart4);
        self.monitor2.record_monitor2(monitor2);
        self.bar2.record_bool(bar2_valid);
    }

    fn gate_expected(&self) -> bool {
        self.uart0.all_expected(self.samples)
            && self.chip.all_expected(self.samples)
            && self.uart4.all_expected(self.samples)
            && self.monitor2.all_expected(self.samples)
            && self.bar2.all_expected(self.samples)
    }

    fn gate_during(&self) -> bool {
        let uart_target_ok = if INTERIOR_TARGET_UART4 {
            self.uart0.all_expected(self.samples) && self.uart4.all_all_ff(self.samples)
        } else {
            self.uart0.all_all_ff(self.samples) && self.uart4.all_expected(self.samples)
        };
        uart_target_ok
            && self.chip.all_expected(self.samples)
            && self.monitor2.all_expected(self.samples)
            && self.bar2.all_expected(self.samples)
    }
}

#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
#[derive(Default)]
struct InteriorProbeCounts {
    before: InteriorPhaseCounts,
    during: InteriorPhaseCounts,
    after: InteriorPhaseCounts,
    boundary: u32,
    unprobed: u32,
    xor: u32,
}

#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
impl InteriorProbeCounts {
    fn record(
        &mut self,
        phase_before: u32,
        phase_after: u32,
        post_completion: bool,
        uart0: &Bar1Probe,
        chip: &Bar1Probe,
        uart4: &Bar1Probe,
        monitor2: &Bar1Probe,
        bar2_valid: bool,
        anchors: &InteriorAnchors,
    ) {
        self.xor ^= phase_before
            ^ phase_after
            ^ uart0.value
            ^ chip.value
            ^ uart4.value
            ^ monitor2.value
            ^ u32::from(bar2_valid);
        if phase_before != phase_after {
            self.boundary = self.boundary.wrapping_add(1);
            return;
        }
        let counts = match (post_completion, phase_before) {
            (false, BLOCK_PHASE_IDLE | BLOCK_PHASE_PRECONDITION_OK) => &mut self.before,
            (false, BLOCK_PHASE_DISABLED) => &mut self.during,
            (true, BLOCK_PHASE_RESTORED) => &mut self.after,
            (
                false,
                BLOCK_PHASE_PRECONDITION_FAIL
                | BLOCK_PHASE_RESTORING
                | BLOCK_PHASE_REJECTED
                | BLOCK_PHASE_RESTORED,
            )
            | (true, _) => {
                self.unprobed = self.unprobed.wrapping_add(1);
                return;
            }
            (false, _) => {
                self.unprobed = self.unprobed.wrapping_add(1);
                return;
            }
        };
        counts.record(uart0, chip, uart4, monitor2, bar2_valid, anchors);
    }

    fn gate(&self) -> bool {
        self.before.samples >= INTERIOR_PRE_GO_PROBES as u32
            && self.before.gate_expected()
            && self.during.gate_during()
            && self.after.samples == INTERIOR_POST_RESTORE_PROBES as u32
            && self.after.gate_expected()
    }
}

#[cfg(feature = "rp1-bar1-interior-64k-hole-proof")]
fn log_interior_probe_counts(counts: &InteriorProbeCounts) {
    for (phase, values) in [
        ("before", &counts.before),
        ("during", &counts.during),
        ("after", &counts.after),
    ] {
        crate::logln!(
            "[RP1PHASE14:probe] phase={} samples={} uart0 exp/allff/other/abort={}/{}/{}/{} chip={}/{}/{}/{} uart4={}/{}/{}/{} monitor2={}/{}/{}/{} bar2={}/{}/{}/{}",
            phase,
            values.samples,
            values.uart0.expected,
            values.uart0.all_ff,
            values.uart0.other,
            values.uart0.abort,
            values.chip.expected,
            values.chip.all_ff,
            values.chip.other,
            values.chip.abort,
            values.uart4.expected,
            values.uart4.all_ff,
            values.uart4.other,
            values.uart4.abort,
            values.monitor2.expected,
            values.monitor2.all_ff,
            values.monitor2.other,
            values.monitor2.abort,
            values.bar2.expected,
            values.bar2.all_ff,
            values.bar2.other,
            values.bar2.abort,
        );
        crate::logln!(
            "[RP1PHASE14:raw] phase={} uart0 first/last/or/and/xor/seen={:08x}/{:08x}/{:08x}/{:08x}/{:08x}/{} uart4={:08x}/{:08x}/{:08x}/{:08x}/{:08x}/{}",
            phase,
            values.uart0.raw_first,
            values.uart0.raw_last,
            values.uart0.raw_or,
            values.uart0.raw_and,
            values.uart0.raw_xor,
            values.uart0.raw_seen,
            values.uart4.raw_first,
            values.uart4.raw_last,
            values.uart4.raw_or,
            values.uart4.raw_and,
            values.uart4.raw_xor,
            values.uart4.raw_seen,
        );
        crate::logln!(
            "[RP1PHASE14:raw-control] phase={} chip first/last/or/and/xor/seen={:08x}/{:08x}/{:08x}/{:08x}/{:08x}/{} monitor2={:08x}/{:08x}/{:08x}/{:08x}/{:08x}/{} bar2={:08x}/{:08x}/{:08x}/{:08x}/{:08x}/{}",
            phase,
            values.chip.raw_first,
            values.chip.raw_last,
            values.chip.raw_or,
            values.chip.raw_and,
            values.chip.raw_xor,
            values.chip.raw_seen,
            values.monitor2.raw_first,
            values.monitor2.raw_last,
            values.monitor2.raw_or,
            values.monitor2.raw_and,
            values.monitor2.raw_xor,
            values.monitor2.raw_seen,
            values.bar2.raw_first,
            values.bar2.raw_last,
            values.bar2.raw_or,
            values.bar2.raw_and,
            values.bar2.raw_xor,
            values.bar2.raw_seen,
        );
    }
    crate::logln!(
        "[RP1PHASE14:probe] boundary={} unprobed={} xor=0x{:08x}",
        counts.boundary,
        counts.unprobed,
        counts.xor,
    );
}

fn send_mode_and_wait_ack(bar2: usize, seq: u32, mode: u32, name: &'static str) -> bool {
    unsafe {
        write_record_word(bar2, W_GO, 0);
        write_record_word(bar2, W_MODE, mode);
        write_record_word(bar2, W_SEQ, seq);
    }
    let start = now_us();
    while now_us().wrapping_sub(start) <= ACK_TIMEOUT_US {
        let ack = unsafe { read_record_word(bar2, W_ACK) };
        if ack == seq {
            crate::logln!("[RP1INBOUND:{}] RP1 ack seq={}", name, seq);
            return true;
        }
        crate::timer::delay_micros(500);
    }
    crate::logln!("[RP1INBOUND:{}] timeout waiting RP1 ack seq={}", name, seq);
    false
}

fn dump_record_summary(label: &'static str, bar2: usize) {
    crate::logln!(
        "[RP1INBOUND:{}] seq={} ack={} go={} mode={} phase={} completion=0x{:08x} completion_seq={} flags=0x{:08x} result=0x{:08x}",
        label,
        unsafe { read_record_word(bar2, W_SEQ) },
        unsafe { read_record_word(bar2, W_ACK) },
        unsafe { read_record_word(bar2, W_GO) },
        unsafe { read_record_word(bar2, W_MODE) },
        unsafe { read_record_word(bar2, W_PHASE) },
        unsafe { read_record_word(bar2, W_COMPLETION) },
        unsafe { read_record_word(bar2, W_COMPLETION_SEQ) },
        unsafe { read_record_word(bar2, W_FLAGS) },
        unsafe { read_record_word(bar2, W_RESULT) },
    );
    crate::logln!(
        "[RP1INBOUND:{}] time start={:08x}:{:08x} end={:08x}:{:08x} elapsed_us={} samples={} overflow={} checksum=0x{:08x}",
        label,
        unsafe { read_record_word(bar2, W_STARTED_US_HI) },
        unsafe { read_record_word(bar2, W_STARTED_US_LO) },
        unsafe { read_record_word(bar2, W_ENDED_US_HI) },
        unsafe { read_record_word(bar2, W_ENDED_US_LO) },
        unsafe { read_record_word(bar2, W_ELAPSED_US) },
        unsafe { read_record_word(bar2, W_SAMPLE_COUNT) },
        unsafe { read_record_word(bar2, W_OVERFLOW_COUNT) },
        unsafe { read_record_word(bar2, W_CHECKSUM) },
    );
    crate::logln!(
        "[RP1INBOUND:{}] args arg0={} arg1=0x{:08x} health=0x{:08x} config_change_count={}",
        label,
        unsafe { read_record_word(bar2, W_ARG0) },
        unsafe { read_record_word(bar2, W_ARG1) },
        unsafe { read_record_word(bar2, W_HEALTH_FLAGS) },
        unsafe { read_record_word(bar2, W_CONFIG_CHANGE_COUNT) },
    );
    crate::logln!(
        "[RP1INBOUND:{}] monitor0_or/max={:08x}/{:08x} monitor1_or/max={:08x}/{:08x} monitor2_or/max={:08x}/{:08x}",
        label,
        unsafe { read_record_word(bar2, W_MONITOR0_OR) },
        unsafe { read_record_word(bar2, W_MONITOR0_MAX) },
        unsafe { read_record_word(bar2, W_MONITOR1_OR) },
        unsafe { read_record_word(bar2, W_MONITOR1_MAX) },
        unsafe { read_record_word(bar2, W_MONITOR2_OR) },
        unsafe { read_record_word(bar2, W_MONITOR2_MAX) },
    );
    crate::logln!(
        "[RP1INBOUND:{}] monitor2 counts bit23={} bit22={} bit21={} first_us={}/{}/{}",
        label,
        unsafe { read_record_word(bar2, W_MONITOR2_BIT23_COUNT) },
        unsafe { read_record_word(bar2, W_MONITOR2_BIT22_COUNT) },
        unsafe { read_record_word(bar2, W_MONITOR2_BIT21_COUNT) },
        unsafe { read_record_word(bar2, W_MONITOR2_BIT23_FIRST_US) },
        unsafe { read_record_word(bar2, W_MONITOR2_BIT22_FIRST_US) },
        unsafe { read_record_word(bar2, W_MONITOR2_BIT21_FIRST_US) },
    );
    for index in 0..3 {
        crate::logln!(
            "[RP1INBOUND:{}] pcie_cfg{} before=0x{:08x} after=0x{:08x}",
            label,
            index,
            unsafe { read_record_word(bar2, W_PCIE_CFG_BEFORE + index) },
            unsafe { read_record_word(bar2, W_PCIE_CFG_AFTER + index) },
        );
    }
    crate::logln!(
        "[RP1INBOUND:{}] scratch={:08x} {:08x} {:08x} {:08x}",
        label,
        unsafe { read_record_word(bar2, W_SCRATCH) },
        unsafe { read_record_word(bar2, W_SCRATCH + 1) },
        unsafe { read_record_word(bar2, W_SCRATCH + 2) },
        unsafe { read_record_word(bar2, W_SCRATCH + 3) },
    );
    crate::logln!(
        "[RP1INBOUND:{}] scratch_initial={:08x} {:08x} {:08x} {:08x}",
        label,
        unsafe { read_record_word(bar2, W_SCRATCH_INITIAL) },
        unsafe { read_record_word(bar2, W_SCRATCH_INITIAL + 1) },
        unsafe { read_record_word(bar2, W_SCRATCH_INITIAL + 2) },
        unsafe { read_record_word(bar2, W_SCRATCH_INITIAL + 3) },
    );
    crate::logln!(
        "[RP1INBOUND:{}] scratch_last={:08x} {:08x} {:08x} {:08x}",
        label,
        unsafe { read_record_word(bar2, W_SCRATCH_LAST) },
        unsafe { read_record_word(bar2, W_SCRATCH_LAST + 1) },
        unsafe { read_record_word(bar2, W_SCRATCH_LAST + 2) },
        unsafe { read_record_word(bar2, W_SCRATCH_LAST + 3) },
    );
    crate::logln!(
        "[RP1INBOUND:{}] scratch_final={:08x} {:08x} {:08x} {:08x} restore_ok=0x{:08x} changed_count={} last_change_us={}",
        label,
        unsafe { read_record_word(bar2, W_SCRATCH_FINAL) },
        unsafe { read_record_word(bar2, W_SCRATCH_FINAL + 1) },
        unsafe { read_record_word(bar2, W_SCRATCH_FINAL + 2) },
        unsafe { read_record_word(bar2, W_SCRATCH_FINAL + 3) },
        unsafe { read_record_word(bar2, W_SCRATCH_RESTORE_OK) },
        unsafe { read_record_word(bar2, W_SCRATCH_CHANGE_COUNT) },
        unsafe { read_record_word(bar2, W_SCRATCH_LAST_CHANGE_US) },
    );
    crate::logln!(
        "[RP1INBOUND:{}] block phase={} disable_us={} restore_us={} selector_saved={:08x} selector_restore={:08x} ctrl2={:08x}/{:08x}/{:08x}/{:08x} block_flags=0x{:08x}",
        label,
        unsafe { read_record_word(bar2, W_BLOCK_PHASE) },
        unsafe { read_record_word(bar2, W_BLOCK_DISABLE_US) },
        unsafe { read_record_word(bar2, W_BLOCK_RESTORE_US) },
        unsafe { read_record_word(bar2, W_SELECTOR_SAVED) },
        unsafe { read_record_word(bar2, W_SELECTOR_RESTORE_READBACK) },
        unsafe { read_record_word(bar2, W_CTRL2_BEFORE) },
        unsafe { read_record_word(bar2, W_CTRL2_BLOCK_VALUE) },
        unsafe { read_record_word(bar2, W_CTRL2_BLOCK_READBACK) },
        unsafe { read_record_word(bar2, W_CTRL2_RESTORE_READBACK) },
        unsafe { read_record_word(bar2, W_FLAGS) }
            & (FLAG_CTRL2_PRECONDITION_OK
                | FLAG_CTRL2_WRITTEN
                | FLAG_CTRL2_BLOCK_READBACK_OK
                | FLAG_CTRL2_RESTORED
                | FLAG_SELECTOR_RESTORED
                | FLAG_SCRATCH_RESTORED),
    );
    for channel in 0..CHANNEL_COUNT {
        let cfg_before = W_AXISHIM_CFG_BEFORE + channel;
        let cfg_after = W_AXISHIM_CFG_AFTER + channel;
        let status = W_AXISHIM_STATUS + channel * AXISHIM_STATUS_WORDS;
        crate::logln!(
            "[RP1INBOUND:{}] channel{} cfg={:08x}->{:08x} status_or={:08x} status_max={:08x} count={} first_us={}",
            label,
            channel,
            unsafe { read_record_word(bar2, cfg_before) },
            unsafe { read_record_word(bar2, cfg_after) },
            unsafe { read_record_word(bar2, status) },
            unsafe { read_record_word(bar2, status + 1) },
            unsafe { read_record_word(bar2, status + 2) },
            unsafe { read_record_word(bar2, status + 3) },
        );
    }
}

#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
fn dump_protection_summary(label: &'static str, bar2: usize) {
    crate::logln!(
        "[RP1PROTECT:{}] DBI bar0/bar1/bar2/bus/command={:08x}/{:08x}/{:08x}/{:08x}/{:08x}",
        label,
        unsafe { read_record_word(bar2, W_BAR0) },
        unsafe { read_record_word(bar2, W_BAR1) },
        unsafe { read_record_word(bar2, W_BAR2) },
        unsafe { read_record_word(bar2, W_BAR1_BUS_BASE) },
        unsafe { read_record_word(bar2, W_COMMAND) },
    );
    crate::logln!(
        "[RP1PROTECT:{}] target_page=0x{:08x} bus_base=0x{:08x} dummy=0x{:08x} timing enable/disable/restore={}/{}/{} flags=0x{:08x}",
        label,
        unsafe { read_record_word(bar2, W_TARGET_PAGE_OFFSET) },
        unsafe { read_record_word(bar2, W_PROTECT_BAR1_BUS_BASE) },
        unsafe { read_record_word(bar2, W_DUMMY_LOCAL_BASE) },
        unsafe { read_record_word(bar2, W_PROTECT_ENABLE_US) },
        unsafe { read_record_word(bar2, W_PROTECT_DISABLE_US) },
        unsafe { read_record_word(bar2, W_PROTECT_RESTORE_US) },
        unsafe { read_record_word(bar2, W_PROTECT_FLAGS) },
    );
    crate::logln!(
        "[RP1PROTECT:{}] local target before/during/after={:08x}/{:08x}/{:08x} control={:08x}/{:08x}/{:08x}",
        label,
        unsafe { read_record_word(bar2, W_TARGET_BEFORE) },
        unsafe { read_record_word(bar2, W_TARGET_DURING) },
        unsafe { read_record_word(bar2, W_TARGET_AFTER) },
        unsafe { read_record_word(bar2, W_CONTROL_BEFORE) },
        unsafe { read_record_word(bar2, W_CONTROL_DURING) },
        unsafe { read_record_word(bar2, W_CONTROL_AFTER) },
    );
    log_iatu_snapshot(label, "original", 0, bar2, W_ORIGINAL_BAR1);
    for (set, base) in [
        ("before", W_SPARE_BEFORE),
        ("programmed", W_SPARE_PROGRAMMED),
        ("readback", W_SPARE_READBACK),
        ("restored", W_SPARE_RESTORED),
    ] {
        for slot in 0..2 {
            log_iatu_snapshot(label, set, slot, bar2, base + slot * IATU_SNAPSHOT_WORDS);
        }
    }
}

#[cfg(any(
    feature = "rp1-bar1-4k-protection-proof",
    feature = "rp1-iatu-second-spare-programming-proof"
))]
fn log_iatu_snapshot(
    label: &'static str,
    set: &'static str,
    slot: usize,
    bar2: usize,
    base: usize,
) {
    crate::logln!(
        "[RP1PROTECT:{}] iATU {}[{}] sel/ctrl1/ctrl2/base_hi/lo/limit/target_hi/lo/upper_limit={:08x}/{:08x}/{:08x}/{:08x}/{:08x}/{:08x}/{:08x}/{:08x}/{:08x}",
        label,
        set,
        slot,
        unsafe { read_record_word(bar2, base) },
        unsafe { read_record_word(bar2, base + 1) },
        unsafe { read_record_word(bar2, base + 2) },
        unsafe { read_record_word(bar2, base + 4) },
        unsafe { read_record_word(bar2, base + 3) },
        unsafe { read_record_word(bar2, base + 5) },
        unsafe { read_record_word(bar2, base + 7) },
        unsafe { read_record_word(bar2, base + 6) },
        unsafe { read_record_word(bar2, base + 8) },
    );
}

fn log_endpoint_link_audit(rp1: &Rp1Config, label: &'static str) {
    crate::logln!(
        "[RP1INBOUND:{}] endpoint/link audit bar1={:?} bar2={:?} msix={:?} dma={:?} pcie={:?}",
        label,
        rp1.peripheral_addr,
        rp1.shared_sram_addr,
        rp1.msi_x_table_addr,
        rp1.dma_window,
        rp1.pcie_base
    );
}

fn busy_wait_us(us: u64) {
    let start = now_us();
    while now_us().wrapping_sub(start) < us {
        core::hint::spin_loop();
    }
}

fn now_us() -> u64 {
    arch_timer::read_counter() / core::cmp::max(1, crate::timer::counter_frequency_hz() / 1_000_000)
}

struct AbortVectorGuard {
    old_vbar: u64,
}

unsafe fn arm_expected_abort(mode: u32) {
    unsafe {
        core::ptr::addr_of_mut!(__rp1_bar1_abort_phase).write_volatile(0);
        core::ptr::addr_of_mut!(__rp1_bar1_abort_mode).write_volatile(u64::from(mode));
        core::ptr::addr_of_mut!(__rp1_bar1_abort_armed).write_volatile(1);
        asm!("dsb sy", "isb", options(nostack, preserves_flags));
    }
}

unsafe fn set_expected_abort_phase(phase: u32) {
    unsafe {
        core::ptr::addr_of_mut!(__rp1_bar1_abort_phase).write_volatile(u64::from(phase));
        asm!("dsb sy", options(nostack, preserves_flags));
    }
}

unsafe fn disarm_expected_abort() {
    unsafe {
        core::ptr::addr_of_mut!(__rp1_bar1_abort_armed).write_volatile(0);
        core::ptr::addr_of_mut!(__rp1_bar1_abort_mode).write_volatile(0);
        core::ptr::addr_of_mut!(__rp1_bar1_abort_phase).write_volatile(0);
        asm!("dsb sy", "isb", options(nostack, preserves_flags));
    }
}

fn install_abort_vector() -> Option<AbortVectorGuard> {
    let vector = core::ptr::addr_of!(__rp1_bar1_abort_vector) as u64;
    if vector & 0x7ff != 0 {
        crate::logln!(
            "[RP1INBOUND:BLOCK_BAR1] vector alignment invalid addr=0x{:x}",
            vector
        );
        return None;
    }
    let daif: u64;
    unsafe {
        asm!("mrs {}, DAIF", out(reg) daif, options(nomem, nostack, preserves_flags));
    }
    crate::logln!("[RP1INBOUND:BLOCK_BAR1] DAIF=0x{:x}", daif);
    if daif & 0x3c0 != 0x3c0 {
        crate::logln!("[RP1INBOUND:BLOCK_BAR1] vector install rejected: exceptions not masked");
        return None;
    }
    let old_vbar: u64;
    unsafe {
        asm!("mrs {}, VBAR_EL2", out(reg) old_vbar, options(nomem, nostack, preserves_flags));
        asm!("msr VBAR_EL2, {}", in(reg) vector, options(nostack, preserves_flags));
        asm!("isb", options(nostack, preserves_flags));
    }
    crate::logln!(
        "[RP1INBOUND:BLOCK_BAR1] vector installed old=0x{:x} new=0x{:x}",
        old_vbar,
        vector
    );
    Some(AbortVectorGuard { old_vbar })
}

fn restore_abort_vector(guard: AbortVectorGuard) {
    unsafe {
        asm!("msr VBAR_EL2, {}", in(reg) guard.old_vbar, options(nostack, preserves_flags));
        asm!("isb", options(nostack, preserves_flags));
    }
    crate::logln!(
        "[RP1INBOUND:BLOCK_BAR1] vector restored old=0x{:x}",
        guard.old_vbar
    );
}

struct Bar1Probe {
    value: u32,
    expected_abort: bool,
}

unsafe fn bar1_abort_probe(addr: *const u32) -> Bar1Probe {
    let value = unsafe { __rp1_bar1_abort_probe_load(addr) };
    let expected_abort = unsafe { core::ptr::addr_of!(__rp1_bar1_abort_hit).read_volatile() != 0 };
    let unexpected = unsafe { core::ptr::addr_of!(__rp1_bar1_unexpected_hit).read_volatile() != 0 };
    if unexpected {
        crate::logln!(
            "[RP1INBOUND:BLOCK_BAR1] abort expected={} unexpected={} esr=0x{:x} far=0x{:x} elr=0x{:x}",
            expected_abort,
            unexpected,
            unsafe { core::ptr::addr_of!(__rp1_bar1_abort_esr).read_volatile() },
            unsafe { core::ptr::addr_of!(__rp1_bar1_abort_far).read_volatile() },
            unsafe { core::ptr::addr_of!(__rp1_bar1_abort_elr).read_volatile() },
        );
    }
    Bar1Probe {
        value,
        expected_abort,
    }
}

unsafe fn read_record_word(bar2: usize, index: usize) -> u32 {
    unsafe { read32(bar2 + RECORD_OFFSET + index * 4) }
}

unsafe fn write_record_word(bar2: usize, index: usize, value: u32) {
    unsafe { write32(bar2 + RECORD_OFFSET + index * 4, value) }
}

unsafe fn read32(addr: usize) -> u32 {
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

unsafe fn write32(addr: usize, value: u32) {
    unsafe { core::ptr::write_volatile(addr as *mut u32, value) };
    unsafe { asm!("dsb sy", options(nostack, preserves_flags)) };
}
