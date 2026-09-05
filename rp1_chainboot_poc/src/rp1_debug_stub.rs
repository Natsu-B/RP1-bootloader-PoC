use crate::rp1_bootstrap::{Rp1Bootstrap, Rp1I2cBus};
use rp1_abi::debug;

const PACKET_BUF_LEN: usize = 768;
const MAX_GDB_MEM: usize = 256;
const COEXISTENCE_PRIVATE_SIZE: usize = 0x300;
const D1RP_WIRE_SIZE: usize = core::mem::size_of::<debug::DebugMailbox>();

const _: () = assert!(D1RP_WIRE_SIZE <= COEXISTENCE_PRIVATE_SIZE);

const OFF_SEQ: u32 = 16;
const OFF_ACK: u32 = 20;
const OFF_COMMAND: u32 = 32;
const OFF_ARG0: u32 = 36;
const OFF_ARG1: u32 = 40;
const OFF_STATUS: u32 = 44;
const OFF_DATA_LEN: u32 = 120;
const OFF_DATA: u32 = 124;

const READ_SNAPSHOT_ALLOWLISTED: u32 = 0x5250_4401;
const LEGACY_READ_SNAPSHOT_ALLOWLISTED: u32 = 7;
const STATUS_OK: u32 = 0;
const STATUS_BAD_COMMAND: u32 = 1;
const STATUS_BAD_SNAPSHOT_ID: u32 = 4;
const SNAPSHOT_CORE_STATUS: u32 = 0x5250_5301;
const SNAPSHOT_PERIPHERAL_STATUS: u32 = 0x5250_5302;
const SNAPSHOT_INVALID_ID: u32 = 0x5250_53fe;
const SNAPSHOT_RESPONSE_MAGIC: u32 = u32::from_le_bytes(*b"S1RP");
const SNAPSHOT_FORMAT_VERSION: u32 = 2;
const SNAPSHOT_MAX_ENTRIES: usize = 8;
const SNAPSHOT_HEADER_WORDS: usize = 6;
const SNAPSHOT_ENTRY_WORDS: usize = 3;
const SNAPSHOT_MAX_LEN: usize =
    (SNAPSHOT_HEADER_WORDS + SNAPSHOT_MAX_ENTRIES * SNAPSHOT_ENTRY_WORDS + 1) * 4;
const SNAPSHOT_CHECKSUM_SEED: u32 = 0x811c_9dc5;
const CORE_STATUS_ADDRESSES: [u32; SNAPSHOT_MAX_ENTRIES] = [
    debug::MAILBOX_ADDR,
    debug::MAILBOX_ADDR + 0x04,
    debug::MAILBOX_ADDR + 0x08,
    debug::MAILBOX_ADDR + 0x14,
    debug::MAILBOX_ADDR + 0x18,
    debug::MAILBOX_ADDR + 0x1c,
    debug::MAILBOX_ADDR + 0x20,
    debug::MAILBOX_ADDR + 0x2c,
];
const PERIPHERAL_STATUS_ADDRESSES: [u32; SNAPSHOT_MAX_ENTRIES] = [
    0x4002_0000,
    0x4001_401c,
    0x400a_c028,
    0x4009_8000,
    0x4009_8060,
    0x4003_0018,
    0x4007_4070,
    0x4005_0028,
];

const UART_BASE: usize = 0x10_7d00_1000;
const UART_DR: usize = 0x00;
const UART_FR: usize = 0x18;
const UART_FR_RXFE: u32 = 1 << 4;
const UART_FR_TXFF: u32 = 1 << 5;
const RP1_SRAM_BASE: u32 = 0x2000_0000;
const RP1_SRAM_SIZE: usize = 0x1_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    Timeout,
    Bus,
    BadAddress,
    Unsupported,
}

pub trait Rp1MemoryTransport {
    fn read_mem(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), TransportError>;
    fn write_mem(&mut self, addr: u32, data: &[u8]) -> Result<(), TransportError>;
}

pub struct Rp1I2cTransport<'a, I2C> {
    bootstrap: &'a mut Rp1Bootstrap<I2C>,
}

impl<'a, I2C> Rp1I2cTransport<'a, I2C> {
    pub fn new(bootstrap: &'a mut Rp1Bootstrap<I2C>) -> Self {
        Self { bootstrap }
    }
}

impl<I2C> Rp1MemoryTransport for Rp1I2cTransport<'_, I2C>
where
    I2C: Rp1I2cBus,
{
    fn read_mem(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), TransportError> {
        self.bootstrap
            .read_mem(addr, buf)
            .map_err(|_| TransportError::Bus)
    }

    fn write_mem(&mut self, addr: u32, data: &[u8]) -> Result<(), TransportError> {
        self.bootstrap
            .write_mem(addr, data)
            .map_err(|_| TransportError::Bus)
    }
}

pub struct Rp1PcieTransport {
    sram_base: usize,
    sram_size: usize,
}

impl Rp1PcieTransport {
    pub fn new(sram_base: usize, sram_size: usize) -> Self {
        Self {
            sram_base,
            sram_size,
        }
    }

    fn translate_rp1_addr(&self, addr: u32, len: usize) -> Result<usize, TransportError> {
        let off = addr
            .checked_sub(RP1_SRAM_BASE)
            .ok_or(TransportError::BadAddress)? as usize;
        let end = off.checked_add(len).ok_or(TransportError::BadAddress)?;
        if end > core::cmp::min(self.sram_size, RP1_SRAM_SIZE) {
            return Err(TransportError::BadAddress);
        }
        self.sram_base
            .checked_add(off)
            .ok_or(TransportError::BadAddress)
    }

    pub fn log_probe(&mut self, label: &str) {
        let mut sram_head = [0u8; 16];
        let mut mailbox_head = [0u8; 16];
        let sram = self.read_mem(RP1_SRAM_BASE, &mut sram_head);
        let mailbox = self.read_mem(debug::MAILBOX_ADDR, &mut mailbox_head);
        crate::logln!(
            "[RP1GDB] probe {} sram={:?} head={:02x?}",
            label,
            sram,
            sram_head
        );
        crate::logln!(
            "[RP1GDB] probe {} mailbox={:?} head={:02x?}",
            label,
            mailbox,
            mailbox_head
        );
    }

    pub fn log_phase_readback(&mut self, phase: &str) {
        let mut sram_head = [0u8; 16];
        let mut mailbox_head = [0u8; 16];
        let sram = self.read_mem(RP1_SRAM_BASE, &mut sram_head);
        let mailbox = self.read_mem(debug::MAILBOX_ADDR, &mut mailbox_head);
        crate::logln!(
            "[RP1PCIE:{}] read 0x20000000 result={:?} data={:02x?}",
            phase,
            sram,
            sram_head
        );
        crate::logln!(
            "[RP1PCIE:{}] read 0x2000fc00 result={:?} data={:02x?}",
            phase,
            mailbox,
            mailbox_head
        );
    }

    pub fn log_pll_core_lock_result(&mut self, phase: &str) {
        for chunk in 0..4u32 {
            let addr = debug::MAILBOX_ADDR + chunk * 16;
            let mut data = [0u8; 16];
            let result = self.read_mem(addr, &mut data);
            crate::logln!(
                "[RP1PLLRESULT:{}] addr=0x{:08x} result={:?} data={:02x?}",
                phase,
                addr,
                result,
                data
            );
        }
    }

    #[cfg(feature = "rp1-boot-rom-dump")]
    pub fn log_boot_rom_dump(&mut self, phase: &str) {
        const DUMP_ADDR: u32 = 0x2000_6400;
        const DUMP_LEN: u32 = 0x8000;
        const CHUNK_LEN: u32 = 32;

        crate::logln!(
            "[RP1ROMDUMP:{}] begin addr=0x{:08x} len=0x{:08x} chunk={}",
            phase,
            DUMP_ADDR,
            DUMP_LEN,
            CHUNK_LEN
        );
        for offset in (0..DUMP_LEN).step_by(CHUNK_LEN as usize) {
            let mut data = [0u8; CHUNK_LEN as usize];
            let result = self.read_mem(DUMP_ADDR + offset, &mut data);
            crate::logln!(
                "[RP1ROMDUMP:{}] offset=0x{:08x} result={:?} data={:02x?}",
                phase,
                offset,
                result,
                data
            );
        }
        crate::logln!("[RP1ROMDUMP:{}] end chunks={}", phase, DUMP_LEN / CHUNK_LEN);
    }
}

impl Rp1MemoryTransport for Rp1PcieTransport {
    fn read_mem(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), TransportError> {
        let base = self.translate_rp1_addr(addr, buf.len())?;
        for (idx, dst) in buf.iter_mut().enumerate() {
            // SAFETY: address range was checked against the validated RP1 shared SRAM BAR alias.
            *dst = unsafe { core::ptr::read_volatile((base + idx) as *const u8) };
        }
        Ok(())
    }

    fn write_mem(&mut self, addr: u32, data: &[u8]) -> Result<(), TransportError> {
        let base = self.translate_rp1_addr(addr, data.len())?;
        for (idx, value) in data.iter().copied().enumerate() {
            // SAFETY: address range was checked against the validated RP1 shared SRAM BAR alias.
            unsafe { core::ptr::write_volatile((base + idx) as *mut u8, value) };
        }
        Ok(())
    }
}

pub fn serve_with_transport<T>(transport: &mut T) -> !
where
    T: Rp1MemoryTransport,
{
    crate::logln!("[RP1GDB] RP1 GDB debug stub mode active");
    if smoke_test_mailbox(transport).is_err() {
        crate::logln!("[RP1GDB] fatal: mailbox smoke test failed");
        loop {
            core::hint::spin_loop();
        }
    }
    crate::logln!("[RP1GDB] attach with: target remote <serial-device>");

    let mut server = GdbServer::new(transport);
    server.run()
}

fn smoke_test_mailbox<T>(transport: &mut T) -> Result<(), TransportError>
where
    T: Rp1MemoryTransport,
{
    let mut header = [0u8; 32];
    transport.read_mem(debug::MAILBOX_ADDR, &mut header)?;
    let magic = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let version = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    let size = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
    let state = u32::from_le_bytes([header[24], header[25], header[26], header[27]]);
    crate::logln!(
        "[RP1GDB] mailbox magic=0x{:08x} version={} size={} state={}",
        magic,
        version,
        size,
        state
    );
    if magic != debug::MAGIC || version != debug::VERSION || size as usize != D1RP_WIRE_SIZE {
        return Err(TransportError::Unsupported);
    }

    let mut command = MailboxCommand { transport, seq: 0 };
    command.command_no_payload(debug::command::PING)?;
    let ping_before_seq = command.seq;
    crate::logln!("[RP1SNAP] ping-before pass seq={}", ping_before_seq);

    run_snapshot_command(
        &mut command,
        SNAPSHOT_CORE_STATUS,
        STATUS_OK,
        SnapshotValidation::Core {
            previous_ack: ping_before_seq,
            mailbox_size: size,
        },
    )?;
    crate::logln!("[RP1SNAP] core snapshot pass seq={}", command.seq);

    let first_timer = run_snapshot_command(
        &mut command,
        SNAPSHOT_PERIPHERAL_STATUS,
        STATUS_OK,
        SnapshotValidation::Peripheral { sample: 1 },
    )?
    .ok_or(TransportError::Unsupported)?;
    crate::logln!(
        "[RP1SNAP] peripheral snapshot pass sample=1 seq={}",
        command.seq
    );

    let second_timer = run_snapshot_command(
        &mut command,
        SNAPSHOT_PERIPHERAL_STATUS,
        STATUS_OK,
        SnapshotValidation::Peripheral { sample: 2 },
    )?
    .ok_or(TransportError::Unsupported)?;
    if first_timer == second_timer {
        return Err(TransportError::Unsupported);
    }
    crate::logln!(
        "[RP1SNAP] peripheral snapshot pass sample=2 seq={} timer_delta={}",
        command.seq,
        second_timer.wrapping_sub(first_timer)
    );

    run_snapshot_command(
        &mut command,
        SNAPSHOT_INVALID_ID,
        STATUS_BAD_SNAPSHOT_ID,
        SnapshotValidation::Empty,
    )?;
    crate::logln!("[RP1SNAP] snapshot-id reject pass seq={}", command.seq);

    let legacy_status = command.command_with_args(LEGACY_READ_SNAPSHOT_ALLOWLISTED, 0, 0)?;
    if legacy_status != STATUS_BAD_COMMAND {
        return Err(TransportError::Unsupported);
    }
    crate::logln!(
        "[RP1SNAP] legacy-command reject pass seq={} id={}",
        command.seq,
        LEGACY_READ_SNAPSHOT_ALLOWLISTED
    );

    command.command_no_payload(debug::command::PING)?;
    crate::logln!("[RP1SNAP] ping-after pass seq={}", command.seq);
    crate::logln!("[RP1SNAP] proof pass");
    Ok(())
}

enum SnapshotValidation {
    Core {
        previous_ack: u32,
        mailbox_size: u32,
    },
    Peripheral {
        sample: u32,
    },
    Empty,
}

fn run_snapshot_command<T>(
    command: &mut MailboxCommand<'_, T>,
    snapshot_id: u32,
    expected_status: u32,
    validation: SnapshotValidation,
) -> Result<Option<u32>, TransportError>
where
    T: Rp1MemoryTransport,
{
    let sequence = command.seq.wrapping_add(1);
    let checksum = snapshot_request_checksum(sequence, snapshot_id);
    let status = command.command_with_args(READ_SNAPSHOT_ALLOWLISTED, snapshot_id, checksum)?;
    if status != expected_status || command.seq != sequence {
        return Err(TransportError::Unsupported);
    }

    let len = command.read_u32(OFF_DATA_LEN)? as usize;
    if len > SNAPSHOT_MAX_LEN {
        return Err(TransportError::Unsupported);
    }
    let mut data = [0u8; SNAPSHOT_MAX_LEN];
    command
        .transport
        .read_mem(debug::MAILBOX_ADDR + OFF_DATA, &mut data[..len])?;
    let count = validate_snapshot_response(&data[..len], snapshot_id, sequence, expected_status)?;

    let timer_value = match validation {
        SnapshotValidation::Core {
            previous_ack,
            mailbox_size,
        } => {
            validate_core_status_entries(&data[..len], count, previous_ack, mailbox_size)?;
            None
        }
        SnapshotValidation::Peripheral { sample } => Some(validate_peripheral_status_entries(
            &data[..len],
            count,
            sample,
        )?),
        SnapshotValidation::Empty => {
            if count != 0 {
                return Err(TransportError::Unsupported);
            }
            None
        }
    };

    crate::logln!(
        "[RP1SNAP] response id={} seq={} status={} count={} len={} checksum=pass",
        snapshot_id,
        sequence,
        expected_status,
        count,
        len
    );
    Ok(timer_value)
}

fn validate_snapshot_response(
    data: &[u8],
    snapshot_id: u32,
    sequence: u32,
    expected_status: u32,
) -> Result<usize, TransportError> {
    if data.len() < (SNAPSHOT_HEADER_WORDS + 1) * 4 || data.len() % 4 != 0 {
        return Err(TransportError::Unsupported);
    }
    let count = snapshot_word(data, 5)? as usize;
    if count > SNAPSHOT_MAX_ENTRIES {
        return Err(TransportError::Unsupported);
    }
    let words = SNAPSHOT_HEADER_WORDS + count * SNAPSHOT_ENTRY_WORDS + 1;
    if data.len() != words * 4
        || snapshot_word(data, 0)? != SNAPSHOT_RESPONSE_MAGIC
        || snapshot_word(data, 1)? != SNAPSHOT_FORMAT_VERSION
        || snapshot_word(data, 2)? != snapshot_id
        || snapshot_word(data, 3)? != sequence
        || snapshot_word(data, 4)? != expected_status
    {
        return Err(TransportError::Unsupported);
    }

    let mut checksum = SNAPSHOT_CHECKSUM_SEED;
    for index in 0..words - 1 {
        checksum = snapshot_checksum_update(checksum, snapshot_word(data, index)?);
    }
    if snapshot_word(data, words - 1)? != checksum {
        return Err(TransportError::Unsupported);
    }
    Ok(count)
}

fn validate_core_status_entries(
    data: &[u8],
    count: usize,
    previous_ack: u32,
    mailbox_size: u32,
) -> Result<(), TransportError> {
    if count != CORE_STATUS_ADDRESSES.len() {
        return Err(TransportError::Unsupported);
    }
    for (index, expected_address) in CORE_STATUS_ADDRESSES.iter().copied().enumerate() {
        let base = SNAPSHOT_HEADER_WORDS + index * SNAPSHOT_ENTRY_WORDS;
        let address = snapshot_word(data, base)?;
        let value = snapshot_word(data, base + 1)?;
        let entry_status = snapshot_word(data, base + 2)?;
        if address != expected_address || entry_status != STATUS_OK {
            return Err(TransportError::Unsupported);
        }
        match index {
            0 if value != debug::MAGIC => return Err(TransportError::Unsupported),
            1 if value != debug::VERSION => return Err(TransportError::Unsupported),
            2 if value != mailbox_size => return Err(TransportError::Unsupported),
            3 if value != previous_ack => return Err(TransportError::Unsupported),
            4 if value != debug::state::RUNNING => return Err(TransportError::Unsupported),
            5 if value != debug::stop_reason::NONE => return Err(TransportError::Unsupported),
            6 if value != READ_SNAPSHOT_ALLOWLISTED => {
                return Err(TransportError::Unsupported);
            }
            7 if value != STATUS_OK => return Err(TransportError::Unsupported),
            _ => {}
        }
        crate::logln!(
            "[RP1SNAP] core-entry index={} address=0x{:08x} value=0x{:08x} status={}",
            index,
            address,
            value,
            entry_status
        );
    }
    Ok(())
}

fn validate_peripheral_status_entries(
    data: &[u8],
    count: usize,
    sample: u32,
) -> Result<u32, TransportError> {
    if count != PERIPHERAL_STATUS_ADDRESSES.len() {
        return Err(TransportError::Unsupported);
    }
    let mut timer_value = None;
    for (index, expected_address) in PERIPHERAL_STATUS_ADDRESSES.iter().copied().enumerate() {
        let base = SNAPSHOT_HEADER_WORDS + index * SNAPSHOT_ENTRY_WORDS;
        let address = snapshot_word(data, base)?;
        let value = snapshot_word(data, base + 1)?;
        let entry_status = snapshot_word(data, base + 2)?;
        if address != expected_address
            || entry_status != STATUS_OK
            || (index != 1 && value == u32::MAX)
        {
            return Err(TransportError::Unsupported);
        }
        if index == 2 {
            timer_value = Some(value);
        }
        crate::logln!(
            "[RP1SNAP] peripheral-entry sample={} index={} address=0x{:08x} value=0x{:08x} status={}",
            sample,
            index,
            address,
            value,
            entry_status
        );
    }
    timer_value.ok_or(TransportError::Unsupported)
}

fn snapshot_request_checksum(sequence: u32, snapshot_id: u32) -> u32 {
    let mut checksum = SNAPSHOT_CHECKSUM_SEED;
    for word in [
        debug::MAGIC,
        debug::VERSION,
        READ_SNAPSHOT_ALLOWLISTED,
        snapshot_id,
        sequence,
    ] {
        checksum = snapshot_checksum_update(checksum, word);
    }
    checksum
}

fn snapshot_checksum_update(checksum: u32, word: u32) -> u32 {
    (checksum ^ word).rotate_left(5).wrapping_mul(0x9e37_79b1)
}

fn snapshot_word(data: &[u8], index: usize) -> Result<u32, TransportError> {
    let offset = index.checked_mul(4).ok_or(TransportError::Unsupported)?;
    let bytes: [u8; 4] = data
        .get(offset..offset + 4)
        .ok_or(TransportError::Unsupported)?
        .try_into()
        .map_err(|_| TransportError::Unsupported)?;
    Ok(u32::from_le_bytes(bytes))
}

struct GdbServer<'a, T: Rp1MemoryTransport> {
    transport: &'a mut T,
    seq: u32,
    packet: [u8; PACKET_BUF_LEN],
    reply: [u8; PACKET_BUF_LEN],
}

impl<'a, T> GdbServer<'a, T>
where
    T: Rp1MemoryTransport,
{
    fn new(transport: &'a mut T) -> Self {
        Self {
            transport,
            seq: 0,
            packet: [0; PACKET_BUF_LEN],
            reply: [0; PACKET_BUF_LEN],
        }
    }

    fn run(&mut self) -> ! {
        loop {
            match self.read_packet() {
                Some(len) => self.handle_packet(len),
                None => self.send_byte(b'-'),
            }
        }
    }

    fn handle_packet(&mut self, len: usize) {
        let mut packet_copy = [0u8; PACKET_BUF_LEN];
        packet_copy[..len].copy_from_slice(&self.packet[..len]);
        let packet = &packet_copy[..len];

        if packet == b"?" {
            self.send_packet(b"S05");
        } else if packet.starts_with(b"qSupported") {
            self.send_packet(b"PacketSize=200;qXfer:features:read-");
        } else if packet == b"g" {
            self.handle_read_regs();
        } else if packet.starts_with(b"m") {
            self.handle_read_mem(packet);
        } else if packet.starts_with(b"M") {
            self.handle_write_mem(packet);
        } else if packet == b"c" || packet == b"s" {
            let _ = self.command_no_payload(debug::command::CONTINUE);
            self.send_packet(b"S05");
        } else if packet == b"D" || packet == b"k" {
            self.send_packet(b"OK");
        } else if packet.starts_with(b"H") || packet.starts_with(b"qAttached") {
            self.send_packet(b"OK");
        } else if packet.starts_with(b"Z") || packet.starts_with(b"z") {
            self.send_packet(b"");
        } else {
            self.send_packet(b"");
        }
    }

    fn handle_read_regs(&mut self) {
        let mut regs = [0u8; 17 * 4];
        let mut vector = [0u8; 8];
        if self.read_mem32(0x2000_0000, &mut vector).is_ok() {
            regs[13 * 4..13 * 4 + 4].copy_from_slice(&vector[0..4]);
            regs[15 * 4..15 * 4 + 4].copy_from_slice(&vector[4..8]);
        }

        let mut pos = 0;
        for b in &regs {
            pos = push_hex_byte(&mut self.reply, pos, *b);
        }
        self.send_packet_from_reply(pos);
    }

    fn handle_read_mem(&mut self, packet: &[u8]) {
        let Some((addr, len)) = parse_addr_len(&packet[1..]) else {
            self.send_packet(b"E00");
            return;
        };
        if len > MAX_GDB_MEM || len * 2 > self.reply.len() {
            self.send_packet(b"E22");
            return;
        }

        if addr >= debug::MAILBOX_ADDR
            && addr.saturating_add(len as u32) <= debug::MAILBOX_ADDR + D1RP_WIRE_SIZE as u32
        {
            let mut data = [0u8; MAX_GDB_MEM];
            if self.read_mem32(addr, &mut data[..len]).is_err() {
                self.send_packet(b"E02");
                return;
            }
            let mut pos = 0usize;
            for b in &data[..len] {
                pos = push_hex_byte(&mut self.reply, pos, *b);
            }
            self.send_packet_from_reply(pos);
            return;
        }

        if self.write_u32(OFF_ARG0, addr).is_err()
            || self.write_u32(OFF_ARG1, len as u32).is_err()
            || self.command_no_payload(debug::command::READ_MEM).is_err()
        {
            self.send_packet(b"E01");
            return;
        }

        let mut data = [0u8; MAX_GDB_MEM];
        if self
            .read_mem32(debug::MAILBOX_ADDR + OFF_DATA, &mut data[..len])
            .is_err()
        {
            self.send_packet(b"E02");
            return;
        }

        let mut pos = 0usize;
        for b in &data[..len] {
            pos = push_hex_byte(&mut self.reply, pos, *b);
        }
        self.send_packet_from_reply(pos);
    }

    fn handle_write_mem(&mut self, packet: &[u8]) {
        let Some(colon) = find_byte(packet, b':') else {
            self.send_packet(b"E00");
            return;
        };
        let Some((addr, len)) = parse_addr_len(&packet[1..colon]) else {
            self.send_packet(b"E00");
            return;
        };
        if len > MAX_GDB_MEM || packet.len() < colon + 1 + len * 2 {
            self.send_packet(b"E22");
            return;
        }

        let mut data = [0u8; MAX_GDB_MEM];
        if decode_hex_bytes(&packet[colon + 1..colon + 1 + len * 2], &mut data[..len]).is_none() {
            self.send_packet(b"E00");
            return;
        }

        if self
            .write_mem32(debug::MAILBOX_ADDR + OFF_DATA, &data[..len])
            .is_err()
            || self.write_u32(OFF_ARG0, addr).is_err()
            || self.write_u32(OFF_ARG1, len as u32).is_err()
            || self.command_no_payload(debug::command::WRITE_MEM).is_err()
        {
            if self.write_mem32(addr, &data[..len]).is_err() {
                self.send_packet(b"E01");
                return;
            }
        }

        self.send_packet(b"OK");
    }

    fn command_no_payload(&mut self, command: u32) -> Result<(), ()> {
        self.seq = self.seq.wrapping_add(1);
        self.write_u32(OFF_COMMAND, command)?;
        self.write_u32(OFF_SEQ, self.seq)?;
        for _ in 0..100_000 {
            if self.read_u32(OFF_ACK)? == self.seq {
                return if self.read_u32(OFF_STATUS)? == debug::status::OK {
                    Ok(())
                } else {
                    Err(())
                };
            }
            crate::timer::delay_micros(10);
        }
        Err(())
    }

    fn read_u32(&mut self, off: u32) -> Result<u32, ()> {
        let mut bytes = [0u8; 4];
        self.read_mem32(debug::MAILBOX_ADDR + off, &mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn write_u32(&mut self, off: u32, value: u32) -> Result<(), ()> {
        self.write_mem32(debug::MAILBOX_ADDR + off, &value.to_le_bytes())
    }

    fn read_mem32(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), ()> {
        let mut off = 0usize;
        while off < buf.len() {
            let n = core::cmp::min(crate::rp1_bootstrap::RP1_CHUNK_SIZE, buf.len() - off);
            self.transport
                .read_mem(addr.wrapping_add(off as u32), &mut buf[off..off + n])
                .map_err(|_| ())?;
            off += n;
        }
        Ok(())
    }

    fn write_mem32(&mut self, addr: u32, data: &[u8]) -> Result<(), ()> {
        let mut off = 0usize;
        while off < data.len() {
            let n = core::cmp::min(crate::rp1_bootstrap::RP1_CHUNK_SIZE, data.len() - off);
            self.transport
                .write_mem(addr.wrapping_add(off as u32), &data[off..off + n])
                .map_err(|_| ())?;
            off += n;
        }
        Ok(())
    }

    fn read_packet(&mut self) -> Option<usize> {
        loop {
            if self.recv_byte() == b'$' {
                break;
            }
        }

        let mut len = 0usize;
        let mut checksum = 0u8;
        loop {
            let b = self.recv_byte();
            if b == b'#' {
                break;
            }
            if len >= self.packet.len() {
                return None;
            }
            self.packet[len] = b;
            len += 1;
            checksum = checksum.wrapping_add(b);
        }

        let got_hi = from_hex(self.recv_byte())?;
        let got_lo = from_hex(self.recv_byte())?;
        let got = (got_hi << 4) | got_lo;
        if got == checksum {
            self.send_byte(b'+');
            Some(len)
        } else {
            None
        }
    }

    fn send_packet(&mut self, data: &[u8]) {
        self.send_byte(b'$');
        let mut checksum = 0u8;
        for b in data {
            checksum = checksum.wrapping_add(*b);
            self.send_byte(*b);
        }
        self.send_byte(b'#');
        self.send_byte(HEX[(checksum >> 4) as usize]);
        self.send_byte(HEX[(checksum & 0x0f) as usize]);
    }

    fn send_packet_from_reply(&mut self, len: usize) {
        self.send_byte(b'$');
        let mut checksum = 0u8;
        for idx in 0..len {
            let b = self.reply[idx];
            checksum = checksum.wrapping_add(b);
            self.send_byte(b);
        }
        self.send_byte(b'#');
        self.send_byte(HEX[(checksum >> 4) as usize]);
        self.send_byte(HEX[(checksum & 0x0f) as usize]);
    }

    fn send_byte(&self, b: u8) {
        uart_putc_raw(b);
    }

    fn recv_byte(&self) -> u8 {
        uart_getc_raw()
    }
}

struct MailboxCommand<'a, T: Rp1MemoryTransport> {
    transport: &'a mut T,
    seq: u32,
}

impl<T> MailboxCommand<'_, T>
where
    T: Rp1MemoryTransport,
{
    fn command_no_payload(&mut self, command: u32) -> Result<(), TransportError> {
        if self.command_with_args(command, 0, 0)? == STATUS_OK {
            Ok(())
        } else {
            Err(TransportError::Unsupported)
        }
    }

    fn command_with_args(
        &mut self,
        command: u32,
        arg0: u32,
        arg1: u32,
    ) -> Result<u32, TransportError> {
        self.seq = self.seq.wrapping_add(1);
        self.write_u32(OFF_ARG0, arg0)?;
        self.write_u32(OFF_ARG1, arg1)?;
        self.write_u32(OFF_COMMAND, command)?;
        self.write_u32(OFF_SEQ, self.seq)?;
        for _ in 0..100_000 {
            if self.read_u32(OFF_ACK)? == self.seq {
                return self.read_u32(OFF_STATUS);
            }
            crate::timer::delay_micros(10);
        }
        Err(TransportError::Timeout)
    }

    fn read_u32(&mut self, off: u32) -> Result<u32, TransportError> {
        let mut bytes = [0u8; 4];
        self.transport
            .read_mem(debug::MAILBOX_ADDR + off, &mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn write_u32(&mut self, off: u32, value: u32) -> Result<(), TransportError> {
        self.transport
            .write_mem(debug::MAILBOX_ADDR + off, &value.to_le_bytes())
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

fn parse_addr_len(input: &[u8]) -> Option<(u32, usize)> {
    let comma = find_byte(input, b',')?;
    let addr = parse_hex_u32(&input[..comma])?;
    let len = parse_hex_u32(&input[comma + 1..])? as usize;
    Some((addr, len))
}

fn parse_hex_u32(input: &[u8]) -> Option<u32> {
    let mut value = 0u32;
    for b in input {
        value = value
            .checked_mul(16)?
            .checked_add(u32::from(from_hex(*b)?))?;
    }
    Some(value)
}

fn decode_hex_bytes(input: &[u8], out: &mut [u8]) -> Option<()> {
    if input.len() != out.len() * 2 {
        return None;
    }
    for idx in 0..out.len() {
        let hi = from_hex(input[idx * 2])?;
        let lo = from_hex(input[idx * 2 + 1])?;
        out[idx] = (hi << 4) | lo;
    }
    Some(())
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn push_hex_byte(out: &mut [u8], pos: usize, byte: u8) -> usize {
    out[pos] = HEX[(byte >> 4) as usize];
    out[pos + 1] = HEX[(byte & 0x0f) as usize];
    pos + 2
}

fn find_byte(input: &[u8], needle: u8) -> Option<usize> {
    input.iter().position(|b| *b == needle)
}

fn uart_getc_raw() -> u8 {
    loop {
        if (uart_read32(UART_FR) & UART_FR_RXFE) == 0 {
            return uart_read32(UART_DR) as u8;
        }
    }
}

fn uart_putc_raw(b: u8) {
    while (uart_read32(UART_FR) & UART_FR_TXFF) != 0 {}
    uart_write32(UART_DR, u32::from(b));
}

fn uart_read32(off: usize) -> u32 {
    unsafe { core::ptr::read_volatile((UART_BASE + off) as *const u32) }
}

fn uart_write32(off: usize, value: u32) {
    unsafe { core::ptr::write_volatile((UART_BASE + off) as *mut u32, value) }
}
