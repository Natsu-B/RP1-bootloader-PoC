use io_api::ethernet::{EthernetFrameIo, MacAddr};
use net::Ipv4Addr;
use net::tftp::TftpClock;

const DHCP_CLIENT_PORT: u16 = 68;
const DHCP_SERVER_PORT: u16 = 67;
const DHCP_TIMEOUT_US: u64 = 3_000_000;
const DHCP_MAX_RETRIES: usize = 3;
const MAX_FRAME_LEN: usize = 1536;
const DHCP_PACKET_MAX: usize = 576;

const DHCP_BOOTREQUEST: u8 = 1;
const DHCP_BOOTREPLY: u8 = 2;
const DHCP_HTYPE_ETHERNET: u8 = 1;
const DHCP_HLEN_ETHERNET: u8 = 6;
const DHCP_MAGIC_COOKIE: [u8; 4] = [99, 130, 83, 99];

const OPT_SUBNET_MASK: u8 = 1;
const OPT_ROUTER: u8 = 3;
const OPT_REQUESTED_IP: u8 = 50;
const OPT_MESSAGE_TYPE: u8 = 53;
const OPT_SERVER_IDENTIFIER: u8 = 54;
const OPT_PARAMETER_REQUEST_LIST: u8 = 55;
const OPT_MAX_DHCP_MESSAGE_SIZE: u8 = 57;
const OPT_VENDOR_CLASS_IDENTIFIER: u8 = 60;
const OPT_TFTP_SERVER_NAME: u8 = 66;
const OPT_CLIENT_SYSTEM_ARCH: u8 = 93;
const OPT_CLIENT_NDI: u8 = 94;
const OPT_CLIENT_GUID: u8 = 97;
const OPT_PAD: u8 = 0;
const OPT_END: u8 = 255;

const DHCP_DISCOVER: u8 = 1;
const DHCP_OFFER: u8 = 2;
const DHCP_REQUEST: u8 = 3;
const DHCP_ACK: u8 = 5;

const BROADCAST_MAC: MacAddr = MacAddr([0xff; 6]);
const ZERO_IP: Ipv4Addr = [0, 0, 0, 0];
const BROADCAST_IP: Ipv4Addr = [255, 255, 255, 255];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DhcpError {
    Encode,
    Transmit,
    Timeout,
    InvalidPacket,
    UnexpectedMessage,
    NoTftpServer,
}

#[derive(Clone, Copy)]
pub struct DhcpLease {
    pub client_ip: Ipv4Addr,
    pub subnet_mask: Option<Ipv4Addr>,
    pub router: Option<Ipv4Addr>,
    pub server_id: Option<Ipv4Addr>,
    pub siaddr: Option<Ipv4Addr>,
    pub opt66_tftp_server: Option<Ipv4Addr>,
    pub reply_src_ip: Ipv4Addr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TftpServerSource {
    DhcpOpt66,
    DhcpSiaddr,
    DhcpServerId,
    DhcpReplySource,
}

#[derive(Clone, Copy)]
pub struct NetworkBootLease {
    pub client_ip: Ipv4Addr,
    pub subnet_mask: Option<Ipv4Addr>,
    pub router: Option<Ipv4Addr>,
    pub tftp_server_ip: Ipv4Addr,
    pub tftp_server_source: TftpServerSource,
}

#[derive(Clone, Copy, Default)]
struct DhcpOptions {
    message_type: Option<u8>,
    subnet_mask: Option<Ipv4Addr>,
    router: Option<Ipv4Addr>,
    server_id: Option<Ipv4Addr>,
    opt66_tftp_server: Option<Ipv4Addr>,
}

#[derive(Clone, Copy)]
struct DhcpReply {
    message_type: u8,
    lease: DhcpLease,
}

pub fn dhcp_acquire(
    eth: &mut dyn EthernetFrameIo,
    clock: &dyn TftpClock,
) -> Result<NetworkBootLease, DhcpError> {
    let local_mac = eth.mac_addr();
    let xid = xid_from_mac(local_mac);
    crate::logln!("[NETBOOT] DHCP start");
    crate::logln!("[DHCP] discover xid=0x{:08x}", xid);

    let discover = encode_dhcp_message(local_mac, xid, DhcpRequestKind::Discover, None)?;
    send_dhcp_packet(eth, &discover)?;
    let offer = wait_for_dhcp_reply(eth, clock, local_mac, xid, DHCP_OFFER, &discover)?;
    log_dhcp_reply("offer", &offer.lease);

    let request = encode_dhcp_message(
        local_mac,
        xid,
        DhcpRequestKind::Request {
            requested_ip: offer.lease.client_ip,
            server_id: offer.lease.server_id,
        },
        Some(&offer.lease),
    )?;
    crate::logln!(
        "[DHCP] request yiaddr={}",
        Ipv4Display(offer.lease.client_ip)
    );
    send_dhcp_packet(eth, &request)?;
    let ack = wait_for_dhcp_reply(eth, clock, local_mac, xid, DHCP_ACK, &request)?;
    log_dhcp_reply("ack", &ack.lease);

    let lease = select_network_boot_lease(&ack.lease)?;
    crate::logln!(
        "[NETBOOT] client_ip={} subnet={} router={}",
        Ipv4Display(lease.client_ip),
        OptionalIpv4Display(lease.subnet_mask),
        OptionalIpv4Display(lease.router)
    );
    crate::logln!(
        "[NETBOOT] selected_tftp={} source={}",
        Ipv4Display(lease.tftp_server_ip),
        lease.tftp_server_source.as_str()
    );
    Ok(lease)
}

fn wait_for_dhcp_reply(
    eth: &mut dyn EthernetFrameIo,
    clock: &dyn TftpClock,
    local_mac: MacAddr,
    xid: u32,
    expected_message_type: u8,
    retry_packet: &[u8],
) -> Result<DhcpReply, DhcpError> {
    let mut rx = [0u8; MAX_FRAME_LEN];
    for retry in 0..=DHCP_MAX_RETRIES {
        if retry != 0 {
            send_dhcp_packet(eth, retry_packet)?;
        }
        let start = clock.now_us();
        while clock.now_us().wrapping_sub(start) < DHCP_TIMEOUT_US {
            let Some(frame_len) = eth.try_recv_frame(&mut rx) else {
                continue;
            };
            if frame_len > rx.len() {
                continue;
            }
            let Ok(datagram) = net::parse_udp_ipv4_frame(&rx[..frame_len]) else {
                continue;
            };
            if datagram.src_port != DHCP_SERVER_PORT || datagram.dst_port != DHCP_CLIENT_PORT {
                continue;
            }
            let reply = match parse_dhcp_reply(datagram.payload, datagram.src_ip, xid, local_mac) {
                Ok(reply) => reply,
                Err(DhcpError::InvalidPacket | DhcpError::UnexpectedMessage) => continue,
                Err(err) => return Err(err),
            };
            if reply.message_type == expected_message_type {
                return Ok(reply);
            }
        }
    }
    Err(DhcpError::Timeout)
}

fn send_dhcp_packet(eth: &mut dyn EthernetFrameIo, payload: &[u8]) -> Result<(), DhcpError> {
    let mut frame = [0u8; MAX_FRAME_LEN];
    let len = net::encode_udp_ipv4_frame(
        &mut frame,
        eth.mac_addr(),
        BROADCAST_MAC,
        ZERO_IP,
        BROADCAST_IP,
        DHCP_CLIENT_PORT,
        DHCP_SERVER_PORT,
        payload,
    )
    .map_err(|_| DhcpError::Encode)?;
    if eth.try_send_frame(&frame[..len]) {
        Ok(())
    } else {
        Err(DhcpError::Transmit)
    }
}

enum DhcpRequestKind {
    Discover,
    Request {
        requested_ip: Ipv4Addr,
        server_id: Option<Ipv4Addr>,
    },
}

fn encode_dhcp_message(
    mac: MacAddr,
    xid: u32,
    kind: DhcpRequestKind,
    _offer: Option<&DhcpLease>,
) -> Result<[u8; DHCP_PACKET_MAX], DhcpError> {
    let mut packet = [0u8; DHCP_PACKET_MAX];
    packet[0] = DHCP_BOOTREQUEST;
    packet[1] = DHCP_HTYPE_ETHERNET;
    packet[2] = DHCP_HLEN_ETHERNET;
    packet[4..8].copy_from_slice(&xid.to_be_bytes());
    packet[10..12].copy_from_slice(&0x8000u16.to_be_bytes());
    packet[28..34].copy_from_slice(&mac.0);
    packet[236..240].copy_from_slice(&DHCP_MAGIC_COOKIE);

    let mut opts = OptionsWriter::new(&mut packet[240..]);
    match kind {
        DhcpRequestKind::Discover => {
            opts.option_bytes(OPT_MESSAGE_TYPE, &[DHCP_DISCOVER])?;
        }
        DhcpRequestKind::Request {
            requested_ip,
            server_id,
        } => {
            opts.option_bytes(OPT_MESSAGE_TYPE, &[DHCP_REQUEST])?;
            opts.option_bytes(OPT_REQUESTED_IP, &requested_ip)?;
            if let Some(server_id) = server_id {
                opts.option_bytes(OPT_SERVER_IDENTIFIER, &server_id)?;
            }
        }
    }
    opts.option_bytes(
        OPT_PARAMETER_REQUEST_LIST,
        &[1, 3, 28, 51, 54, 58, 59, 60, 66, 67, 43],
    )?;
    opts.option_bytes(OPT_MAX_DHCP_MESSAGE_SIZE, &1500u16.to_be_bytes())?;
    opts.option_bytes(
        OPT_VENDOR_CLASS_IDENTIFIER,
        b"PXEClient:Arch:00000:UNDI:002001",
    )?;
    opts.option_bytes(OPT_CLIENT_SYSTEM_ARCH, &0u16.to_be_bytes())?;
    opts.option_bytes(OPT_CLIENT_NDI, &[1, 2, 1])?;
    opts.option_bytes(OPT_CLIENT_GUID, &client_guid_from_mac(mac))?;
    opts.end()?;
    Ok(packet)
}

struct OptionsWriter<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> OptionsWriter<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn option_bytes(&mut self, code: u8, value: &[u8]) -> Result<(), DhcpError> {
        let len = u8::try_from(value.len()).map_err(|_| DhcpError::Encode)?;
        let end = self
            .pos
            .checked_add(2)
            .and_then(|pos| pos.checked_add(value.len()))
            .ok_or(DhcpError::Encode)?;
        if end > self.buf.len() {
            return Err(DhcpError::Encode);
        }
        self.buf[self.pos] = code;
        self.buf[self.pos + 1] = len;
        self.buf[self.pos + 2..end].copy_from_slice(value);
        self.pos = end;
        Ok(())
    }

    fn end(&mut self) -> Result<(), DhcpError> {
        if self.pos >= self.buf.len() {
            return Err(DhcpError::Encode);
        }
        self.buf[self.pos] = OPT_END;
        Ok(())
    }
}

fn parse_dhcp_reply(
    payload: &[u8],
    reply_src_ip: Ipv4Addr,
    expected_xid: u32,
    expected_mac: MacAddr,
) -> Result<DhcpReply, DhcpError> {
    if payload.len() < 240
        || payload[0] != DHCP_BOOTREPLY
        || payload[1] != DHCP_HTYPE_ETHERNET
        || payload[2] != DHCP_HLEN_ETHERNET
        || payload[236..240] != DHCP_MAGIC_COOKIE
    {
        return Err(DhcpError::InvalidPacket);
    }
    let xid = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
    if xid != expected_xid || payload[28..34] != expected_mac.0 {
        return Err(DhcpError::UnexpectedMessage);
    }
    let options = parse_dhcp_options(&payload[240..])?;
    let Some(message_type) = options.message_type else {
        return Err(DhcpError::InvalidPacket);
    };
    let client_ip = read_ipv4(payload, 16).ok_or(DhcpError::InvalidPacket)?;
    let siaddr = nonzero_ip(read_ipv4(payload, 20).ok_or(DhcpError::InvalidPacket)?);
    Ok(DhcpReply {
        message_type,
        lease: DhcpLease {
            client_ip,
            subnet_mask: options.subnet_mask,
            router: options.router,
            server_id: options.server_id,
            siaddr,
            opt66_tftp_server: options.opt66_tftp_server,
            reply_src_ip,
        },
    })
}

fn parse_dhcp_options(mut options: &[u8]) -> Result<DhcpOptions, DhcpError> {
    let mut parsed = DhcpOptions::default();
    while let Some((&code, rest)) = options.split_first() {
        options = rest;
        match code {
            OPT_PAD => {}
            OPT_END => return Ok(parsed),
            _ => {
                let Some((&len, rest)) = options.split_first() else {
                    return Err(DhcpError::InvalidPacket);
                };
                let len = usize::from(len);
                let Some(value) = rest.get(..len) else {
                    return Err(DhcpError::InvalidPacket);
                };
                match code {
                    OPT_MESSAGE_TYPE => {
                        if value.len() == 1 {
                            parsed.message_type = Some(value[0]);
                        }
                    }
                    OPT_SUBNET_MASK => parsed.subnet_mask = parse_ipv4_option(value),
                    OPT_ROUTER => parsed.router = parse_ipv4_option(value),
                    OPT_SERVER_IDENTIFIER => parsed.server_id = parse_ipv4_option(value),
                    OPT_TFTP_SERVER_NAME => parsed.opt66_tftp_server = parse_ascii_ipv4(value),
                    _ => {}
                }
                options = &rest[len..];
            }
        }
    }
    Ok(parsed)
}

fn select_network_boot_lease(lease: &DhcpLease) -> Result<NetworkBootLease, DhcpError> {
    let (tftp_server_ip, tftp_server_source) =
        if let Some(ip) = nonzero_ip_opt(lease.opt66_tftp_server) {
            (ip, TftpServerSource::DhcpOpt66)
        } else if let Some(ip) = nonzero_ip_opt(lease.siaddr) {
            (ip, TftpServerSource::DhcpSiaddr)
        } else if let Some(ip) = nonzero_ip_opt(lease.server_id) {
            (ip, TftpServerSource::DhcpServerId)
        } else if let Some(ip) = nonzero_ip(lease.reply_src_ip) {
            (ip, TftpServerSource::DhcpReplySource)
        } else {
            return Err(DhcpError::NoTftpServer);
        };
    Ok(NetworkBootLease {
        client_ip: lease.client_ip,
        subnet_mask: lease.subnet_mask,
        router: lease.router,
        tftp_server_ip,
        tftp_server_source,
    })
}

impl TftpServerSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DhcpOpt66 => "dhcp-opt66",
            Self::DhcpSiaddr => "dhcp-siaddr",
            Self::DhcpServerId => "dhcp-server-id",
            Self::DhcpReplySource => "dhcp-reply-source",
        }
    }
}

fn log_dhcp_reply(label: &str, lease: &DhcpLease) {
    crate::logln!(
        "[DHCP] {} yiaddr={} siaddr={} server_id={} opt66={} src={}",
        label,
        Ipv4Display(lease.client_ip),
        OptionalIpv4Display(lease.siaddr),
        OptionalIpv4Display(lease.server_id),
        OptionalIpv4Display(lease.opt66_tftp_server),
        Ipv4Display(lease.reply_src_ip)
    );
}

fn xid_from_mac(mac: MacAddr) -> u32 {
    u32::from_be_bytes([mac.0[2], mac.0[3], mac.0[4], mac.0[5]]) ^ 0x5250_3101
}

fn client_guid_from_mac(mac: MacAddr) -> [u8; 17] {
    let mut guid = [0u8; 17];
    guid[0] = 0;
    guid[11..17].copy_from_slice(&mac.0);
    guid
}

fn read_ipv4(bytes: &[u8], offset: usize) -> Option<Ipv4Addr> {
    let bytes = bytes.get(offset..offset + 4)?;
    Some([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn parse_ipv4_option(bytes: &[u8]) -> Option<Ipv4Addr> {
    let bytes = bytes.get(..4)?;
    Some([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn nonzero_ip_opt(ip: Option<Ipv4Addr>) -> Option<Ipv4Addr> {
    ip.and_then(nonzero_ip)
}

fn nonzero_ip(ip: Ipv4Addr) -> Option<Ipv4Addr> {
    if ip == ZERO_IP { None } else { Some(ip) }
}

fn parse_ascii_ipv4(bytes: &[u8]) -> Option<Ipv4Addr> {
    let bytes = trim_ascii_nul_space(bytes);
    if bytes.is_empty() {
        return None;
    }

    let mut octets = [0u8; 4];
    let mut idx = 0usize;
    let mut value = 0u16;
    let mut saw_digit = false;

    for &b in bytes {
        match b {
            b'0'..=b'9' => {
                saw_digit = true;
                value = value.checked_mul(10)?.checked_add(u16::from(b - b'0'))?;
                if value > u16::from(u8::MAX) {
                    return None;
                }
            }
            b'.' => {
                if !saw_digit || idx >= 3 {
                    return None;
                }
                octets[idx] = value as u8;
                idx += 1;
                value = 0;
                saw_digit = false;
            }
            _ => return None,
        }
    }

    if !saw_digit || idx != 3 {
        return None;
    }
    octets[idx] = value as u8;
    Some(octets)
}

fn trim_ascii_nul_space(mut bytes: &[u8]) -> &[u8] {
    while let Some((&last, rest)) = bytes.split_last() {
        if last == 0 || last == b' ' || last == b'\n' || last == b'\r' || last == b'\t' {
            bytes = rest;
        } else {
            break;
        }
    }
    bytes
}

struct Ipv4Display(Ipv4Addr);

impl core::fmt::Display for Ipv4Display {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}.{}.{}", self.0[0], self.0[1], self.0[2], self.0[3])
    }
}

struct OptionalIpv4Display(Option<Ipv4Addr>);

impl core::fmt::Display for OptionalIpv4Display {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            Some(ip) => Ipv4Display(ip).fmt(f),
            None => f.write_str("missing"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_message_type_option() {
        let opts = parse_dhcp_options(&[OPT_MESSAGE_TYPE, 1, DHCP_OFFER, OPT_END]).unwrap();
        assert_eq!(opts.message_type, Some(DHCP_OFFER));
    }

    #[test]
    fn parses_server_identifier_option() {
        let opts =
            parse_dhcp_options(&[OPT_SERVER_IDENTIFIER, 4, 192, 168, 50, 1, OPT_END]).unwrap();
        assert_eq!(opts.server_id, Some([192, 168, 50, 1]));
    }

    #[test]
    fn parses_option_66_ascii_ipv4() {
        let opts = parse_dhcp_options(&[
            OPT_TFTP_SERVER_NAME,
            13,
            b'1',
            b'9',
            b'2',
            b'.',
            b'1',
            b'6',
            b'8',
            b'.',
            b'5',
            b'0',
            b'.',
            b'1',
            0,
            OPT_END,
        ])
        .unwrap();
        assert_eq!(opts.opt66_tftp_server, Some([192, 168, 50, 1]));
    }

    #[test]
    fn selects_siaddr_fallback() {
        let lease = DhcpLease {
            client_ip: [192, 168, 50, 25],
            subnet_mask: None,
            router: None,
            server_id: None,
            siaddr: Some([192, 168, 50, 1]),
            opt66_tftp_server: None,
            reply_src_ip: [192, 168, 50, 254],
        };
        let selected = select_network_boot_lease(&lease).unwrap();
        assert_eq!(selected.tftp_server_ip, [192, 168, 50, 1]);
        assert_eq!(selected.tftp_server_source, TftpServerSource::DhcpSiaddr);
    }

    #[test]
    fn selects_tftp_priority() {
        let lease = DhcpLease {
            client_ip: [192, 168, 50, 25],
            subnet_mask: None,
            router: None,
            opt66_tftp_server: Some([192, 168, 50, 66]),
            siaddr: Some([192, 168, 50, 2]),
            server_id: Some([192, 168, 50, 54]),
            reply_src_ip: [192, 168, 50, 1],
        };
        let selected = select_network_boot_lease(&lease).unwrap();
        assert_eq!(selected.tftp_server_ip, [192, 168, 50, 66]);
        assert_eq!(selected.tftp_server_source, TftpServerSource::DhcpOpt66);
    }

    #[test]
    fn malformed_option_66_falls_back() {
        let opts = parse_dhcp_options(&[
            OPT_TFTP_SERVER_NAME,
            8,
            b'n',
            b'o',
            b't',
            b'-',
            b'i',
            b'p',
            b'v',
            b'4',
            OPT_SERVER_IDENTIFIER,
            4,
            192,
            168,
            50,
            54,
            OPT_END,
        ])
        .unwrap();
        let lease = DhcpLease {
            client_ip: [192, 168, 50, 25],
            subnet_mask: None,
            router: None,
            opt66_tftp_server: opts.opt66_tftp_server,
            siaddr: None,
            server_id: opts.server_id,
            reply_src_ip: [192, 168, 50, 1],
        };
        let selected = select_network_boot_lease(&lease).unwrap();
        assert_eq!(selected.tftp_server_ip, [192, 168, 50, 54]);
        assert_eq!(selected.tftp_server_source, TftpServerSource::DhcpServerId);
    }
}
