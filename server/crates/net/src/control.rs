//! packet-level replies in the session: the ack-only packet, nmt control
//! replies (challenge, welcome) built from the client's own hello header, and
//! the client packet/bunch header reader. mirrors range_udp_handshake.

use aries_bits::{BitReader, BitWriter, Bits};

/// packet header word: (seq << 18) | (acked << 4); low 4 bits = history words - 1.
pub fn packet_header_word(sequence: u32, acked: u32) -> u32 {
    (sequence << 18) | (acked << 4)
}

/// sequence fields of a decoded client packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientPacketHeader {
    /// the client's own packet sequence
    pub sequence: u32,
    /// the newest server sequence the client acked
    pub acked: u32,
    /// history bitmap for server sequences before `acked`
    pub history: u32,
}

pub fn read_packet_header(decoded: &Bits) -> Option<ClientPacketHeader> {
    if decoded.bit_count < 64 {
        return None;
    }
    let mut r = BitReader::from_bits(decoded);
    let word = r.read_u32();
    let history = r.read_u32();
    Some(ClientPacketHeader { sequence: (word >> 18) & 0x3FFF, acked: (word >> 4) & 0x3FFF, history })
}

/// the ack-only 77-bit packet: header, history mask, packet info bit, 10 zero
/// bits, a zero bit, terminator (python build_server_session_ack_from_sequence).
pub fn session_ack_packet(server_sequence: u32, client_sequence: u32, history_mask: u32) -> Bits {
    let mut w = BitWriter::new();
    w.append_u32(packet_header_word(server_sequence, client_sequence));
    w.append_u32(history_mask);
    w.append_bits(1, 1);
    w.append_bits(0, 11);
    w.append_bits(1, 1);
    w.finish().expect("fixed widths")
}

/// bunch header fields of the first bunch in a client packet (python inspect_control_bunch).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BunchInfo {
    pub bunch_start_bit: usize,
    pub header_end_bit: usize,
    pub is_control: bool,
    pub is_open: bool,
    pub is_close: bool,
    pub replication_paused: bool,
    pub reliable: bool,
    pub channel_index: u32,
    pub has_exports: bool,
    pub has_must_map: bool,
    pub partial: bool,
    pub header_flag_4: bool,
    pub channel_sequence: Option<u32>,
    pub channel_sequence_start_bit: Option<usize>,
    pub hardcoded_name: bool,
    pub channel_name_index: Option<u32>,
    pub bunch_bit_count: usize,
    pub body_start_bit: usize,
    pub end_bit: usize,
    /// first body byte (the nmt message type on channel 0)
    pub control_message_type: Option<u8>,
}

/// python inspect_control_bunch: the first bunch after the 76-bit packet header.
/// none when the packet is too short; a close bunch returns early with only the close flag.
pub fn inspect_first_bunch(decoded: &Bits) -> Option<BunchInfo> {
    if decoded.bit_count < 128 {
        return None;
    }
    let mut r = BitReader::from_bits(decoded);
    r.set_position(64);
    if r.read_bool() {
        r.read_bits(10);
    }
    r.read_bits(1);
    let mut info = BunchInfo { bunch_start_bit: r.position(), ..Default::default() };
    info.is_control = r.read_bool();
    info.is_open = info.is_control && r.read_bool();
    info.is_close = info.is_control && r.read_bool();
    if info.is_close {
        return (!r.is_error()).then_some(info);
    }
    info.replication_paused = r.read_bool();
    info.reliable = r.read_bool();
    info.channel_index = read_packed_5(&mut r)?;
    info.has_exports = r.read_bool();
    info.has_must_map = r.read_bool();
    info.partial = r.read_bool();
    info.header_flag_4 = r.read_bool();
    if info.reliable {
        info.channel_sequence_start_bit = Some(r.position());
        info.channel_sequence = Some(r.read_bits(12) as u32);
    }
    if info.partial {
        r.read_bits(2);
    }
    info.hardcoded_name = (info.is_control || info.reliable) && r.read_bool();
    if info.hardcoded_name {
        info.channel_name_index = Some(read_packed_5(&mut r)?);
    }
    info.bunch_bit_count = r.read_bits(13) as usize;
    info.body_start_bit = r.position();
    info.header_end_bit = info.body_start_bit;
    info.end_bit = info.body_start_bit + info.bunch_bit_count;
    if info.bunch_bit_count >= 8 {
        info.control_message_type = Some(r.read_bits(8) as u8);
    }
    (!r.is_error()).then_some(info)
}

/// packed uint limited to five bytes like the python take_packed.
fn read_packed_5(r: &mut BitReader) -> Option<u32> {
    let mut value = 0u32;
    for shift in (0..35).step_by(7) {
        let byte = r.read_bits(8) as u32;
        if r.is_error() {
            return None;
        }
        value |= (byte >> 1) << shift;
        if byte & 1 == 0 {
            return Some(value);
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ControlError {
    #[error("control reply requires the 232-bit hello and an authenticated client packet")]
    BadInput,
    #[error("unsupported control reply schema")]
    Schema,
    #[error("hello control bunch did not match the proven framing")]
    HelloShape,
    #[error("control string outside local bounds")]
    BadString,
}

/// python build_server_control_probe. message 3 (challenge, one string) or
/// 1 (welcome, three strings). the hello's bunch header is copied with bOpen
/// cleared and its channel sequence replaced by server_sequence + 1.
/// returns (packet bits, client sequence, server sequence used).
pub fn control_reply(hello: &Bits, client: &Bits, message_type: u8, strings: &[&str], server_sequence_override: Option<u32>) -> Result<(Bits, u32, u32), ControlError> {
    if hello.bit_count != 232 || client.bit_count < 77 {
        return Err(ControlError::BadInput);
    }
    if !matches!((message_type, strings.len()), (3, 1) | (1, 3)) {
        return Err(ControlError::Schema);
    }
    let shape = inspect_first_bunch(hello).ok_or(ControlError::HelloShape)?;
    if shape.control_message_type != Some(0) || shape.channel_index != 0 || !shape.is_control || !shape.reliable || shape.end_bit != hello.bit_count - 1 {
        return Err(ControlError::HelloShape);
    }
    let client_header = read_packet_header(client).ok_or(ControlError::BadInput)?;
    let mut server_sequence = (client_header.acked + 1) & 0x3FFF;
    if let Some(sequence) = server_sequence_override {
        if sequence >= 0x4000 {
            return Err(ControlError::BadInput);
        }
        server_sequence = sequence;
    }
    let history_mask: u32 = if message_type == 3 { 3 } else { 7 };

    let header_width = shape.body_start_bit - 76 - 13;
    let mut r = BitReader::from_bits(hello);
    r.set_position(76);
    // copy the hello's bunch header bit by bit, then patch bOpen and the sequence
    let mut copied: Vec<bool> = (0..header_width).map(|_| r.read_bool()).collect();
    copied[1] = false; // the client's control channel is already open
    let sequence_start = shape.channel_sequence_start_bit.ok_or(ControlError::HelloShape)? - 76;
    let channel_sequence = (server_sequence + 1) & 0xFFF;
    for bit in 0..12 {
        copied[sequence_start + bit] = (channel_sequence >> bit) & 1 != 0;
    }

    let mut body = BitWriter::new();
    body.append_bits(message_type as u64, 8);
    for value in strings {
        if value.len() > 200 || !value.is_ascii() || value.contains('\0') {
            return Err(ControlError::BadString);
        }
        if value.is_empty() {
            body.append_i32(0);
        } else {
            body.append_i32(value.len() as i32 + 1);
            for byte in value.bytes() {
                body.append_bits(byte as u64, 8);
            }
            body.append_bits(0, 8);
        }
    }
    let body = body.finish().map_err(|_| ControlError::BadString)?;

    let mut w = BitWriter::new();
    w.append_u32(packet_header_word(server_sequence, client_header.sequence));
    w.append_u32(history_mask);
    w.append_bits(1, 1);
    w.append_bits(0, 11);
    for bit in copied {
        w.append_bool(bit);
    }
    w.append_bits(body.bit_count as u64, 13);
    w.append_bits_value(&body);
    w.append_bits(1, 1);
    Ok((w.finish().map_err(|_| ControlError::BadString)?, client_header.sequence, server_sequence))
}

/// python update_received_packet_history: bit d-1 for each remembered client
/// sequence at distance d in 1..=32 behind `current`.
pub fn received_history_mask(recent: impl IntoIterator<Item = u32>, current: u32) -> u32 {
    let mut mask = 0u32;
    for previous in recent {
        let distance = current.wrapping_sub(previous) & 0x3FFF;
        if (1..=32).contains(&distance) {
            mask |= 1 << (distance - 1);
        }
    }
    mask
}
