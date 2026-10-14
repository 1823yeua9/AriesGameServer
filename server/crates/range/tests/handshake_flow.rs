//! loopback harness: a scripted client drives the stateless handshake, hello,
//! nmt challenge/welcome, join and session acks through RangeServer, and
//! decodes every reply.

use std::net::SocketAddr;

use aries_bits::{BitReader, BitWriter, Bits};
use aries_net::AresGcm;
use aries_net::client::{open_server_datagram, seal_client_packet};
use aries_net::control::{inspect_first_bunch, read_packet_header};
use aries_net::handshake::{self, CHALLENGE_BITS};
use aries_range::{KeySource, Outbox, RangeConfig, RangeServer, SessionKeys};

const CLIENT_KEY: [u8; 32] = [0x20; 32];
const SERVER_KEY: [u8; 32] = [0x40; 32];
const STATELESS: u32 = 12;

struct FixedKeys;

impl KeySource for FixedKeys {
    fn read_keys(&mut self) -> Result<SessionKeys, String> {
        Ok(SessionKeys { client_send: CLIENT_KEY, server_send: SERVER_KEY })
    }
}

struct Client {
    gcm: AresGcm,
    server_gcm: AresGcm,
    sequence: u32,
    acked: u32,
    counter: u32,
}

fn packed(w: &mut BitWriter, value: u64) {
    w.append_packed_uint(value);
}

impl Client {
    fn header(&mut self, w: &mut BitWriter) {
        w.append_u32((self.sequence << 18) | (self.acked << 4));
        w.append_u32(0);
        w.append_bits(1, 1);
        w.append_bits(0, 11);
        self.sequence = (self.sequence + 1) & 0x3FFF;
    }

    /// an ack-only 77-bit packet.
    fn short(&mut self) -> Bits {
        let mut w = BitWriter::new();
        self.header(&mut w);
        w.append_bits(1, 1);
        w.finish().unwrap()
    }

    /// a reliable control bunch on channel 0 with an nmt message, padded to `total` bits.
    fn control(&mut self, message_type: u8, open: bool, total: usize) -> Bits {
        let mut w = BitWriter::new();
        self.header(&mut w);
        w.append_bits(1, 1); // control
        w.append_bool(open);
        w.append_bits(0, 1); // close
        w.append_bits(0, 1); // paused
        w.append_bits(1, 1); // reliable
        packed(&mut w, 0);
        w.append_bits(0, 4);
        w.append_bits(100, 12);
        w.append_bits(1, 1);
        packed(&mut w, 2);
        let body_bits = total - 1 - w.bit_count() - 13;
        w.append_bits(body_bits as u64, 13);
        w.append_bits(message_type as u64, 8);
        for _ in 8..body_bits {
            w.append_bits(0, 1);
        }
        w.append_bits(1, 1);
        let bits = w.finish().unwrap();
        assert_eq!(bits.bit_count, total);
        bits
    }

    fn seal(&mut self, packet: &Bits) -> Vec<u8> {
        self.counter += 1;
        seal_client_packet(packet, self.counter, STATELESS, &self.gcm)
    }
}

fn initial_datagram() -> Vec<u8> {
    let mut w = BitWriter::new();
    w.append_bits(0, 64);
    w.append_bits(1, 2);
    w.append_bits(3, 3);
    w.append_bits(1, 1);
    w.append_bits(0, 1);
    w.append_bits(4, 8);
    w.append_bits(4, 8);
    w.append_bits(0, 8);
    w.append_bits(1, 8);
    w.append_u32(0x1234);
    w.append_bits(0, 16);
    w.append_bits(0, 1);
    w.append_f64(0.0);
    for _ in 0..(160 + 128) / 32 {
        w.append_u32(0);
    }
    w.append_bits(1, 1);
    w.finish().unwrap().bytes
}

fn response_datagram(challenge: &[u8]) -> Vec<u8> {
    // echo the challenge's timestamp and cookie back as a type-2 response
    let mut r = BitReader::new(&challenge[4..], (challenge.len() - 4) * 8);
    r.set_position(2 + 3 + 2 + 8 + 8 + 8 + 8 + 32 + 16 + 1);
    let timestamp = r.read_f64();
    let cookie: Vec<u64> = (0..20).map(|_| r.read_bits(8)).collect();
    let mut w = BitWriter::new();
    w.append_bits(0, 64);
    w.append_bits(1, 2);
    w.append_bits(3, 3);
    w.append_bits(1, 1);
    w.append_bits(0, 1);
    w.append_bits(4, 8);
    w.append_bits(4, 8);
    w.append_bits(2, 8);
    w.append_bits(2, 8);
    w.append_u32(0x1234);
    w.append_bits(0, 16);
    w.append_bits(0, 1);
    w.append_f64(timestamp);
    for byte in cookie {
        w.append_bits(byte, 8);
    }
    for _ in 0..4 {
        w.append_u32(0);
    }
    w.append_bits(1, 1);
    w.finish().unwrap().bytes
}

#[test]
fn handshake_through_session_acks() {
    let source: SocketAddr = "127.0.0.1:50506".parse().unwrap();
    let mut server = RangeServer::new(RangeConfig::default(), Box::new(FixedKeys), None);
    let mut client = Client { gcm: AresGcm::new(&CLIENT_KEY), server_gcm: AresGcm::new(&SERVER_KEY), sequence: 8000, acked: 11195, counter: 700 };
    let mut out = Outbox::default();
    let mut now = 100.0;

    // stateless handshake
    server.handle_datagram(now, source, &initial_datagram(), &mut out);
    let (_, challenge) = out.datagrams.pop().expect("challenge");
    assert_eq!(challenge.len(), 4 + CHALLENGE_BITS.div_ceil(8));
    server.handle_datagram(now, source, &response_datagram(&challenge), &mut out);
    let (_, ack) = out.datagrams.pop().expect("ack");
    let ack_packet = handshake::parse_handshake(&[vec![0u8; 4], ack.clone()].concat()).ok();
    assert!(ack_packet.is_none() || ack_packet.unwrap().packet_type == 3);
    assert!(out.datagrams.is_empty());

    // hello (232 bits) is stored, nothing sent
    let hello = client.control(0, true, 232);
    let datagram = client.seal(&hello);
    server.handle_datagram(now, source, &datagram, &mut out);
    assert!(out.datagrams.is_empty());

    // 77-bit packet -> nmt challenge with S = client's acked + 1
    let short = client.short();
    let datagram = client.seal(&short);
    server.handle_datagram(now, source, &datagram, &mut out);
    let (_, reply) = out.datagrams.pop().expect("nmt challenge");
    let (packet, stateless) = open_server_datagram(&reply, &client.server_gcm).expect("opens");
    assert_eq!(stateless, STATELESS);
    let header = read_packet_header(&packet).unwrap();
    assert_eq!(header.sequence, 11196);
    let bunch = inspect_first_bunch(&packet).expect("control bunch");
    assert_eq!((bunch.channel_index, bunch.control_message_type, bunch.is_open), (0, Some(3), false));
    assert_eq!(bunch.channel_sequence, Some(11197 & 0xFFF));

    // login -> welcome
    now += 0.5;
    let login = client.control(5, false, 600);
    let datagram = client.seal(&login);
    server.handle_datagram(now, source, &datagram, &mut out);
    let (_, reply) = out.datagrams.pop().expect("welcome");
    let (packet, _) = open_server_datagram(&reply, &client.server_gcm).unwrap();
    assert_eq!(read_packet_header(&packet).unwrap().sequence, 11197);
    assert_eq!(inspect_first_bunch(&packet).unwrap().control_message_type, Some(1));

    // join -> join ack (77-bit ack-only, mask 3)
    let join = client.control(9, false, 142);
    let datagram = client.seal(&join);
    server.handle_datagram(now, source, &datagram, &mut out);
    let (_, reply) = out.datagrams.pop().expect("join ack");
    let (packet, _) = open_server_datagram(&reply, &client.server_gcm).unwrap();
    assert_eq!(packet.bit_count, 77);
    let header = read_packet_header(&packet).unwrap();
    assert_eq!((header.sequence, header.history), (11198, 3));

    // inside 0.1 s: no session ack; after: one
    now += 0.05;
    let short = client.short();
    let datagram = client.seal(&short);
    server.handle_datagram(now, source, &datagram, &mut out);
    assert!(out.datagrams.is_empty());
    now += 0.06;
    let short = client.short();
    let datagram = client.seal(&short);
    server.handle_datagram(now, source, &datagram, &mut out);
    let (_, reply) = out.datagrams.pop().expect("session ack");
    let (packet, _) = open_server_datagram(&reply, &client.server_gcm).unwrap();
    assert_eq!(packet.bit_count, 77);
    assert_eq!(read_packet_header(&packet).unwrap().sequence, 11199);
    let conn = &server.connections[&source];
    assert_eq!(conn.base_sequence, 11197 & 0xFFF);
    assert_eq!(conn.next_server_sequence, 11200);
}
