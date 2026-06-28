use crate::rp1_bootstrap::{Rp1Bootstrap, Rp1I2cBus};
use rp1_abi::debug;

const PACKET_BUF_LEN: usize = 768;
const MAX_GDB_MEM: usize = 256;

const OFF_SEQ: u32 = 16;
const OFF_ACK: u32 = 20;
const OFF_COMMAND: u32 = 32;
const OFF_ARG0: u32 = 36;
const OFF_ARG1: u32 = 40;
const OFF_STATUS: u32 = 44;
const OFF_REGS: u32 = 48;
const OFF_DATA_LEN: u32 = 120;
const OFF_DATA: u32 = 124;

pub fn serve<I2C>(bootstrap: &mut Rp1Bootstrap<I2C>) -> !
where
    I2C: Rp1I2cBus,
{
    crate::logln!("[RP1GDB] RP1 GDB debug stub mode active");
    crate::logln!("[RP1GDB] attach with: target remote <serial-device>");

    let mut server = GdbServer::new(bootstrap);
    server.run()
}

struct GdbServer<'a, I2C> {
    bootstrap: &'a mut Rp1Bootstrap<I2C>,
    seq: u32,
    packet: [u8; PACKET_BUF_LEN],
    reply: [u8; PACKET_BUF_LEN],
}

impl<'a, I2C> GdbServer<'a, I2C>
where
    I2C: Rp1I2cBus,
{
    fn new(bootstrap: &'a mut Rp1Bootstrap<I2C>) -> Self {
        Self {
            bootstrap,
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
        let packet = &self.packet[..len];
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
            self.command_no_payload(debug::command::CONTINUE);
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
        if self.command_no_payload(debug::command::GET_REGS).is_err() {
            self.send_packet(b"E01");
            return;
        }

        let mut regs = [0u8; debug::MAILBOX_REG_COUNT * 4];
        if self.read_mem32(debug::MAILBOX_ADDR + OFF_REGS, &mut regs).is_err() {
            self.send_packet(b"E02");
            return;
        }

        let mut out = [0u8; 17 * 8];
        let mut pos = 0;
        for b in &regs[..17 * 4] {
            pos = push_hex_byte(&mut out, pos, *b);
        }
        self.send_packet(&out[..pos]);
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
        let len = pos;
        self.send_packet_from_reply(len);
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

        if self.write_mem32(debug::MAILBOX_ADDR + OFF_DATA, &data[..len]).is_err()
            || self.write_u32(OFF_ARG0, addr).is_err()
            || self.write_u32(OFF_ARG1, len as u32).is_err()
            || self.command_no_payload(debug::command::WRITE_MEM).is_err()
        {
            self.send_packet(b"E01");
            return;
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
            self.bootstrap
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
            self.bootstrap
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
        crate::uart::putc_raw(b);
    }

    fn recv_byte(&self) -> u8 {
        crate::uart::getc_raw()
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
        value = value.checked_mul(16)?.checked_add(u32::from(from_hex(*b)?))?;
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
