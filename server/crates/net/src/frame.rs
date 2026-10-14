//! packet framing on the ares transport. one bunch per datagram, exactly like
//! range_server_bunch.build_server_bunch_frame; client packets via
//! range_udp_handshake.decode_client_ares_packet.

use aries_bits::{BitError, BitReader, BitWriter, Bits};

use crate::gcm::{AresGcm, CLIENT_TO_SERVER, SERVER_TO_CLIENT};
use crate::transform::{inverse_short_packet, transform_short_packet};

/// the trailer byte the client expects after the payload bits.
pub const ARES_TRAILER: u8 = 0xCC;
/// bunch body size field is 13 bits.
pub const MAX_BUNCH_BITS: usize = (1 << 13) - 1;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BunchHeader {
    pub channel_index: u32,
    pub channel_name_index: u32,
    pub channel_sequence: u32,
    pub is_open: bool,
    pub is_close: bool,
    pub is_reliable: bool,
    pub replication_paused: bool,
    pub has_exports: bool,
    /// EChannelCloseReason (0 = destroyed), SerializeInt(reason, 15) after bClose
    pub close_reason: u32,
}

impl BunchHeader {
    pub fn reliable(channel_index: u32, channel_sequence: u32, has_exports: bool) -> Self {
        Self { channel_index, channel_name_index: ACTOR_CHANNEL_NAME, channel_sequence, is_reliable: true, has_exports, ..Default::default() }
    }

    pub fn unreliable(channel_index: u32) -> Self {
        Self { channel_index, channel_name_index: ACTOR_CHANNEL_NAME, is_reliable: false, ..Default::default() }
    }
}

/// hardcoded name index the responder uses for every actor channel.
pub const ACTOR_CHANNEL_NAME: u32 = 102;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketInfo {
    pub server_sequence: u32,
    pub acked_client_sequence: u32,
    pub ack_history_mask: u32,
    pub stateless_header: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("packet sequence outside 14-bit range")]
    Sequence,
    #[error("invalid stateless header")]
    StatelessHeader,
    #[error("open and close flags conflict")]
    OpenClose,
    #[error("a close bunch must be reliable")]
    UnreliableClose,
    #[error("close reason outside EChannelCloseReason")]
    CloseReason,
    #[error("reliable sequence outside 12-bit range")]
    ChannelSequence,
    #[error("an unreliable bunch cannot open a channel")]
    UnreliableOpen,
    #[error("bunch body exceeds the 13-bit size field ({0} bits)")]
    BodyTooLarge(usize),
    #[error("transform rejected the packet")]
    Transform,
    #[error(transparent)]
    Bits(#[from] BitError),
}

/// the decoded (pre-transform) packet bits: packet header, bunch header, body, terminator.
pub fn build_packet_bits(body: &Bits, header: &BunchHeader, info: &PacketInfo) -> Result<Bits, FrameError> {
    if info.server_sequence >= 0x4000 || info.acked_client_sequence >= 0x4000 {
        return Err(FrameError::Sequence);
    }
    if info.stateless_header >= 32 {
        return Err(FrameError::StatelessHeader);
    }
    if header.is_close && header.is_open {
        return Err(FrameError::OpenClose);
    }
    if header.is_close && !header.is_reliable {
        return Err(FrameError::UnreliableClose);
    }
    if header.close_reason >= 15 {
        return Err(FrameError::CloseReason);
    }
    if header.is_reliable && header.channel_sequence >= 0x1000 {
        return Err(FrameError::ChannelSequence);
    }
    if !header.is_reliable && header.is_open {
        return Err(FrameError::UnreliableOpen);
    }
    if body.bit_count > MAX_BUNCH_BITS {
        return Err(FrameError::BodyTooLarge(body.bit_count));
    }

    let mut w = BitWriter::new();
    w.append_bits(((info.server_sequence as u64) << 18) | ((info.acked_client_sequence as u64) << 4), 32);
    w.append_bits(info.ack_history_mask as u64, 32);
    w.append_bits(1, 1); // packet info present
    w.append_bits(0, 10);
    w.append_bits(0, 1);
    if header.is_reliable {
        // every reliable bunch uses the control header and names its channel
        w.append_bits(1, 1); // bControl
        w.append_bool(header.is_open);
        w.append_bool(header.is_close);
        if header.is_close {
            w.append_serialize_int(header.close_reason as u64, 15);
        }
        w.append_bool(header.replication_paused);
        w.append_bits(1, 1); // bReliable
        w.append_packed_uint(header.channel_index as u64);
        w.append_bool(header.has_exports);
        w.append_bits(0, 1); // bHasMustBeMappedGUIDs
        w.append_bits(0, 1); // bPartial
        w.append_bits(0, 1); // header flag 4
        w.append_bits(header.channel_sequence as u64, 12);
        w.append_bits(1, 1); // hardcoded channel name
        w.append_packed_uint(header.channel_name_index as u64);
    } else {
        // unreliable data bunch on an open channel: no sequence, no name
        w.append_bits(0, 1); // bControl
        w.append_bool(header.replication_paused);
        w.append_bits(0, 1); // bReliable
        w.append_packed_uint(header.channel_index as u64);
        w.append_bool(header.has_exports);
        w.append_bits(0, 1);
        w.append_bits(0, 1);
        w.append_bits(0, 1);
    }
    w.append_bits(body.bit_count as u64, 13);
    w.append_bits_value(body);
    w.append_bits(1, 1); // packet terminator
    Ok(w.finish()?)
}

/// python wrap_gcm: trailer bits after the payload, gcm with counter 0, then
/// counter(4) | tag(4) | ciphertext | suffix byte.
pub fn wrap_gcm(plain: &Bits, gcm: &AresGcm, counter: u32) -> Vec<u8> {
    let trailing = plain.bit_count & 7;
    let mut data = plain.bytes.clone();
    let suffix = if trailing != 0 {
        let last = data.last_mut().expect("non-empty payload");
        *last &= (1u8 << trailing) - 1;
        *last |= ARES_TRAILER << trailing;
        ARES_TRAILER >> (8 - trailing)
    } else {
        ARES_TRAILER
    };
    let nonce = AresGcm::nonce(counter, SERVER_TO_CLIENT);
    let tag = gcm.seal(&nonce, &mut data);
    let mut out = Vec::with_capacity(data.len() + 9);
    out.extend_from_slice(&counter.to_le_bytes());
    out.extend_from_slice(&tag);
    out.extend_from_slice(&data);
    out.push(suffix);
    out
}

/// a full server datagram for one bunch. returns (datagram, decoded packet bit count).
pub fn build_server_frame(body: &Bits, header: &BunchHeader, info: &PacketInfo, gcm: &AresGcm) -> Result<(Vec<u8>, usize), FrameError> {
    let mut packet = build_packet_bits(body, header, info)?;
    let bit_count = packet.bit_count;
    if !transform_short_packet(&mut packet.bytes, bit_count, 0) {
        return Err(FrameError::Transform);
    }
    let encrypted = wrap_gcm(&packet, gcm, 0);
    let mut w = BitWriter::new();
    w.append_bits(0, 32);
    w.append_bits(info.stateless_header as u64, 6);
    w.append_payload(&encrypted, 64 + bit_count + 8);
    w.append_bits(1, 1); // terminal bit
    Ok((w.finish()?.bytes, bit_count))
}

/// an authenticated, decrypted, inverse-transformed client packet.
#[derive(Debug, Clone)]
pub struct ClientPacket {
    pub decoded: Bits,
    pub counter: u32,
    pub stateless_header: u32,
}

/// python decode_client_ares_packet. none when too short or the tag fails.
pub fn decode_client_packet(datagram: &[u8], gcm: &AresGcm) -> Option<ClientPacket> {
    let last = *datagram.last()?;
    if datagram.len() < 19 || last == 0 {
        return None;
    }
    let packet_bit_count = (datagram.len() - 1) * 8 + (8 - last.leading_zeros() as usize) - 1;
    let plaintext_bit_count = packet_bit_count.checked_sub(64 + 6 + 72)?;
    let ciphertext_length = plaintext_bit_count.div_ceil(8);
    if plaintext_bit_count == 0 || ciphertext_length > 2048 {
        return None;
    }
    let mut r = BitReader::new(datagram, datagram.len() * 8);
    r.set_position(64);
    let stateless_header = r.read_bits(6) as u32;
    let counter = r.read_bits(32) as u32;
    let tag = (r.read_bits(32) as u32).to_le_bytes();
    let mut ciphertext: Vec<u8> = (0..ciphertext_length).map(|_| r.read_bits(8) as u8).collect();
    if r.is_error() {
        return None;
    }
    let nonce = AresGcm::nonce(counter, CLIENT_TO_SERVER);
    if !gcm.open(&nonce, &mut ciphertext, &tag) {
        return None;
    }
    if !inverse_short_packet(&mut ciphertext, plaintext_bit_count, counter) {
        return None;
    }
    // keep the bits invariant: unused high bits of the last byte are zero
    if plaintext_bit_count % 8 != 0 {
        let last = ciphertext.last_mut().expect("non-empty");
        *last &= (1u8 << (plaintext_bit_count % 8)) - 1;
    }
    let decoded = Bits { bytes: ciphertext, bit_count: plaintext_bit_count };
    Some(ClientPacket { decoded, counter, stateless_header })
}
