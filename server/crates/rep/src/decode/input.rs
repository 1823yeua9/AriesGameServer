//! the pawn's input-event streams inside the client's ch8 rpc payload
//! (range_client_input.py:170-281). python never reads the ch8 field index; it
//! scans the decoded content bit by bit for the 0xA3 marker and keeps every
//! complete parse. native writers: RVA 0x6D34950 (marker), 0x6D41F90 (bounds
//! and count), 0x6D41380 (kind and payload).

use aries_bits::Bits;

use super::content::{PAWN_ACTOR_MAP_VALUE, read_actor_content_block};
use super::reader::{read_bits, read_packed_uint};
use super::{DecodeError, DecodeResult};

/// stream marker byte (range_client_input.py:198).
pub const INPUT_STREAM_MARKER: u64 = 0xA3;
/// the native stream writer rejects queues above 256 entries (range_client_input.py:207-208).
pub const MAX_INPUT_STREAM_EVENTS: u64 = 256;
/// marker 8 + continuation 1 + first event id 32 + the 8-bit minimum packed count (range_client_input.py:198).
const MIN_STREAM_BITS: usize = 49;
/// a scan skips the last 48 bits: no stream can start there (range_client_input.py:262).
const SCAN_TAIL_BITS: usize = 48;
/// per-event header: continuation bit + 4-bit kind (range_client_input.py:217-220).
const EVENT_HEADER_BITS: usize = 5;
/// activation payload width (kind 1, range_client_input.py:233).
const ACTIVATION_BITS: usize = 11;

/// payload width of a fixed-size event kind (range_client_input.py:214-215).
/// kinds 4 and 8 carry a packed net guid instead; 13..15 are unknown.
pub fn input_event_payload_bits(kind: u8) -> Option<usize> {
    match kind {
        0 => Some(37),
        1 => Some(11),
        2 => Some(1),
        3 => Some(2),
        5 => Some(48),
        6 => Some(32),
        7 => Some(32),
        9 => Some(40),
        10 => Some(9),
        11 => Some(8),
        12 => Some(32),
        _ => None,
    }
}

/// one event (range_client_input.InputEvent). activation fields only on kind
/// 1, object_guid only on kinds 4 and 8.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputEvent {
    pub event_id: u32,
    pub kind: u8,
    pub slot: Option<u8>,
    pub flag: Option<u8>,
    pub state: Option<u8>,
    pub action: Option<u8>,
    /// packed, up to 35 bits like python
    pub object_guid: Option<u64>,
}

/// one complete stream (range_client_input.InputStream). bit positions are in
/// the decoded content payload; end_bit is one past the terminal bit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputStream {
    pub marker_bit: usize,
    pub end_bit: usize,
    pub first_event_id: u32,
    pub last_event_id: u32,
    pub events: Vec<InputEvent>,
}

/// range_client_input._parse_input_stream_at (:190-245). Ok(None) for any
/// incomplete or invalid candidate: bad marker or continuation bit, count
/// outside 1..=256 or past u32, last id above `max_generated_event_id`, a
/// truncated field, an unknown kind, or a missing zero terminal bit.
pub fn parse_input_stream_at(payload: &Bits, marker_bit: usize, max_generated_event_id: Option<u32>) -> DecodeResult<Option<InputStream>> {
    let data = &payload.bytes;
    let bit_count = payload.bit_count;
    if marker_bit + MIN_STREAM_BITS > bit_count || read_bits(data, marker_bit, 8)? != INPUT_STREAM_MARKER {
        return Ok(None);
    }
    if read_bits(data, marker_bit + 8, 1)? == 0 {
        return Ok(None);
    }
    let first_event_id = read_bits(data, marker_bit + 9, 32)?;
    let Ok((count, mut position)) = read_packed_uint(data, marker_bit + 41, bit_count) else {
        return Ok(None);
    };
    if !(1..=MAX_INPUT_STREAM_EVENTS).contains(&count) || first_event_id + count - 1 > u32::MAX as u64 {
        return Ok(None);
    }
    let last_event_id = (first_event_id + count - 1) as u32;
    if max_generated_event_id.is_some_and(|max| last_event_id > max) {
        return Ok(None);
    }
    let mut events = Vec::with_capacity(count as usize);
    for index in 0..count {
        if position + EVENT_HEADER_BITS > bit_count || read_bits(data, position, 1)? == 0 {
            return Ok(None);
        }
        let kind = read_bits(data, position + 1, 4)? as u8;
        position += EVENT_HEADER_BITS;
        let event_id = (first_event_id + index) as u32;
        if kind == 4 || kind == 8 {
            let Ok((object_guid, next)) = read_packed_uint(data, position, bit_count) else {
                return Ok(None);
            };
            position = next;
            events.push(InputEvent { event_id, kind, object_guid: Some(object_guid), ..Default::default() });
            continue;
        }
        let Some(payload_bits) = input_event_payload_bits(kind).filter(|bits| position + bits <= bit_count) else {
            return Ok(None);
        };
        if kind == 1 {
            let encoded = read_bits(data, position, ACTIVATION_BITS)?;
            events.push(InputEvent {
                event_id,
                kind,
                slot: Some(((encoded >> 1) & 0xF) as u8),
                flag: Some((encoded & 1) as u8),
                state: Some(((encoded >> 8) & 0x7) as u8),
                action: Some(((encoded >> 5) & 0x7) as u8),
                object_guid: None,
            });
        } else {
            events.push(InputEvent { event_id, kind, ..Default::default() });
        }
        position += payload_bits;
    }
    if position >= bit_count || read_bits(data, position, 1)? != 0 {
        return Ok(None);
    }
    Ok(Some(InputStream { marker_bit, end_bit: position + 1, first_event_id: first_event_id as u32, last_event_id, events }))
}

/// range_client_input.scan_received_input_streams (:248-269). tries every
/// bit-aligned 0xA3 and returns every complete parse in order. it does not
/// skip past a stream it found, so markers inside a real stream's events can
/// come back as extra (spurious) streams, exactly like python.
pub fn scan_received_input_streams(payload: &Bits, max_generated_event_id: Option<u32>) -> DecodeResult<Vec<InputStream>> {
    if payload.bit_count > payload.bytes.len() * 8 {
        return Err(DecodeError::ContentBitCount);
    }
    let mut streams = Vec::new();
    for marker_bit in 0..payload.bit_count.saturating_sub(SCAN_TAIL_BITS) {
        if read_bits(&payload.bytes, marker_bit, 8)? != INPUT_STREAM_MARKER {
            continue;
        }
        if let Some(stream) = parse_input_stream_at(payload, marker_bit, max_generated_event_id)? {
            streams.push(stream);
        }
    }
    Ok(streams)
}

/// range_client_input.decode_received_actor_inputs (:272-281): the bunch's
/// first block with the pawn's actor map value (10); a rep-layout block has
/// no streams. bounds are packet bits; the responder passes the first
/// bunch's body bounds from inspect_control_bunch (see with_client_trailer).
pub fn decode_received_actor_inputs(packet: &Bits, body_start_bit: usize, body_end_bit: usize, max_generated_event_id: Option<u32>) -> DecodeResult<Vec<InputStream>> {
    let block = read_actor_content_block(packet, body_start_bit, body_end_bit, PAWN_ACTOR_MAP_VALUE)?;
    if block.has_rep_layout {
        return Ok(Vec::new());
    }
    scan_received_input_streams(&block.payload, max_generated_event_id)
}
