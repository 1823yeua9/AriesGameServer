//! rep-layout property payloads (plain, before the content transform).
//! every payload is [0 = no checksum] then (packed handle, value)... then packed 0.
//! docs/port/02_bunch_primitives.md section 6.1.

use aries_bits::{BitWriter, Bits};

use crate::{RepError, RepResult};

/// the empty rep payload [0][packed 0] = 9 bits (opens a subobject's layout).
pub fn empty_rep() -> Bits {
    let mut w = BitWriter::new();
    w.append_bits(0, 1);
    w.append_packed_uint(0);
    w.finish().expect("fixed")
}

/// python build_bool_property_payload: [0][handle][1-bit value][0].
pub fn bool_property(handle: u32, value: bool) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    w.append_bits(0, 1);
    w.append_packed_uint(handle as u64);
    w.append_bool(value);
    w.append_packed_uint(0);
    Ok(w.finish()?)
}

/// python build_object_property_payload: [0][handle][guid][0].
pub fn object_property(handle: u32, net_guid: u32) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    w.append_bits(0, 1);
    w.append_packed_uint(handle as u64);
    w.append_packed_uint(net_guid as u64);
    w.append_packed_uint(0);
    Ok(w.finish()?)
}

/// python build_pawn_autonomous_role_payload / build_base_pistol_autonomous_role_payload:
/// RemoteRole (handle 4) as a `width`-bit ENetRole. production: width 3, value 2.
pub fn autonomous_role(role_bit_width: u32, role_value: u32) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    w.append_bits(0, 1);
    w.append_packed_uint(4);
    w.append_bits(role_value as u64, role_bit_width);
    w.append_packed_uint(0);
    Ok(w.finish()?)
}

// ---- every other production rep-layout payload (spec 02 section 6.1) ----

/// a python ValueError guard: Invalid(what) unless ok, so a bad stage fails loudly.
pub(crate) fn check(ok: bool, what: &str) -> RepResult<()> {
    if ok { Ok(()) } else { Err(RepError::Invalid(what.to_owned())) }
}

/// ENetRole wire width in 13.06 (FByteProperty, ceil(log2(ROLE_MAX)), rsb:1426-1430).
pub const REMOTE_ROLE_BITS: u32 = 3;
/// ROLE_AutonomousProxy, the production RemoteRole (rsb:1432-1436, hs:2668).
pub const AUTONOMOUS_PROXY: u32 = 2;
/// GameState.Phase, 8-bit EAresGamePhase (rsb:574-588).
pub const PHASE_HANDLE: u32 = 67;
/// production game_state_phase_value (launch.ps1:175).
pub const RANGE_PHASE: u32 = 4;
/// AGameState.MatchState (rsb:603) and PlayerState.UniqueId (rsb:626) share handle 21 on their own actors.
pub const MATCH_STATE_HANDLE: u32 = 21;
pub const UNIQUE_ID_HANDLE: u32 = 21;
/// GameInfo.AuthGameplayStartTimestamp (rsb:646).
pub const GAMEPLAY_START_HANDLE: u32 = 40;
/// bIsStoreEnabled, bIsInfiniteMoneyEnabled, bIsIgnoreShoppingRestrictionsEnabled (rsb:656).
pub const RANGE_SHOP_SETTING_HANDLES: [u32; 3] = [44, 48, 49];
/// PlayerState.DesiredClass first FGuid leaf (rsb:1242, hs:2547).
pub const DESIRED_CLASS_BASE_HANDLE: u32 = 201;
/// Wushu's primary-asset guid, the --desired-class-guid default (hs:4753, parsed hs:720-723).
pub const WUSHU_CLASS_GUID: [u32; 4] = [0xADD6_443A, 0x41BD_E414, 0xF6AD_E58D, 0x267F_4E95];
/// backend.offline_player_id (backend.py:36), the UniqueId subject.
pub const OFFLINE_PLAYER_ID: &str = "7c9f41a4-65e2-4d81-a8d5-f879c3e5ab61";

/// [0][handle][value as `width` raw bits][0]: one fixed-width leaf (enum, byte,
/// int32). the writer fails when value does not fit.
pub fn bits_property(handle: u32, value: u64, width: u32) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    w.append_bits(0, 1);
    w.append_packed_uint(handle as u64);
    w.append_bits(value, width);
    w.append_packed_uint(0);
    Ok(w.finish()?)
}

/// [0][handle][32 raw bits][0]: an int32/uint32 leaf (FRepLayout writes it raw).
/// inline in rsb:849-853 (int property) and rsb:1019-1023 (money).
pub fn int_property(handle: u32, value: u32) -> RepResult<Bits> {
    bits_property(handle, value as u64, 32)
}

/// [0][handle][guid 0][0]: an object leaf set to null (hs:2012-2016 hud refresh).
pub fn null_object_property(handle: u32) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    w.append_bits(0, 1);
    w.append_packed_uint(handle as u64);
    w.append_packed_uint(0);
    w.append_packed_uint(0);
    Ok(w.finish()?)
}

/// python build_desired_class_payload (rsb:1241): an FGuid flattened into four
/// u32 leaves at base_handle..base_handle+3.
pub fn desired_class(guid: [u32; 4], base_handle: u32) -> RepResult<Bits> {
    check((1..=0xFFFC).contains(&base_handle), "base_handle outside supported range")?;
    let mut w = BitWriter::new();
    w.append_bits(0, 1);
    for (offset, part) in (0u32..).zip(guid) {
        w.append_packed_uint((base_handle + offset) as u64);
        w.append_u32(part);
    }
    w.append_packed_uint(0);
    Ok(w.finish()?)
}

/// python build_object_array_property_payload (rsb:1300): TArray<UObject*> as
/// [16-bit count], (child handle i+1, guid) per element, child 0, outer 0.
pub fn object_array(handle: u32, net_guids: &[u32]) -> RepResult<Bits> {
    check((1..=0xFFFF).contains(&handle) && net_guids.len() <= 256, "array handle or element count outside supported range")?;
    check(net_guids.iter().all(|g| (1..=0xFFFF).contains(g)), "array object GUID outside supported range")?;
    let mut w = BitWriter::new();
    w.append_bits(0, 1);
    w.append_packed_uint(handle as u64);
    w.append_bits(net_guids.len() as u64, 16);
    for (child, &guid) in (1u64..).zip(net_guids) {
        w.append_packed_uint(child);
        w.append_packed_uint(guid as u64);
    }
    w.append_packed_uint(0);
    w.append_packed_uint(0);
    Ok(w.finish()?)
}

/// python build_guid_array_property_payload (rsb:1325): TArray<FGuid> with four
/// u32 leaves per element at child handles 4e+1..4e+4.
pub fn guid_array(handle: u32, guids: &[[u32; 4]]) -> RepResult<Bits> {
    check((1..=0xFFFF).contains(&handle) && guids.len() <= 256, "GUID array handle or element count outside supported range")?;
    let mut w = BitWriter::new();
    w.append_bits(0, 1);
    w.append_packed_uint(handle as u64);
    w.append_bits(guids.len() as u64, 16);
    for (element, guid) in (0u64..).zip(guids) {
        for (part_index, &part) in (1u64..).zip(guid) {
            w.append_packed_uint(4 * element + part_index);
            w.append_u32(part);
        }
    }
    w.append_packed_uint(0);
    w.append_packed_uint(0);
    Ok(w.finish()?)
}

/// python build_ordered_property_payload (rsb:1395): already-serialized leaves,
/// handles strictly increasing.
pub fn ordered(fields: &[(u32, Bits)]) -> RepResult<Bits> {
    check(!fields.is_empty(), "at least one property is required")?;
    let mut w = BitWriter::new();
    w.append_bits(0, 1); // no property checksum
    let mut previous = 0;
    for (handle, value) in fields {
        check(previous < *handle, "property handles must be positive and increasing")?;
        w.append_packed_uint(*handle as u64);
        w.append_bits_value(value);
        previous = *handle;
    }
    w.append_packed_uint(0);
    Ok(w.finish()?)
}

/// python build_phase_property_payload (rsb:574): GameState.Phase, 8 bits.
pub fn phase(phase_value: u32) -> RepResult<Bits> {
    check(phase_value <= 7, "phase_value must be a concrete EAresGamePhase")?;
    bits_property(PHASE_HANDLE, phase_value as u64, 8)
}

/// python build_match_state_property_payload (rsb:591): non-hardcoded FName
/// [0][fstring][32-bit number 0]. only InProgress was inspected.
pub fn match_state(state_name: &str) -> RepResult<Bits> {
    check(state_name == "InProgress", "only the inspected InProgress transition is supported")?;
    let mut w = BitWriter::new();
    w.append_bits(0, 1);
    w.append_packed_uint(MATCH_STATE_HANDLE as u64);
    w.append_bits(0, 1); // not hardcoded
    w.append_fstring_ascii(state_name);
    w.append_bits(0, 32); // fname number
    w.append_packed_uint(0);
    Ok(w.finish()?)
}

/// python str(uuid.UUID(s)) == s: 8-4-4-4-12 lowercase hex.
fn is_canonical_uuid(text: &str) -> bool {
    text.len() == 36
        && text.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
        })
}

/// python build_player_unique_id_property_payload (rsb:611): FUniqueNetIdRepl as
/// the client's own NMT_Login encoding [F8][fstring Riot][fstring subject].
pub fn player_unique_id(subject: &str) -> RepResult<Bits> {
    check(is_canonical_uuid(subject), "UniqueId subject must be a canonical lowercase UUID")?;
    let mut w = BitWriter::new();
    w.append_bits(0, 1);
    w.append_packed_uint(UNIQUE_ID_HANDLE as u64);
    w.append_bits(0xF8, 8);
    w.append_fstring_ascii("Riot");
    w.append_fstring_ascii(subject);
    w.append_packed_uint(0);
    Ok(w.finish()?)
}

/// python build_gameplay_start_property_payload (rsb:634): one raw f32, only 0.0
/// (python's == lets -0.0 through, which packs as 0x80000000).
pub fn gameplay_start(start_time: f64) -> RepResult<Bits> {
    check(start_time == 0.0, "only inspected zero gameplay start is supported")?;
    bits_property(GAMEPLAY_START_HANDLE, (start_time as f32).to_bits() as u64, 32)
}

/// python build_range_shop_settings_payload (rsb:659): each sorted unique handle
/// in 44..=49 set true (one bit).
pub fn range_shop_settings(handles: &[u32]) -> RepResult<Bits> {
    check(!handles.is_empty() && handles.iter().all(|h| (44..=49).contains(h)), "only the inspected GameInfo Range-setting handles 44..49 are supported")?;
    let mut sorted = handles.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut w = BitWriter::new();
    w.append_bits(0, 1);
    for handle in sorted {
        w.append_packed_uint(handle as u64);
        w.append_bits(1, 1);
    }
    w.append_packed_uint(0);
    Ok(w.finish()?)
}

/// python build_base_pistol_autonomous_role_payload (rsb:1450): RemoteRole = 2,
/// width 2 or 3 only. also the knife's role (range_melee_equippable.py:67).
pub fn base_pistol_autonomous_role(role_bit_width: u32) -> RepResult<Bits> {
    check(matches!(role_bit_width, 2 | 3), "ENetRole wire width must be 2 or 3 bits")?;
    autonomous_role(role_bit_width, AUTONOMOUS_PROXY)
}
