//! controller rpcs the client sends on ch5 that we act on: ServerPurchase
//! (and Remove/Sell, decoded but only logged) and the range settings replies
//! (range_client_rpc.py). rpc params: one presence bit per non-bool parent
//! parameter; struct members follow in order with no handles; bools are 1 bit.

use std::collections::HashSet;

use aries_bits::Bits;

use super::bunch::{BunchView, iter_bunches};
use super::content::{CONTROLLER_ACTOR_MAP_VALUE, read_actor_content_block};
use super::reader::BoundedReader;
use super::{DecodeError, DecodeResult};

/// the controller's actor channel (range_client_rpc.py:162, 239).
pub const CONTROLLER_CHANNEL: u64 = 5;
/// ShootingRangeController_C FClassNetCache max 178, plus one (range_client_rpc.py:23).
pub const CONTROLLER_FIELD_MAX: u64 = 179;
/// pawn rpc field max (build_input_ack_rpc_payload(.., 41, 53), range_udp_handshake.py:1541-1542).
/// python never reads a ch8 field; kept for read_rpc_header callers.
pub const PAWN_FIELD_MAX: u64 = 53;

/// range_client_rpc.py:20-22.
pub const SERVER_PURCHASE_FIELD: u64 = 156;
pub const SERVER_REMOVE_FIELD: u64 = 157;
pub const SERVER_SELL_FIELD: u64 = 161;

/// range_client_rpc.py:198-205. 170 goes server -> client (the request).
pub const CLIENT_UPDATE_RANGE_SETTINGS_FIELD: u64 = 170;
pub const SERVER_END_MISSION_MODULE_FIELD: u64 = 172;
pub const SERVER_LOAD_AND_SWAP_CHARACTER_FIELD: u64 = 173;
pub const SERVER_RESET_BOTS_FIELD: u64 = 174;
pub const SERVER_TOGGLE_FREE_SHOOTING_FIELD: u64 = 175;
pub const SERVER_UPDATE_SHOOTING_RANGE_SETTINGS_FIELD: u64 = 176;
pub const SERVER_UPDATE_TARGET_PRACTICE_SETTINGS_FIELD: u64 = 177;
pub const RANGE_SETTINGS_FIELDS: [u64; 6] = [172, 173, 174, 175, 176, 177];

/// EAresItemSlot width (range_client_rpc.py:175).
const TARGET_SLOT_BITS: usize = 8;
/// EAresPurchasableType width (range_client_rpc.py:176).
const PURCHASABLE_TYPE_BITS: usize = 3;
/// export flags byte after a default (1) net guid (range_client_rpc.py:141).
const EXPORT_FLAGS_BITS: usize = 8;
/// network checksum after the name when export flag 4 is set (range_client_rpc.py:146-147).
const EXPORT_CHECKSUM_BITS: usize = 32;
const EXPORT_HAS_PATH: u64 = 1;
const EXPORT_HAS_CHECKSUM: u64 = 4;
/// outer chains deeper than this are rejected (range_client_rpc.py:134).
const MAX_OUTER_DEPTH: u32 = 8;

/// the rpc field header inside a decoded block payload (spec 06 5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RpcHeader {
    /// SerializeInt(field, field_max)
    pub field: u64,
    /// packed size of the params that follow
    pub payload_bits: u64,
    /// payload bit where the params start
    pub params_start: usize,
}

impl RpcHeader {
    /// python's params reader: limit = start + payload_bits, never clamped to the block.
    pub fn params<'a>(&self, payload: &'a Bits) -> BoundedReader<'a> {
        BoundedReader::new(&payload.bytes, self.params_start, self.params_start + self.payload_bits as usize)
    }
}

/// the field index and params size at the start of a block payload
/// (range_client_rpc.py:168-173 and 245-250). field_max is the class max + 1.
pub fn read_rpc_header(payload: &Bits, field_max: u64) -> DecodeResult<RpcHeader> {
    let mut reader = BoundedReader::new(&payload.bytes, 0, payload.bit_count);
    let field = reader.serialize_int(field_max)?;
    let payload_bits = reader.packed()?;
    Ok(RpcHeader { field, payload_bits, params_start: reader.position })
}

/// range_client_rpc._object_reference (:131-148). a ue5 package-map object
/// reference: the net guid, and for the default guid (1) its export flags,
/// outer reference, name and checksum. forms: None (guid 0), "<guid:N>"
/// (mapped), "<guid:1>" (no path), "Name", "Outer.Name", "<guid:N>.Name".
fn object_reference(reader: &mut BoundedReader, depth: u32) -> DecodeResult<Option<String>> {
    if depth > MAX_OUTER_DEPTH {
        return Err(DecodeError::OuterTooDeep);
    }
    let net_guid = reader.packed()?;
    if net_guid == 0 {
        return Ok(None);
    }
    // already mapped: only the guid travels
    if net_guid != 1 {
        return Ok(Some(format!("<guid:{net_guid}>")));
    }
    let flags = reader.take(EXPORT_FLAGS_BITS)?;
    if flags & EXPORT_HAS_PATH == 0 {
        return Ok(Some("<guid:1>".to_string()));
    }
    let outer = object_reference(reader, depth + 1)?;
    let name = reader.fstring()?;
    if flags & EXPORT_HAS_CHECKSUM != 0 {
        reader.take(EXPORT_CHECKSUM_BITS)?;
    }
    // python joins only when the outer is truthy, so an empty outer name drops out
    Ok(Some(match outer {
        Some(outer) if !outer.is_empty() => format!("{outer}.{name}"),
        _ => name,
    }))
}

/// _object_reference at `position` with a reader limit. returns (reference, end position).
pub fn read_object_reference(data: &[u8], position: usize, limit: usize) -> DecodeResult<(Option<String>, usize)> {
    let mut reader = BoundedReader::new(data, position, limit);
    let reference = object_reference(&mut reader, 0)?;
    Ok((reference, reader.position))
}

/// one ServerPurchase / ServerRemove / ServerSell (range_client_rpc.py:151-157).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurchaseRequest {
    /// 156 purchase, 157 remove, 161 sell
    pub field: u64,
    /// e.g. /Game/.../BattleRifle.BattleRifle_C or <guid:525>.AssaultRifle_AK_C
    pub item_path: Option<String>,
    /// 0 when absent
    pub target_slot: u8,
    /// 0 when absent
    pub purchasable_type: u8,
    pub channel_sequence: u32,
}

/// a controller rpc that passed the field filter: its payload, field, the
/// payload bit after the field, and the bunch's sequence.
struct ControllerRpc {
    payload: Bits,
    field: u64,
    field_end: usize,
    channel_sequence: u32,
}

impl ControllerRpc {
    /// the packed params size, then python's params reader over
    /// [start, start + size). the size is read only after the field filter.
    fn params(&self) -> DecodeResult<(BoundedReader<'_>, usize)> {
        let mut reader = BoundedReader::new(&self.payload.bytes, self.field_end, self.payload.bit_count);
        let payload_bits = reader.packed()? as usize;
        let end = reader.position + payload_bits;
        Ok((BoundedReader::new(&self.payload.bytes, reader.position, end), end))
    }
}

/// the gate both controller decoders share (range_client_rpc.py:162-171,
/// 239-248): a whole reliable ch5 bunch whose first block is an rpc (not rep
/// layout) with a field in `fields`.
fn controller_rpc(decoded: &Bits, bunch: &BunchView, fields: &[u64]) -> DecodeResult<Option<ControllerRpc>> {
    if bunch.channel_index != CONTROLLER_CHANNEL || !bunch.is_reliable || bunch.is_partial {
        return Ok(None);
    }
    let Some(channel_sequence) = bunch.channel_sequence else {
        return Ok(None);
    };
    let block = read_actor_content_block(decoded, bunch.body_start_bit, bunch.end_bit, CONTROLLER_ACTOR_MAP_VALUE)?;
    if block.has_rep_layout {
        return Ok(None);
    }
    let mut reader = BoundedReader::new(&block.payload.bytes, 0, block.payload.bit_count);
    let field = reader.serialize_int(CONTROLLER_FIELD_MAX)?;
    if !fields.contains(&field) {
        return Ok(None);
    }
    Ok(Some(ControllerRpc { field, field_end: reader.position, channel_sequence, payload: block.payload }))
}

/// range_client_rpc.decode_controller_rpc (:160-178). None unless the bunch
/// carries field 156, 157 or 161. the params are not checked to end at the
/// payload size.
pub fn decode_controller_rpc(decoded: &Bits, bunch: &BunchView) -> DecodeResult<Option<PurchaseRequest>> {
    let Some(rpc) = controller_rpc(decoded, bunch, &[SERVER_PURCHASE_FIELD, SERVER_REMOVE_FIELD, SERVER_SELL_FIELD])? else {
        return Ok(None);
    };
    let (mut params, _) = rpc.params()?;
    let item_path = if params.take_bool()? { object_reference(&mut params, 0)? } else { None };
    let target_slot = if params.take_bool()? { params.take(TARGET_SLOT_BITS)? as u8 } else { 0 };
    let purchasable_type = if params.take_bool()? { params.take(PURCHASABLE_TYPE_BITS)? as u8 } else { 0 };
    Ok(Some(PurchaseRequest { field: rpc.field, item_path, target_slot, purchasable_type, channel_sequence: rpc.channel_sequence }))
}

/// range_client_rpc.decode_purchase_requests (:181-194). a bad bunch is
/// skipped; a packet too short to walk gives nothing.
pub fn decode_purchase_requests(decoded: &Bits) -> Vec<PurchaseRequest> {
    iter_bunches(decoded).map(|bunches| decode_purchase_requests_in(decoded, &bunches)).unwrap_or_default()
}

/// decode_purchase_requests over bunches already walked (decode the packet once).
pub fn decode_purchase_requests_in(decoded: &Bits, bunches: &[BunchView]) -> Vec<PurchaseRequest> {
    bunches.iter().filter_map(|bunch| decode_controller_rpc(decoded, bunch).ok().flatten()).collect()
}

/// one range settings rpc (range_client_rpc.RangeSettingsRequest, :208-226).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RangeSettingsRequest {
    pub field: u64,
    pub channel_sequence: u32,
    // 176 ServerUpdateShootingRangeSettings(FShootingRangeSettings)
    pub bot_armor: bool,
    pub bot_strafe: bool,
    pub infinite_ammo: bool,
    pub skill_test: i32,
    pub plant_difficulty: i32,
    pub defuse_difficulty: i32,
    pub practice_mode: bool,
    pub bot_amount_index: i32,
    // 177 ServerUpdateTargetPracticeSettings(int32, bool, bool)
    pub distance_index: i32,
    pub bot_enabled: bool,
    pub is_init_settings_flow: bool,
    // 172/173: an object parameter (path when exported)
    pub object_path: Option<String>,
}

impl RangeSettingsRequest {
    /// every parameter at its default.
    pub fn new(field: u64, channel_sequence: u32) -> Self {
        Self { field, channel_sequence, ..Default::default() }
    }
}

/// _int32 (:229-230): 32 raw bits as two's complement.
fn take_i32(params: &mut BoundedReader) -> DecodeResult<i32> {
    Ok(params.take(32)? as u32 as i32)
}

/// range_client_rpc.decode_range_settings_rpc (:233-276). None unless the
/// bunch carries one of fields 172..177. 174/175 must carry no params; the
/// others must end exactly at the payload size.
pub fn decode_range_settings_rpc(decoded: &Bits, bunch: &BunchView) -> DecodeResult<Option<RangeSettingsRequest>> {
    let Some(rpc) = controller_rpc(decoded, bunch, &RANGE_SETTINGS_FIELDS)? else {
        return Ok(None);
    };
    let field = rpc.field;
    let (mut params, end) = rpc.params()?;
    let mut request = RangeSettingsRequest::new(field, rpc.channel_sequence);
    match field {
        SERVER_RESET_BOTS_FIELD | SERVER_TOGGLE_FREE_SHOOTING_FIELD => {
            // a non-zero params size
            if end != params.position {
                return Err(DecodeError::ParameterlessSettings);
            }
            return Ok(Some(request));
        }
        SERVER_UPDATE_SHOOTING_RANGE_SETTINGS_FIELD => {
            // absent struct: every member keeps its default
            if params.take_bool()? {
                request.bot_armor = params.take_bool()?;
                request.bot_strafe = params.take_bool()?;
                request.infinite_ammo = params.take_bool()?;
                request.skill_test = take_i32(&mut params)?;
                request.plant_difficulty = take_i32(&mut params)?;
                request.defuse_difficulty = take_i32(&mut params)?;
                request.practice_mode = params.take_bool()?;
                request.bot_amount_index = take_i32(&mut params)?;
            }
        }
        SERVER_UPDATE_TARGET_PRACTICE_SETTINGS_FIELD => {
            request.distance_index = if params.take_bool()? { take_i32(&mut params)? } else { 0 };
            request.bot_enabled = params.take_bool()?;
            request.is_init_settings_flow = params.take_bool()?;
        }
        _ => {
            // 172 / 173: one object parameter
            request.object_path = if params.take_bool()? { object_reference(&mut params, 0)? } else { None };
        }
    }
    if params.position != end {
        return Err(DecodeError::SettingsEnd(field));
    }
    Ok(Some(request))
}

/// range_client_rpc.decode_range_settings_requests (:279-292).
pub fn decode_range_settings_requests(decoded: &Bits) -> Vec<RangeSettingsRequest> {
    iter_bunches(decoded).map(|bunches| decode_range_settings_requests_in(decoded, &bunches)).unwrap_or_default()
}

/// decode_range_settings_requests over bunches already walked.
pub fn decode_range_settings_requests_in(decoded: &Bits, bunches: &[BunchView]) -> Vec<RangeSettingsRequest> {
    bunches.iter().filter_map(|bunch| decode_range_settings_rpc(decoded, bunch).ok().flatten()).collect()
}

/// the responder's purchase dedup (range_udp_handshake.py:1864-1867): a
/// request whose raw 12-bit sequence is in `seen` is dropped, every other one
/// records its sequence before any other check. `seen` is per connection,
/// shared by 156/157/161 and never cleared (so it collides after a wrap).
pub fn take_new_purchases(requests: Vec<PurchaseRequest>, seen: &mut HashSet<u32>) -> Vec<PurchaseRequest> {
    requests.into_iter().filter(|request| seen.insert(request.channel_sequence)).collect()
}

/// the responder's settings dedup (range_udp_handshake.py:1934-1937), same
/// rule as purchases with its own per-connection set shared by 172..177.
pub fn take_new_range_settings(requests: Vec<RangeSettingsRequest>, seen: &mut HashSet<u32>) -> Vec<RangeSettingsRequest> {
    requests.into_iter().filter(|request| seen.insert(request.channel_sequence)).collect()
}
