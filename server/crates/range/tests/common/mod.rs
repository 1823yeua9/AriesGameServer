//! the scripted client shared by the harness tests.
#![allow(dead_code)]

use std::net::SocketAddr;

use aries_bits::{BitReader, BitWriter, Bits};
use aries_net::AresGcm;
use aries_net::client::seal_client_packet;
use aries_range::{KeySource, Outbox, RangeServer, SessionKeys};

pub const CLIENT_KEY: [u8; 32] = [0x20; 32];
pub const SERVER_KEY: [u8; 32] = [0x40; 32];
pub const STATELESS: u32 = 12;

pub struct FixedKeys;

impl KeySource for FixedKeys {
    fn read_keys(&mut self) -> Result<SessionKeys, String> {
        Ok(SessionKeys { client_send: CLIENT_KEY, server_send: SERVER_KEY })
    }
}

pub struct Client {
    pub gcm: AresGcm,
    pub server_gcm: AresGcm,
    pub sequence: u32,
    pub acked: u32,
    pub counter: u32,
}

fn packed(w: &mut BitWriter, value: u64) {
    w.append_packed_uint(value);
}

impl Client {
    pub fn header(&mut self, w: &mut BitWriter) {
        w.append_u32((self.sequence << 18) | (self.acked << 4));
        w.append_u32(0);
        w.append_bits(1, 1);
        w.append_bits(0, 11);
        self.sequence = (self.sequence + 1) & 0x3FFF;
    }

    /// an ack-only 77-bit packet.
    pub fn short(&mut self) -> Bits {
        let mut w = BitWriter::new();
        self.header(&mut w);
        w.append_bits(1, 1);
        w.finish().unwrap()
    }

    /// a reliable control bunch on channel 0 with an nmt message, padded to `total` bits.
    pub fn control(&mut self, message_type: u8, open: bool, total: usize) -> Bits {
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

    pub fn seal(&mut self, packet: &Bits) -> Vec<u8> {
        self.counter += 1;
        seal_client_packet(packet, self.counter, STATELESS, &self.gcm)
    }
}

pub fn initial_datagram() -> Vec<u8> {
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

pub fn response_datagram(challenge: &[u8]) -> Vec<u8> {
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

impl Client {
    pub fn new(sequence: u32, acked: u32, counter: u32) -> Self {
        Self { gcm: AresGcm::new(&CLIENT_KEY), server_gcm: AresGcm::new(&SERVER_KEY), sequence, acked, counter }
    }

    /// a captured decoded packet with this client's header spliced over its first 64 bits.
    pub fn captured(&mut self, hex: &str, bit_count: usize) -> Bits {
        let source = Bits::from_hex(hex, bit_count).unwrap();
        let mut w = BitWriter::new();
        w.append_u32((self.sequence << 18) | (self.acked << 4));
        w.append_u32(0);
        self.sequence = (self.sequence + 1) & 0x3FFF;
        w.append_bits_value(&slice(&source, 64, bit_count));
        w.finish().unwrap()
    }

    /// stateless handshake, hello, nmt challenge, login and join.
    pub fn connect(&mut self, server: &mut RangeServer, now: f64, source: SocketAddr, out: &mut Outbox) {
        server.handle_datagram(now, source, &initial_datagram(), out);
        let (_, challenge) = out.datagrams.pop().unwrap();
        server.handle_datagram(now, source, &response_datagram(&challenge), out);
        let hello = self.control(0, true, 232);
        let short = self.short();
        let login = self.control(5, false, 600);
        let join = self.control(9, false, 142);
        for packet in [hello, short, login, join] {
            let d = self.seal(&packet);
            server.handle_datagram(now, source, &d, out);
        }
        out.datagrams.clear();
    }

    /// one ack-only packet at `now`.
    pub fn tick(&mut self, server: &mut RangeServer, now: f64, source: SocketAddr, out: &mut Outbox) {
        let short = self.short();
        let d = self.seal(&short);
        server.handle_datagram(now, source, &d, out);
    }
}

/// the bits of a packet between two positions.
pub fn slice(packet: &Bits, start: usize, end: usize) -> Bits {
    let mut r = BitReader::new(&packet.bytes, packet.bytes.len() * 8);
    r.set_position(start);
    r.read_payload(end - start)

}
