use arch_hal::soc::bcm2712::Rp1Config;

use crate::BootError;

const BAR2_BASE: u64 = 0x1f00_4000_00;
const BAR2_SIZE: u64 = 0x1_0000;
const RPC_START: usize = 0xf900;
const REQUEST_OFFSET: usize = 0xf900;
const RESPONSE_OFFSET: usize = 0xf980;
const RPC_END: usize = 0xfa00;
const SLOT_WORDS: usize = 32;
const OWNER_WORD: usize = 15;

const REQUEST_MAGIC: u32 = 0x5152_3152;
const RESPONSE_MAGIC: u32 = 0x5352_3152;
const VERSION: u32 = 1;
const HEADER_WORDS: u32 = 16;
const TOTAL_WORDS: u32 = 32;
const CHECKSUM_SEED: u32 = 0x5250_4331;
const CHECKSUM_MULTIPLIER: u32 = 0x9e37_79b1;

const OP_PING: u32 = 0;
const OP_GET_CAPABILITIES: u32 = 1;
const OP_GET_CLOCK_STATE: u32 = 2;
const CLOCK_PLL_SYS_PRI_PH: u32 = 6;
const CLOCK_UART: u32 = 15;
const STATUS_OK: u32 = 0;
const EXPECTED_FEATURES: u32 = 0x3b;
const EXPECTED_OPCODE_MASK: u32 = 0x7;
const TIMEOUT_US: u64 = 250_000;

const _: () = assert!(REQUEST_OFFSET == RPC_START);
const _: () = assert!(REQUEST_OFFSET + SLOT_WORDS * 4 == RESPONSE_OFFSET);
const _: () = assert!(RESPONSE_OFFSET + SLOT_WORDS * 4 == RPC_END);

#[derive(Clone, Copy)]
struct Response {
    effective: u32,
    physical: u32,
    result0: u32,
    result1: u32,
}

pub fn run_pre_linux_probe(rp1: &Rp1Config) -> Result<(), BootError> {
    let Some((base, size)) = rp1.shared_sram_addr else {
        crate::logln!("[RP1RPC] fail: BAR2 absent");
        return Err(BootError::Rp1Bar2Rpc);
    };
    if base != BAR2_BASE || size != BAR2_SIZE {
        crate::logln!(
            "[RP1RPC] fail: BAR2 precondition base=0x{:x} size=0x{:x}",
            base,
            size
        );
        return Err(BootError::Rp1Bar2Rpc);
    }
    let Ok(base) = usize::try_from(base) else {
        crate::logln!("[RP1RPC] fail: BAR2 base does not fit usize");
        return Err(BootError::Rp1Bar2Rpc);
    };

    crate::logln!(
        "[RP1RPC] start base=0x{:x} size=0x{:x} window=0xf900..0xf9ff",
        base,
        size
    );
    let client = RpcClient { base };
    client.check_ready_and_idle()?;

    let ping = client.call(1, OP_PING, 0)?;
    if ping.effective != 1
        || ping.physical != 1
        || ping.result0 != VERSION
        || ping.result1 != EXPECTED_OPCODE_MASK
    {
        crate::logln!(
            "[RP1RPC] fail: ping payload effective=0x{:08x} physical=0x{:08x} result0=0x{:08x} result1=0x{:08x}",
            ping.effective,
            ping.physical,
            ping.result0,
            ping.result1
        );
        return Err(BootError::Rp1Bar2Rpc);
    }
    crate::logln!(
        "[RP1RPC] ping ok effective=0x{:08x} physical=0x{:08x} result0=0x{:08x} result1=0x{:08x}",
        ping.effective,
        ping.physical,
        ping.result0,
        ping.result1
    );

    let caps = client.call(2, OP_GET_CAPABILITIES, 0)?;
    if caps.result0 != EXPECTED_FEATURES || caps.result1 != EXPECTED_OPCODE_MASK {
        crate::logln!(
            "[RP1RPC] fail: caps payload features=0x{:08x} opcodes=0x{:08x}",
            caps.result0,
            caps.result1
        );
        return Err(BootError::Rp1Bar2Rpc);
    }
    crate::logln!(
        "[RP1RPC] caps ok features=0x{:08x} opcodes=0x{:08x}",
        caps.result0,
        caps.result1
    );

    let pll = client.call(3, OP_GET_CLOCK_STATE, CLOCK_PLL_SYS_PRI_PH)?;
    crate::logln!(
        "[RP1RPC] clock id=6 ok effective=0x{:08x} physical=0x{:08x} result0=0x{:08x} result1=0x{:08x}",
        pll.effective,
        pll.physical,
        pll.result0,
        pll.result1
    );

    let uart = client.call(4, OP_GET_CLOCK_STATE, CLOCK_UART)?;
    crate::logln!(
        "[RP1RPC] clock id=15 ok effective=0x{:08x} physical=0x{:08x} div=0x{:08x} sel=0x{:08x}",
        uart.effective,
        uart.physical,
        uart.result0,
        uart.result1
    );
    crate::logln!("[RP1RPC] pass");
    Ok(())
}

struct RpcClient {
    base: usize,
}

#[derive(Clone, Copy)]
enum Slot {
    Request,
    Response,
}

impl RpcClient {
    fn check_ready_and_idle(&self) -> Result<(), BootError> {
        if self.read(Slot::Response, 0) != RESPONSE_MAGIC
            || self.read(Slot::Response, 1) != VERSION
            || self.read(Slot::Response, 2) != HEADER_WORDS
            || self.read(Slot::Response, 3) != TOTAL_WORDS
        {
            crate::logln!("[RP1RPC] fail: idle response header not ready");
            return Err(BootError::Rp1Bar2Rpc);
        }
        self.require_idle("initial")
    }

    fn require_idle(&self, phase: &str) -> Result<(), BootError> {
        let request_owner = self.read(Slot::Request, OWNER_WORD);
        let response_owner = self.read(Slot::Response, OWNER_WORD);
        if request_owner != 0 || response_owner != 0 {
            crate::logln!(
                "[RP1RPC] fail: slot busy {} request_owner=0x{:08x} response_owner=0x{:08x}",
                phase,
                request_owner,
                response_owner
            );
            return Err(BootError::Rp1Bar2Rpc);
        }
        Ok(())
    }

    fn call(&self, sequence: u32, opcode: u32, object_id: u32) -> Result<Response, BootError> {
        self.require_idle("before submit")?;
        let request = request_words(sequence, opcode, object_id);
        crate::logln!(
            "[RP1RPC] submit seq={} opcode={} object={}",
            sequence,
            opcode,
            object_id
        );

        for (index, word) in request.iter().enumerate() {
            if index != 14 && index != OWNER_WORD {
                self.write(Slot::Request, index, *word);
            }
        }
        self.write(Slot::Request, 14, request[14]);
        dmb_sy();
        self.write(Slot::Request, OWNER_WORD, 1);
        dsb_sy();
        if self.read(Slot::Request, OWNER_WORD) != 1 {
            crate::logln!("[RP1RPC] fail: request owner readback");
            return Err(BootError::Rp1Bar2Rpc);
        }

        let deadline = now_ticks().wrapping_add(timeout_ticks());
        loop {
            let owner = self.read(Slot::Response, OWNER_WORD);
            if owner == 1 {
                break;
            }
            if owner != 0 {
                crate::logln!("[RP1RPC] fail: invalid response owner=0x{:08x}", owner);
                return Err(BootError::Rp1Bar2Rpc);
            }
            if now_ticks().wrapping_sub(deadline) < u64::MAX / 2 {
                crate::logln!("[RP1RPC] fail: timeout seq={}", sequence);
                return Err(BootError::Rp1Bar2Rpc);
            }
        }
        dmb_sy();

        let mut response = [0u32; SLOT_WORDS];
        for (index, word) in response.iter_mut().enumerate() {
            *word = self.read(Slot::Response, index);
        }
        let decoded = validate_response(&response, sequence, opcode)?;
        self.write(Slot::Response, OWNER_WORD, 0);
        dsb_sy();
        if self.read(Slot::Response, OWNER_WORD) != 0 {
            crate::logln!("[RP1RPC] fail: response owner clear readback");
            return Err(BootError::Rp1Bar2Rpc);
        }
        self.wait_request_clear(sequence)?;
        Ok(decoded)
    }

    fn wait_request_clear(&self, sequence: u32) -> Result<(), BootError> {
        let deadline = now_ticks().wrapping_add(timeout_ticks());
        while self.read(Slot::Request, OWNER_WORD) != 0 {
            if now_ticks().wrapping_sub(deadline) < u64::MAX / 2 {
                crate::logln!("[RP1RPC] fail: request owner clear timeout seq={}", sequence);
                return Err(BootError::Rp1Bar2Rpc);
            }
        }
        Ok(())
    }

    fn read(&self, slot: Slot, word: usize) -> u32 {
        let addr = self.addr(slot, word);
        unsafe { core::ptr::read_volatile(addr as *const u32) }
    }

    fn write(&self, slot: Slot, word: usize, value: u32) {
        let addr = self.addr(slot, word);
        unsafe { core::ptr::write_volatile(addr as *mut u32, value) }
    }

    fn addr(&self, slot: Slot, word: usize) -> usize {
        assert!(word < SLOT_WORDS);
        let offset = match slot {
            Slot::Request => REQUEST_OFFSET,
            Slot::Response => RESPONSE_OFFSET,
        };
        let addr = self.base + offset + word * 4;
        assert!(addr >= self.base + RPC_START && addr + 4 <= self.base + RPC_END);
        addr
    }
}

fn request_words(sequence: u32, opcode: u32, object_id: u32) -> [u32; SLOT_WORDS] {
    let mut words = [0u32; SLOT_WORDS];
    words[0] = REQUEST_MAGIC;
    words[1] = VERSION;
    words[2] = HEADER_WORDS;
    words[3] = TOTAL_WORDS;
    words[4] = sequence;
    words[5] = opcode;
    words[6] = object_id;
    words[14] = checksum(&words);
    words
}

fn validate_response(
    words: &[u32; SLOT_WORDS],
    sequence: u32,
    opcode: u32,
) -> Result<Response, BootError> {
    if words[OWNER_WORD] != 1
        || words[0] != RESPONSE_MAGIC
        || words[1] != VERSION
        || words[2] != HEADER_WORDS
        || words[3] != TOTAL_WORDS
        || words[4] != sequence
        || words[5] != opcode
        || words[7] != 0
        || words[14] != checksum(words)
        || words[6] != STATUS_OK
        || words[16..].iter().any(|word| *word != 0)
    {
        crate::logln!(
            "[RP1RPC] fail: bad response seq={} opcode={} status={} owner={} checksum=0x{:08x}",
            words[4],
            words[5],
            words[6],
            words[OWNER_WORD],
            words[14]
        );
        return Err(BootError::Rp1Bar2Rpc);
    }
    Ok(Response {
        effective: words[8],
        physical: words[9],
        result0: words[12],
        result1: words[13],
    })
}

fn checksum(words: &[u32; SLOT_WORDS]) -> u32 {
    let mut value = CHECKSUM_SEED;
    let mut index = 0;
    while index < SLOT_WORDS {
        if index != 14 && index != OWNER_WORD {
            value = (value ^ words[index])
                .rotate_left(5)
                .wrapping_mul(CHECKSUM_MULTIPLIER);
        }
        index += 1;
    }
    value
}

fn now_ticks() -> u64 {
    arch_timer::read_counter()
}

fn timeout_ticks() -> u64 {
    core::cmp::max(1, crate::timer::counter_frequency_hz() / 1_000_000) * TIMEOUT_US
}

fn dmb_sy() {
    unsafe { core::arch::asm!("dmb sy", options(nostack, preserves_flags)) }
}

fn dsb_sy() {
    unsafe { core::arch::asm!("dsb sy", options(nostack, preserves_flags)) }
}
