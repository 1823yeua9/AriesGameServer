//! every bunch of one decoded client packet (range_client_rpc.iter_bunches).
//! the packet inspector only reports the first bunch; a reliable rpc can ride
//! behind a movement bunch, and once its packet is acked the client never
//! resends it, so the rpc decoders walk them all.

use aries_bits::{BitReader, Bits};
use aries_net::frame::ARES_TRAILER;

use super::reader::BoundedReader;
use super::DecodeResult;

/// first bit after the 32-bit header word and the 32-bit ack history
/// (range_client_rpc.py:86, range_udp_handshake.py:480).
const PACKET_INFO_BIT: usize = 64;
/// width of the packet info (jitter) field after its present bit (range_client_rpc.py:88).
const PACKET_INFO_BITS: usize = 10;
/// ChSequence width on client reliable bunches (range_client_rpc.py:117).
const CHANNEL_SEQUENCE_BITS: usize = 12;
/// partial-bunch bits after the sequence, meaning unknown (range_client_rpc.py:118-119).
const PARTIAL_BITS: usize = 2;
/// BunchDataBits width (range_client_rpc.py:123).
const BUNCH_SIZE_BITS: usize = 13;
/// the walk stops unless more than this many bits are left (range_client_rpc.py:91).
const MIN_BUNCH_TAIL: usize = 16;

/// one bunch header and where its body lies in the decoded packet. python's
/// Bunch keeps start/control/reliable/partial/channel/sequence/body/end; the
/// other header fields are read on the way and kept for logs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BunchView {
    pub start_bit: usize,
    pub is_control: bool,
    pub is_open: bool,
    pub replication_paused: bool,
    pub is_reliable: bool,
    /// packed, up to 35 bits like python (never masked)
    pub channel_index: u64,
    pub has_exports: bool,
    pub has_must_map: bool,
    pub is_partial: bool,
    pub header_flag_4: bool,
    /// raw 12-bit ChSequence, only on reliable bunches
    pub channel_sequence: Option<u32>,
    pub hardcoded_name: bool,
    /// only when hardcoded_name is set (no FString form is read)
    pub channel_name_index: Option<u64>,
    pub body_start_bit: usize,
    pub end_bit: usize,
}

impl BunchView {
    /// BunchDataBits.
    pub fn bit_count(&self) -> usize {
        self.end_bit - self.body_start_bit
    }

    /// a copy of the body bits [body_start_bit, end_bit).
    pub fn body(&self, decoded: &Bits) -> Bits {
        let mut reader = BitReader::slice(&decoded.bytes, self.body_start_bit, self.end_bit);
        reader.read_payload(self.bit_count())
    }
}

/// range_client_rpc.iter_bunches (:84-100). errors only when the packet is too
/// short for the packet-info bits; a bad bunch header ends the walk and keeps
/// the bunches before it, and a control+close bunch ends it without being
/// returned. `end_bit == bit_count` is accepted.
pub fn iter_bunches(decoded: &Bits) -> DecodeResult<Vec<BunchView>> {
    let bit_count = decoded.bit_count;
    let mut reader = BoundedReader::new(&decoded.bytes, PACKET_INFO_BIT, bit_count);
    if reader.take_bool()? {
        reader.take(PACKET_INFO_BITS)?;
    }
    // python consumes this bit even without packet info (spec 06 open question 2)
    reader.take(1)?;
    let mut bunches = Vec::new();
    while reader.position + MIN_BUNCH_TAIL < bit_count {
        match read_bunch_header(&mut reader, bit_count) {
            Ok(Some(bunch)) => {
                reader.position = bunch.end_bit;
                bunches.push(bunch);
            }
            Ok(None) | Err(_) => break,
        }
    }
    Ok(bunches)
}

/// range_client_rpc._read_bunch_header (:103-128); field table in spec 06 4.1.
fn read_bunch_header(reader: &mut BoundedReader, bit_count: usize) -> DecodeResult<Option<BunchView>> {
    let mut bunch = BunchView { start_bit: reader.position, ..Default::default() };
    bunch.is_control = reader.take_bool()?;
    if bunch.is_control {
        bunch.is_open = reader.take_bool()?;
        // close: no reason bits are read, the walk stops here
        if reader.take_bool()? {
            return Ok(None);
        }
    }
    bunch.replication_paused = reader.take_bool()?;
    bunch.is_reliable = reader.take_bool()?;
    bunch.channel_index = reader.header_packed()?;
    bunch.has_exports = reader.take_bool()?;
    bunch.has_must_map = reader.take_bool()?;
    bunch.is_partial = reader.take_bool()?;
    bunch.header_flag_4 = reader.take_bool()?;
    if bunch.is_reliable {
        bunch.channel_sequence = Some(reader.take(CHANNEL_SEQUENCE_BITS)? as u32);
    }
    if bunch.is_partial {
        reader.take(PARTIAL_BITS)?;
    }
    if bunch.is_control || bunch.is_reliable {
        bunch.hardcoded_name = reader.take_bool()?;
        if bunch.hardcoded_name {
            bunch.channel_name_index = Some(reader.header_packed()?);
        }
    }
    let size = reader.take(BUNCH_SIZE_BITS)? as usize;
    bunch.body_start_bit = reader.position;
    if bunch.body_start_bit + size > bit_count {
        return Err(super::DecodeError::BunchExceedsPacket);
    }
    bunch.end_bit = bunch.body_start_bit + size;
    Ok(Some(bunch))
}

/// the decoded packet as python holds it: the client fills the high
/// 8 - P%8 bits of its last plaintext byte with the low bits of 0xCC and the
/// inverse transform leaves them (spec 06 2.4). aries_net::decode_client_packet
/// zeroes them; restore them before a read that is not bounded by P (the
/// first-bunch input ack takes its bounds from inspect_control_bunch, which
/// never checks end <= P).
pub fn with_client_trailer(decoded: &Bits) -> Bits {
    let mut out = decoded.clone();
    let tail = decoded.bit_count % 8;
    if let Some(last) = out.bytes.get_mut(decoded.bit_count / 8).filter(|_| tail != 0) {
        *last = (*last & ((1u8 << tail) - 1)) | (ARES_TRAILER << tail);
    }
    out
}
