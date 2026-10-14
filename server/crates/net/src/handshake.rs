//! ue stateless connect handshake (connectionless packets before the ares
//! session). production uses the raw framing: 4 zero envelope bytes, no ares
//! trailer. mirrors range_udp_handshake parse_handshake/build_challenge/build_ack.

use aries_bits::{BitReader, BitWriter, Bits};

pub const HANDSHAKE_VERSION: u8 = 4;
pub const PACKET_INITIAL: u8 = 0;
pub const PACKET_CHALLENGE: u8 = 1;
pub const PACKET_RESPONSE: u8 = 2;
pub const PACKET_ACK: u8 = 3;
/// challenge and ack plaintext length.
pub const CHALLENGE_BITS: usize = 377;
/// production raw envelope before the challenge/ack.
pub const RAW_ENVELOPE_BYTES: usize = 4;

#[derive(Debug, Clone, PartialEq)]
pub struct HandshakePacket {
    pub travel_count: u8,
    pub client_id: u8,
    pub minimum_version: u8,
    pub version: u8,
    pub packet_type: u8,
    pub packet_count: u8,
    pub network_version: u32,
    pub runtime_features: u16,
    pub secret_id: u8,
    pub timestamp: f64,
    pub cookie: [u8; 20],
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HandshakeError {
    #[error("missing expected eight-byte ares initial envelope")]
    Envelope,
    #[error("truncated handshake")]
    Truncated,
    #[error("not an ordinary stateless handshake")]
    NotStateless,
    #[error("unexpected handshake version")]
    Version,
    #[error("unexpected handshake type")]
    Type,
    #[error("invalid initial handshake payload")]
    InitialPayload,
}

/// python parse_handshake: 8 zero bytes, then the handshake bits.
pub fn parse_handshake(datagram: &[u8]) -> Result<HandshakePacket, HandshakeError> {
    if datagram.len() < 57 || datagram[..8] != [0u8; 8] {
        return Err(HandshakeError::Envelope);
    }
    let mut r = BitReader::new(datagram, datagram.len() * 8);
    r.set_position(64);
    let travel_count = r.read_bits(2) as u8;
    let client_id = r.read_bits(3) as u8;
    let handshake_bit = r.read_bits(1);
    let restart_bit = r.read_bits(1);
    let minimum_version = r.read_bits(8) as u8;
    let version = r.read_bits(8) as u8;
    let packet_type = r.read_bits(8) as u8;
    let packet_count = r.read_bits(8) as u8;
    let network_version = r.read_bits(32) as u32;
    let runtime_features = r.read_bits(16) as u16;
    let secret_id = r.read_bits(1) as u8;
    let timestamp = r.read_f64();
    let mut cookie = [0u8; 20];
    for byte in cookie.iter_mut() {
        *byte = r.read_bits(8) as u8;
    }
    if r.is_error() {
        return Err(HandshakeError::Truncated);
    }
    if handshake_bit != 1 || restart_bit != 0 {
        return Err(HandshakeError::NotStateless);
    }
    if version != HANDSHAKE_VERSION || minimum_version > version {
        return Err(HandshakeError::Version);
    }
    if packet_type != PACKET_INITIAL && packet_type != PACKET_RESPONSE {
        return Err(HandshakeError::Type);
    }
    if packet_type == PACKET_INITIAL && (timestamp != 0.0 || cookie != [0u8; 20]) {
        return Err(HandshakeError::InitialPayload);
    }
    Ok(HandshakePacket {
        travel_count,
        client_id,
        minimum_version,
        version,
        packet_type,
        packet_count,
        network_version,
        runtime_features,
        secret_id,
        timestamp,
        cookie,
    })
}

fn build_reply(packet: &HandshakePacket, reply_type: u8, secret_id: u8, timestamp: f64, cookie: &[u8; 20], padding: &[u8; 8]) -> Bits {
    let mut w = BitWriter::new();
    w.append_bits(packet.travel_count as u64, 2);
    w.append_bits(packet.client_id as u64, 3);
    w.append_bits(1, 1);
    w.append_bits(0, 1);
    w.append_bits(packet.minimum_version as u64, 8);
    w.append_bits(packet.version as u64, 8);
    w.append_bits(reply_type as u64, 8);
    w.append_bits(packet.packet_count as u64, 8);
    w.append_bits(packet.network_version as u64, 32);
    w.append_bits(packet.runtime_features as u64, 16);
    w.append_bits(secret_id as u64, 1);
    w.append_f64(timestamp);
    for &byte in cookie {
        w.append_bits(byte as u64, 8);
    }
    // the native serializer writes 8 random bytes then a final bit, no zero padding
    for &byte in padding {
        w.append_bits(byte as u64, 8);
    }
    w.append_bits(1, 1);
    w.finish().expect("fixed-width handshake fields")
}

/// the challenge to an initial packet. timestamp > 0 and the cookie are what
/// the response must echo back.
pub fn build_challenge(initial: &HandshakePacket, timestamp: f64, cookie: &[u8; 20], padding: &[u8; 8]) -> Option<Bits> {
    if initial.packet_type != PACKET_INITIAL || timestamp <= 0.0 {
        return None;
    }
    Some(build_reply(initial, PACKET_CHALLENGE, 0, timestamp, cookie, padding))
}

/// the type-3 acknowledgement of a matching response (secret bit 1, timestamp -1).
pub fn build_ack(response: &HandshakePacket, padding: &[u8; 8]) -> Option<Bits> {
    if response.packet_type != PACKET_RESPONSE {
        return None;
    }
    Some(build_reply(response, PACKET_ACK, 1, -1.0, &response.cookie, padding))
}

/// python wrap_raw_challenge(payload, bits, envelope, include_ares_trailer=False):
/// zero envelope bytes then the payload bytes.
pub fn wrap_raw(payload: &Bits, envelope_bytes: usize) -> Vec<u8> {
    let mut out = vec![0u8; envelope_bytes];
    out.extend_from_slice(&payload.bytes);
    out
}
