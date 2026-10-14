//! rpc payloads (spec 02 section 6.3, spec 04 section 9.2) and rpc content-block bodies. owner: I1.
//! a payload is si(field, class_max + 1) pk(param bits) params; every parameter
//! carries the legacy presence bit because the connection's flags @172 are 0.

use aries_bits::{BitWriter, Bits};

use crate::actors::CONTROLLER_GUID;
use crate::block::{Target, append_rpc_block};
use crate::bodies::PAWN_GUID;
use crate::props::check;
use crate::RepResult;

/// ShootingRangeController_C FClassNetCache max index (rsb:881).
pub const CONTROLLER_CLASS_MAX: u32 = 178;
/// ClientRestart field, written as a raw byte (= si(44, 179), rsb:279).
pub const CLIENT_RESTART_FIELD: u32 = 44;
/// ClientSetHUD field resolved live in every logged run (rsb:289-291, hs:3784-3791).
pub const CLIENT_SET_HUD_FIELD: u32 = 52;
/// rsb:879 client_update_multiple_levels_streaming_status_field.
pub const LEVELS_STREAMING_STATUS_FIELD: u32 = 66;
/// rsb:880 client_flush_level_streaming_field.
pub const FLUSH_LEVEL_STREAMING_FIELD: u32 = 26;
/// rsb:921 client_update_range_settings_field.
pub const UPDATE_RANGE_SETTINGS_FIELD: u32 = 170;
/// Wushu_PC_C FClassNetCache max index (hs:4494, launch_range.py:50-51).
pub const PAWN_CLASS_MAX: u32 = 53;
/// ShooterCharacter.ClientAckInputAndMovement (hs:4494, hs:1542).
pub const ACK_INPUT_AND_MOVEMENT_FIELD: u32 = 41;
/// ClientMoveResponsePacked, the one placement correction (--move-ack-index 25, launch_range.py:50-51).
pub const MOVE_RESPONSE_FIELD: u32 = 25;
/// LatestReceivedEventID that leaves the client's input-event cursor alone (hs:4489-4494).
pub const NO_INPUT_EVENT: u32 = 0xFFFF_FFFF;
/// the Range game-mode sublevels (rsb:874-877).
pub const RANGE_GAMEPLAY_SUBLEVELS: [&str; 2] = ["/Game/Maps/Poveglia/Range_Only_Gameplay", "/Game/LiveStory/LiveStory_Permanent/Range_LiveStory_Permanent"];
/// FJumpVariables dwords in the correction container (rmc:15).
const JUMP_STATE_DWORDS: usize = 13;

/// field index (bounded SerializeInt), packed parameter bit count, parameters.
fn rpc_payload(field_index: u32, class_max_index: u32, params: &Bits) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    w.append_serialize_int(field_index as u64, class_max_index as u64 + 1);
    w.append_packed_uint(params.bit_count as u64);
    w.append_bits_value(params);
    Ok(w.finish()?)
}

/// python build_client_restart_rpc_payload (rsb:268): [44 as 8 bits][pk 9][1][pk pawn 10].
pub fn client_restart() -> RepResult<Bits> {
    let mut params = BitWriter::new();
    params.append_bits(1, 1);
    params.append_packed_uint(PAWN_GUID as u64);
    let params = params.finish()?;
    let mut w = BitWriter::new();
    w.append_bits(CLIENT_RESTART_FIELD as u64, 8);
    w.append_packed_uint(params.bit_count as u64);
    w.append_bits_value(&params);
    Ok(w.finish()?)
}

/// python build_client_set_hud_payload (rsb:285): one object parameter, the HUD class.
pub fn client_set_hud(field_index: u32, class_max_index: u32, hud_class_net_guid: u32) -> RepResult<Bits> {
    check(field_index < class_max_index && hud_class_net_guid >= 1, "invalid HUD RPC field index or class NetGUID")?;
    let mut params = BitWriter::new();
    params.append_bits(1, 1);
    params.append_packed_uint(hud_class_net_guid as u64);
    rpc_payload(field_index, class_max_index, &params.finish()?)
}

/// python build_input_ack_rpc_payload (rsb:463): ClientAckInputAndMovement with
/// LatestReceivedEventID and FTimeStampIndex.Value, 32 bits each.
pub fn input_ack(latest_event_id: u32, movement_timestamp_index: u32, field_index: u32, class_max_index: u32) -> RepResult<Bits> {
    check(field_index < class_max_index, "field index outside class cache range")?;
    let mut params = BitWriter::new();
    params.append_bits(1, 1);
    params.append_u32(latest_event_id);
    params.append_bits(1, 1);
    params.append_u32(movement_timestamp_index);
    rpc_payload(field_index, class_max_index, &params.finish()?)
}

/// python build_packed_bits_struct (rsb:399): FCharacterNetworkSerializationPackedBits
/// is a packed bit count then exactly that many raw bits.
pub fn packed_bits_struct(payload: &Bits) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    w.append_packed_uint(payload.bit_count as u64);
    w.append_bits_value(payload);
    Ok(w.finish()?)
}

/// python build_client_update_multiple_levels_streaming_status_payload (rsb:884):
/// [1][16-bit count], per level FName (not hardcoded) + number, LODIndex -1, four bools.
pub fn levels_streaming_status(package_names: &[&str], field_index: u32, class_max_index: u32, loaded: bool, visible: bool) -> RepResult<Bits> {
    check((1..=16).contains(&package_names.len()), "between 1 and 16 levels per RPC")?;
    check(package_names.iter().all(|name| name.starts_with("/Game/")), "level package must be a /Game/ path")?;
    let mut params = BitWriter::new();
    params.append_bits(1, 1);
    params.append_bits(package_names.len() as u64, 16);
    for name in package_names {
        params.append_bits(0, 1); // fname not hardcoded
        params.append_fstring_ascii(name);
        params.append_bits(0, 32); // fname number
        params.append_u32(0xFFFF_FFFF); // LODIndex -1
        params.append_bool(loaded);
        params.append_bool(visible);
        params.append_bits(0, 1); // bNewShouldBlockOnLoad
        params.append_bits(0, 1); // bNewShouldBlockOnUnload
    }
    rpc_payload(field_index, class_max_index, &params.finish()?)
}

/// an rpc with no parameters: si(field, max + 1) pk(0).
fn no_param_rpc(field_index: u32, class_max_index: u32) -> RepResult<Bits> {
    rpc_payload(field_index, class_max_index, &Bits::empty())
}

/// python build_client_flush_level_streaming_payload (rsb:951).
pub fn flush_level_streaming(field_index: u32, class_max_index: u32) -> RepResult<Bits> {
    no_param_rpc(field_index, class_max_index)
}

/// python build_client_update_range_settings_payload (rsb:927): the client answers
/// with its saved range settings.
pub fn update_range_settings(field_index: u32, class_max_index: u32) -> RepResult<Bits> {
    no_param_rpc(field_index, class_max_index)
}

/// three f64 coordinates (FVector custom version 47); python append_vector64 (rmc:18).
fn append_vector64(w: &mut BitWriter, vector: [f64; 3]) -> RepResult<()> {
    check(vector.iter().all(|c| c.is_finite()), "vector must contain three finite coordinates")?;
    for coordinate in vector {
        w.append_f64(coordinate);
    }
    Ok(())
}

/// python build_correction_container (rmc:25): the no-base, no-special-mode
/// ClientMoveResponsePacked correction branch.
pub fn correction_container(move_number: u32, location: [f64; 3], velocity: [f64; 3]) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    w.append_bits(0, 1); // bAckGoodMove false
    w.append_packed_uint(move_number as u64);
    w.append_bits(0, 4); // no base, rotation or special data
    append_vector64(&mut w, location)?; // NewLoc
    append_vector64(&mut w, velocity)?; // NewVel
    w.append_bits(0, 1); // correction/debug vector absent
    w.append_bits(0, 32);
    for _ in 0..JUMP_STATE_DWORDS {
        w.append_bits(0, 32);
    }
    w.append_bits(0, 1);
    w.append_bits(0, 2);
    w.append_bits(0, 32);
    w.append_bits(0, 1); // object reference absent
    w.append_bits(0, 1); // object reference absent
    w.append_bits(0, 1); // movement mode absent -> MOVE_Walking
    w.append_bits(0, 1);
    w.append_bits(0, 1);
    Ok(w.finish()?)
}

/// python build_correction_rpc_payload (rmc:58): the container in a packed-bits
/// struct behind one presence bit.
pub fn correction_rpc(move_number: u32, location: [f64; 3], field_index: u32, class_max_index: u32, velocity: [f64; 3]) -> RepResult<Bits> {
    check(field_index < class_max_index, "field index outside class cache range")?;
    let packed = packed_bits_struct(&correction_container(move_number, location, velocity)?)?;
    let mut params = BitWriter::new();
    params.append_bits(1, 1);
    params.append_bits_value(&packed);
    rpc_payload(field_index, class_max_index, &params.finish()?)
}

/// an rpc content block on the channel actor: [0][1][pk bits][enc(payload, actor)]
/// (python append_actor_rpc_content_block(b"", 0, enc, bits), rsb:256).
pub fn actor_rpc_body(payload: &Bits, actor_guid: u32) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    append_rpc_block(&mut w, Target::Actor, payload, actor_guid)?;
    Ok(w.finish()?)
}

/// python build_controller_rpc_body (rsb:938): seed = controller guid 4.
pub fn controller_rpc_body(payload: &Bits) -> RepResult<Bits> {
    actor_rpc_body(payload, CONTROLLER_GUID)
}

/// an rpc on the pawn channel (seed 10): move acks, input acks, the correction (hs:4496-4499, 1543-1546).
pub fn pawn_rpc_body(payload: &Bits) -> RepResult<Bits> {
    actor_rpc_body(payload, PAWN_GUID)
}
