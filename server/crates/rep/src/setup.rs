//! inline bring-up bodies built in run_probe (spec 03 section 7 + 9.4, spec 04 section 8 + 9). owner: I1.
//! one function per stage, returning the exact bunch body python passes to
//! build_server_bunch_frame. channel, reliable sequence and has_exports belong
//! to the stage table, not here; "exports" in a doc means the bunch needs has_exports.
//! hs = range_udp_handshake.py, rsb = range_server_bunch.py.

use aries_bits::{BitWriter, Bits};

use crate::actors::{
    self, ARES_HUD_CLASS, BASE_PISTOL_GUID, CONTROLLER_GUID, GAME_STATE_GUID, PAWN_SPAWN_LOCATION, PLAYER_INFO_GUID, PLAYER_STATE_GUID,
    STARTING_EQUIPPABLES, TEAM_STATE_GUIDS, class_netguid_export, starting_equippable_guids,
};
use crate::block::{Target, append_export_header, append_rpc_block, append_static_export_entry};
use crate::bodies::{
    self, ACTOR_OWNER_HANDLE, AUTH_CURRENT_STATE_HANDLE, BASE_PISTOL_INACTIVE_STATE_GUID, BASE_PISTOL_STATE_MACHINE_GUID, ClassProperty,
    FIRST_ITEM_SLOT_GUID, INVENTORY_GUID, ITEM_SLOT_CLASS_GUID, PAWN_GUID, actor_rep_body, exported_subobject_rep_body, subobject_rep_body,
};
use crate::content::encode_block_payload;
use crate::props::{self, check, empty_rep, object_property};
use crate::rpc::{self, ACK_INPUT_AND_MOVEMENT_FIELD, MOVE_RESPONSE_FIELD, NO_INPUT_EVENT, PAWN_CLASS_MAX};
use crate::RepResult;

/// GameState handle 18, unnamed in code (hs:2387).
pub const GAME_STATE_BOOL_HANDLE: u32 = 18;
/// --game-state-bool-probe-value 1 (launch_range.py:31).
pub const GAME_STATE_BOOL_VALUE: bool = true;
/// Controller.PlayerState / Controller.Pawn (hs:2415, 2441).
pub const CONTROLLER_PLAYER_STATE_HANDLE: u32 = 15;
pub const CONTROLLER_PAWN_HANDLE: u32 = 16;
/// PlayerState.PlayerInfo, SpawnedCharacter, PossessedCharacter (hs:2468-2470, 2496-2498).
pub const PLAYER_STATE_PLAYER_INFO_HANDLE: u32 = 30;
pub const SPAWNED_CHARACTER_HANDLE: u32 = 194;
pub const POSSESSED_CHARACTER_HANDLE: u32 = 195;
/// PlayerInfo.AresController (hs:2575-2577).
pub const PLAYER_INFO_CONTROLLER_HANDLE: u32 = 15;
/// Pawn.PlayerState / Pawn.Controller (hs:2614, 2640).
pub const PAWN_PLAYER_STATE_HANDLE: u32 = 15;
pub const PAWN_CONTROLLER_HANDLE: u32 = 16;
/// the pawn's InventoryComponent stable name (hs:2714).
pub const INVENTORY_NAME: &str = "InventoryComponent";
/// /Script/ShooterGame package of the ItemSlot class export (hs:2768).
pub const ITEM_SLOT_PACKAGE_GUID: u32 = 63;
/// UAresInventory.ItemSlots[0] wire handle; slot i is 3 + i (hs:2848, 2891).
pub const ITEM_SLOTS_FIRST_HANDLE: u32 = 3;
/// rsb:709-712 item_slot_net_guid bound.
pub const ITEM_SLOT_COUNT: u32 = 16;
/// BasePistol EquippableSlot Secondary (hs:3053-3055).
pub const BASE_PISTOL_SLOT: u32 = 1;
/// GameState.TeamStates (hs:3450), PlayerArray (hs:3947), PlayerState team (hs:3512).
pub const TEAM_STATES_HANDLE: u32 = 34;
pub const PLAYER_ARRAY_HANDLE: u32 = 17;
pub const PLAYER_STATE_TEAM_HANDLE: u32 = 27;
/// BlueTeam/RedTeam components of the GameState (rsb:1362-1365, hs:3481-3484).
pub const TEAM_COMPONENT_GUIDS: [u32; 2] = [104, 106];
pub const TEAM_COMPONENT_NAMES: [&str; 2] = ["BlueTeam", "RedTeam"];
/// BombTeam stable subobject of PlayerState 6 and its TeamNumber (hs:3548, 3603).
pub const BOMB_TEAM_GUID: u32 = 108;
pub const TEAM_NUMBER_HANDLE: u32 = 3;
/// --bomb-team-number-value / --bomb-team-number-bits defaults (hs:4670, 4672).
/// the value is a python int (any sign), so it stays signed here.
pub const BOMB_TEAM_NUMBER_VALUE: i64 = 1;
pub const BOMB_TEAM_NUMBER_BITS: u32 = 8;
/// PlayerState.AccessibleCharacterList (hs:3638, 3647).
pub const ACCESSIBLE_CHARACTER_LIST_HANDLE: u32 = 31;
/// the knife: starting equippable 1 on ch13, its SM and InactiveState (range_melee_equippable.py:17-23, 52).
pub const KNIFE_GUID: u32 = 204;
pub const KNIFE_STATE_MACHINE_GUID: u32 = 206;
pub const KNIFE_INACTIVE_STATE_GUID: u32 = 256;
/// ViewTargetComponent on the controller: guid 50, stable name, TargetActor handle 3 (hs:4281-4307).
pub const VIEWTARGET_GUID: u32 = 50;
pub const VIEWTARGET_NAME: &str = "ViewTarget";
pub const VIEWTARGET_HANDLE: u32 = 3;
/// --move-correction-location (launch.ps1:176), the pawn spawn literal.
pub const MOVE_CORRECTION_LOCATION: [f64; 3] = PAWN_SPAWN_LOCATION;

/// a plain object leaf in a channel-actor rep block.
fn actor_object_link(actor_guid: u32, handle: u32, net_guid: u32) -> RepResult<Bits> {
    actor_rep_body(&object_property(handle, net_guid)?, actor_guid)
}

// ---- part A (spec 03 section 7) ----

/// A01 ch3 open, exports: the exported static WorldSettings path (hs:2196).
pub fn world_settings_open() -> RepResult<Bits> {
    actors::world_settings_reference()
}

/// A02 ch4 open, exports (hs:2223).
pub fn game_state_open() -> RepResult<Bits> {
    actors::game_state_reference()
}

/// A03 ch5 open, exports (hs:2291).
pub fn controller_open() -> RepResult<Bits> {
    actors::controller_reference()
}

/// A04 ch6 open, exports (hs:2313).
pub fn player_state_open() -> RepResult<Bits> {
    actors::player_state_reference()
}

/// A05 ch7 open, exports (hs:2335).
pub fn player_info_open() -> RepResult<Bits> {
    actors::player_info_reference()
}

/// A06 ch8 open, exports: Wushu at the spawn literal (hs:2357-2362).
pub fn pawn_open() -> RepResult<Bits> {
    actors::wushu_pawn_reference(Some(PAWN_SPAWN_LOCATION))
}

/// A07 ch4: GameState handle 18 = value, seed 2 (hs:2386-2391). production GAME_STATE_BOOL_VALUE.
pub fn game_state_bool_h18_body(value: bool) -> RepResult<Bits> {
    actor_rep_body(&props::bool_property(GAME_STATE_BOOL_HANDLE, value)?, GAME_STATE_GUID)
}

/// A08 ch5: Controller.PlayerState = 6, seed 4 (hs:2415-2419).
pub fn controller_player_state_link() -> RepResult<Bits> {
    actor_object_link(CONTROLLER_GUID, CONTROLLER_PLAYER_STATE_HANDLE, PLAYER_STATE_GUID)
}

/// A09 ch5: Controller.Pawn = 10, seed 4 (hs:2441).
pub fn controller_pawn_link() -> RepResult<Bits> {
    actor_object_link(CONTROLLER_GUID, CONTROLLER_PAWN_HANDLE, PAWN_GUID)
}

/// A10 ch6: PlayerState.PlayerInfo = 8, seed 6 (hs:2471).
pub fn player_state_player_info_link() -> RepResult<Bits> {
    actor_object_link(PLAYER_STATE_GUID, PLAYER_STATE_PLAYER_INFO_HANDLE, PLAYER_INFO_GUID)
}

/// A11 first frame ch6: SpawnedCharacter = 10, seed 6 (hs:2508). also the hud refresh's second body (hs:2018).
pub fn player_state_spawned_character_link() -> RepResult<Bits> {
    actor_object_link(PLAYER_STATE_GUID, SPAWNED_CHARACTER_HANDLE, PAWN_GUID)
}

/// A11 second frame ch6: PossessedCharacter = 10, seed 6 (hs:2508).
pub fn player_state_possessed_character_link() -> RepResult<Bits> {
    actor_object_link(PLAYER_STATE_GUID, POSSESSED_CHARACTER_HANDLE, PAWN_GUID)
}

/// A12 ch6: DesiredClass = guid (handles 201..204), seed 6 (hs:2545-2551).
/// production props::WUSHU_CLASS_GUID.
pub fn player_state_desired_class_body(guid: [u32; 4]) -> RepResult<Bits> {
    actor_rep_body(&props::desired_class(guid, props::DESIRED_CLASS_BASE_HANDLE)?, PLAYER_STATE_GUID)
}

/// A13 ch7: PlayerInfo.AresController = 4, seed 8 (hs:2586-2590).
pub fn player_info_controller_link() -> RepResult<Bits> {
    actor_object_link(PLAYER_INFO_GUID, PLAYER_INFO_CONTROLLER_HANDLE, CONTROLLER_GUID)
}

/// A14 ch8: Pawn.PlayerState = 6, seed 10 (hs:2614).
pub fn pawn_player_state_link() -> RepResult<Bits> {
    actor_object_link(PAWN_GUID, PAWN_PLAYER_STATE_HANDLE, PLAYER_STATE_GUID)
}

/// A15 ch8: Pawn.Controller = 4, seed 10 (hs:2640).
pub fn pawn_controller_link() -> RepResult<Bits> {
    actor_object_link(PAWN_GUID, PAWN_CONTROLLER_HANDLE, CONTROLLER_GUID)
}

/// A16 ch8: Pawn.RemoteRole = 2 in 3 bits, seed 10 (hs:2667-2672; 3 is the dead
/// pawn_authority_role trial).
pub fn pawn_remote_role_body() -> RepResult<Bits> {
    actor_rep_body(&props::pawn_autonomous_role(props::REMOTE_ROLE_BITS, props::AUTONOMOUS_PROXY)?, PAWN_GUID)
}

/// A17 ch8, exports: InventoryComponent (60, outer pawn) with the empty rep, seed 10 (hs:2706-2729).
pub fn inventory_open_body() -> RepResult<Bits> {
    exported_subobject_rep_body(INVENTORY_GUID, PAWN_GUID, INVENTORY_NAME, INVENTORY_GUID, &empty_rep(), PAWN_GUID)
}

/// dynamic-subobject block on UItemSlot: [1][0][pk slot][0 bStablyNamed][0 writer bit]
/// [pk class 61][0 bActorIsOuter][pk inventory 60][pk bits][enc(plain, pawn)] (hs:2790-2806).
fn append_item_slot_block(w: &mut BitWriter, slot_guid: u32, plain: &Bits) -> RepResult<()> {
    let encoded = encode_block_payload(plain, PAWN_GUID)?;
    w.append_bits(1, 1); // bHasRepLayout
    w.append_bits(0, 1); // subobject
    w.append_packed_uint(slot_guid as u64);
    w.append_bits(0, 1); // bStablyNamed = 0, dynamic
    w.append_bits(0, 1); // extra writer bit before the class (hs:2786-2789)
    w.append_packed_uint(ITEM_SLOT_CLASS_GUID as u64);
    w.append_bits(0, 1); // bActorIsOuter = 0
    w.append_packed_uint(INVENTORY_GUID as u64);
    w.append_packed_uint(encoded.bit_count as u64);
    w.append_bits_value(&encoded);
    Ok(())
}

/// A18 (slot 0) and A20 create (slots 1..15) ch8, exports: the ItemSlot class
/// export (61, package 63, re-sent every time) and UItemSlot_<slot> created
/// with the empty rep (hs:2767-2806, 2895-2922).
pub fn item_slot_create_body(slot: u32) -> RepResult<Bits> {
    check(slot < ITEM_SLOT_COUNT, "item slot out of range")?;
    let mut w = BitWriter::new();
    append_export_header(&mut w, 1);
    append_static_export_entry(&mut w, "ItemSlot", "/Script/ShooterGame", ITEM_SLOT_CLASS_GUID, ITEM_SLOT_PACKAGE_GUID);
    append_item_slot_block(&mut w, FIRST_ITEM_SLOT_GUID + 2 * slot, &empty_rep())?;
    Ok(w.finish()?)
}

/// A19 (slot 0) and A20 link (slots 1..15) ch8: Inventory.ItemSlots[slot]
/// (handle 3 + slot) = UItemSlot_<slot>, seed 10 (hs:2840-2851, 2939-2947).
pub fn item_slot_link_body(slot: u32) -> RepResult<Bits> {
    check(slot < ITEM_SLOT_COUNT, "item slot out of range")?;
    let plain = object_property(ITEM_SLOTS_FIRST_HANDLE + slot, FIRST_ITEM_SLOT_GUID + 2 * slot)?;
    subobject_rep_body(INVENTORY_GUID, &plain, PAWN_GUID)
}

/// A21 ch9 open, exports: the plain BasePistol open, used when no packaged
/// cosmetic is selected (hs:3001-3007). the cosmetic open is range_weapon_cosmetics'.
pub fn base_pistol_open_body() -> RepResult<Bits> {
    actors::base_pistol_reference()
}

/// A22 ch8: UItemSlot_1.Contents = 94 (hs:3062-3076, = build_item_slot_contents_block(1, 94)).
pub fn pistol_slot_contents_body() -> RepResult<Bits> {
    bodies::item_slot_contents_block(BASE_PISTOL_SLOT, BASE_PISTOL_GUID, PAWN_GUID)
}

/// A23 ch9: BasePistol handle 12 = pawn, seed 94 (hs:3107-3113, = build_channel_actor_owner_block(94, 10)).
pub fn pistol_owner_body() -> RepResult<Bits> {
    bodies::channel_actor_owner_block(BASE_PISTOL_GUID, PAWN_GUID)
}

/// A24 ch9: BasePistol RemoteRole = 2 in 3 bits, seed 94 (hs:3149-3156).
pub fn pistol_remote_role_body() -> RepResult<Bits> {
    actor_rep_body(&props::base_pistol_autonomous_role(props::REMOTE_ROLE_BITS)?, BASE_PISTOL_GUID)
}

/// A25 ch9, exports: EquippableStateMachine (96, outer 94) with the empty rep, seed 94 (hs:3188-3208).
pub fn pistol_state_machine_open_body() -> RepResult<Bits> {
    exported_subobject_rep_body(BASE_PISTOL_STATE_MACHINE_GUID, BASE_PISTOL_GUID, "EquippableStateMachine", BASE_PISTOL_STATE_MACHINE_GUID, &empty_rep(), BASE_PISTOL_GUID)
}

/// A26 ch9, exports: InactiveState (98) export + AuthCurrentState = 98 (hs:3239-3241).
pub fn pistol_inactive_state_body() -> RepResult<Bits> {
    bodies::equippable_state_link_body("InactiveState", BASE_PISTOL_INACTIVE_STATE_GUID)
}

/// A27 ch8, index 1..5: Inventory CurrentEquippable = 94, CorrectionIndex = index
/// (hs:3315-3333, = build_inventory_correction_body(index, 94)).
pub fn pistol_correction_body(index: u32) -> RepResult<Bits> {
    bodies::inventory_correction_body(index, Some(BASE_PISTOL_GUID))
}

// ---- part B (spec 04 section 8) ----

/// B1 ch5: ClientRestart(pawn 10), seed 4 (hs:3360-3364).
pub fn client_restart_body() -> RepResult<Bits> {
    rpc::controller_rpc_body(&rpc::client_restart()?)
}

/// B2 ch4: GameState.Phase, seed 2 (hs:3387-3392). production props::RANGE_PHASE.
pub fn game_state_phase_body(phase_value: u32) -> RepResult<Bits> {
    actor_rep_body(&props::phase(phase_value)?, GAME_STATE_GUID)
}

/// B3 ch10 + i open, exports: BaseTeamState 100 + 2i (hs:3419-3421).
pub fn team_state_open_body(team_index: usize) -> RepResult<Bits> {
    check(team_index < TEAM_STATE_GUIDS.len(), "team index out of range")?;
    actors::base_team_state_reference(TEAM_STATE_GUIDS[team_index])
}

/// B4 ch4: GameState.TeamStates = [100, 102], seed 2 (hs:3449-3454).
pub fn team_state_array_body() -> RepResult<Bits> {
    actor_rep_body(&props::object_array(TEAM_STATES_HANDLE, &TEAM_STATE_GUIDS)?, GAME_STATE_GUID)
}

/// B5 ch10 + i, exports: BlueTeam/RedTeam component export and team link with
/// role 1 + i (hs:3480-3484).
pub fn team_state_link_body(team_index: usize) -> RepResult<Bits> {
    check(team_index < TEAM_STATE_GUIDS.len(), "team index out of range")?;
    bodies::team_state_component_update(TEAM_STATE_GUIDS[team_index], TEAM_COMPONENT_GUIDS[team_index], TEAM_COMPONENT_NAMES[team_index], Some(1 + team_index as u32))
}

/// B6 ch6: PlayerState team = 100, seed 6 (hs:3512-3516).
pub fn player_state_team_body() -> RepResult<Bits> {
    actor_object_link(PLAYER_STATE_GUID, PLAYER_STATE_TEAM_HANDLE, TEAM_STATE_GUIDS[0])
}

/// B7 ch6, exports: BombTeam (108, outer 6) with the empty rep, seed 6 (hs:3548-3568).
pub fn bomb_team_open_body() -> RepResult<Bits> {
    exported_subobject_rep_body(BOMB_TEAM_GUID, PLAYER_STATE_GUID, "BombTeam", BOMB_TEAM_GUID, &empty_rep(), PLAYER_STATE_GUID)
}

/// python `value & ((1 << bits) - 1)` written as `bits` raw bits (hs:3604-3605).
/// python ints are unbounded two's complement, so every bit above 63 repeats
/// the sign: -1 with 70 bits is 70 ones, 5 with 70 bits is 5 then zeros.
fn append_masked_bits(w: &mut BitWriter, value: i64, bits: u32) {
    let fill = if value < 0 { u64::MAX } else { 0 };
    let mut chunk = value as u64;
    let mut left = bits;
    while left > 0 {
        let take = left.min(64);
        let mask = if take == 64 { u64::MAX } else { (1u64 << take) - 1 };
        w.append_bits(chunk & mask, take);
        chunk = fill;
        left -= take;
    }
}

/// B8 ch6: BombTeam.TeamNumber = value masked to `bits` bits, seed 6 (hs:3601-3615).
/// production BOMB_TEAM_NUMBER_VALUE / BOMB_TEAM_NUMBER_BITS (8 bits, still
/// unconfirmed against the live NetSerialize, hs:3594-3596).
pub fn bomb_team_number_body(value: i64, bits: u32) -> RepResult<Bits> {
    let mut rep = BitWriter::new();
    rep.append_bits(0, 1);
    rep.append_packed_uint(TEAM_NUMBER_HANDLE as u64);
    append_masked_bits(&mut rep, value, bits);
    rep.append_packed_uint(0);
    subobject_rep_body(BOMB_TEAM_GUID, &rep.finish()?, PLAYER_STATE_GUID)
}

/// B9 ch6: AccessibleCharacterList = [guid], seed 6 (hs:3646-3652). production props::WUSHU_CLASS_GUID.
pub fn accessible_character_list_body(guid: [u32; 4]) -> RepResult<Bits> {
    actor_rep_body(&props::guid_array(ACCESSIBLE_CHARACTER_LIST_HANDLE, &[guid])?, PLAYER_STATE_GUID)
}

/// B10 ch6: PlayerState.UniqueId, seed 6 (hs:3684-3689). production props::OFFLINE_PLAYER_ID.
pub fn player_unique_id_body(subject: &str) -> RepResult<Bits> {
    actor_rep_body(&props::player_unique_id(subject)?, PLAYER_STATE_GUID)
}

/// B11 ch6, exports: MoneyManagementComponent with Money = 9000 (hs:3718).
pub fn range_money_body() -> RepResult<Bits> {
    bodies::money_component_body(bodies::RANGE_MONEY, bodies::MONEY_COMPONENT_GUID)
}

/// B12 ch8: Pawn.Owner = controller 4, seed 10 (hs:3748-3752).
pub fn pawn_owner_body() -> RepResult<Bits> {
    actor_object_link(PAWN_GUID, ACTOR_OWNER_HANDLE, CONTROLLER_GUID)
}

/// B13 ch5, exports: AresHUD_C export + ClientSetHUD(109), seed 4 (hs:3789-3797).
/// field/max come from the live class cache; every logged run had 52/178.
pub fn hud_set_body(field_index: u32, class_max_index: u32) -> RepResult<Bits> {
    let hud = ARES_HUD_CLASS;
    let export = class_netguid_export(hud.class_name, hud.package_path, hud.class_guid, hud.package_guid)?;
    let payload = rpc::client_set_hud(field_index, class_max_index, hud.class_guid)?;
    let mut w = BitWriter::new();
    w.append_bits_value(&export);
    append_rpc_block(&mut w, Target::Actor, &payload, CONTROLLER_GUID)?;
    Ok(w.finish()?)
}

/// B14 ch4: MatchState = InProgress, seed 2 (hs:3876-3880).
pub fn match_state_body() -> RepResult<Bits> {
    actor_rep_body(&props::match_state("InProgress")?, GAME_STATE_GUID)
}

/// B15 ch4: AuthGameplayStartTimestamp = 0.0, seed 2 (hs:3912-3916).
pub fn gameplay_start_body() -> RepResult<Bits> {
    actor_rep_body(&props::gameplay_start(0.0)?, GAME_STATE_GUID)
}

/// B16 ch4: PlayerArray = [6], seed 2 (hs:3946-3951).
pub fn player_array_body() -> RepResult<Bits> {
    actor_rep_body(&props::object_array(PLAYER_ARRAY_HANDLE, &[PLAYER_STATE_GUID])?, GAME_STATE_GUID)
}

/// B17 ch4: range shop switches 44/48/49 = true, seed 2 (hs:3983-3987).
pub fn range_shop_body() -> RepResult<Bits> {
    actor_rep_body(&props::range_shop_settings(&props::RANGE_SHOP_SETTING_HANDLES)?, GAME_STATE_GUID)
}

/// B18 first frame, ch12 + i open, exports: the default open of equippable i (hs:4024).
/// the knife-cosmetic and ability-tuning opens replace it elsewhere.
pub fn starting_equippable_open_body(index: usize) -> RepResult<Bits> {
    actors::starting_equippable_open(index)
}

/// B18 second frame, ch12 + i: Owner = pawn, seed = the equippable (hs:4069).
pub fn starting_equippable_owner_body(index: usize) -> RepResult<Bits> {
    let (actor, _, _) = starting_equippable_guids(index)?;
    bodies::channel_actor_owner_block(actor, PAWN_GUID)
}

/// B18 third frame, ch8: UItemSlot_<EquippableSlot>.Contents = the equippable (hs:4070).
pub fn starting_equippable_contents_body(index: usize) -> RepResult<Bits> {
    let (actor, _, _) = starting_equippable_guids(index)?;
    bodies::item_slot_contents_block(STARTING_EQUIPPABLES[index].slot, actor, PAWN_GUID)
}

/// B19 ch13: knife RemoteRole = 2 in 3 bits, seed 204 (range_melee_equippable.py:60-74, hs:4114).
pub fn knife_remote_role_body() -> RepResult<Bits> {
    actor_rep_body(&props::base_pistol_autonomous_role(props::REMOTE_ROLE_BITS)?, KNIFE_GUID)
}

/// B20 ch13, exports: knife EquippableStateMachine (206, outer 204) with the
/// empty rep, seed 204 (range_melee_equippable.py:89-109, hs:4144).
pub fn knife_state_machine_open_body() -> RepResult<Bits> {
    exported_subobject_rep_body(KNIFE_STATE_MACHINE_GUID, KNIFE_GUID, "EquippableStateMachine", KNIFE_STATE_MACHINE_GUID, &empty_rep(), KNIFE_GUID)
}

/// B21 ch13, exports: knife InactiveState (256, outer 204) + AuthCurrentState = 256
/// on SM 206, seed 204 (range_melee_equippable.py:112-139, hs:4174).
pub fn knife_inactive_state_body() -> RepResult<Bits> {
    let plain = object_property(AUTH_CURRENT_STATE_HANDLE, KNIFE_INACTIVE_STATE_GUID)?;
    exported_subobject_rep_body(KNIFE_INACTIVE_STATE_GUID, KNIFE_GUID, "InactiveState", KNIFE_STATE_MACHINE_GUID, &plain, KNIFE_GUID)
}

/// B22 ch5, exports: ViewTarget component (50, outer 4) with TargetActor (3) =
/// pawn, seed 4 (hs:4281-4313, viewtarget_transform on).
pub fn viewtarget_body() -> RepResult<Bits> {
    let plain = object_property(VIEWTARGET_HANDLE, PAWN_GUID)?;
    exported_subobject_rep_body(VIEWTARGET_GUID, CONTROLLER_GUID, VIEWTARGET_NAME, VIEWTARGET_GUID, &plain, CONTROLLER_GUID)
}

// ---- move and input acks (spec 04 section 9), ch8 unreliable ----

/// the first move ack: ClientMoveResponsePacked placement correction at
/// `location` with zero velocity, seed 10 (hs:4483-4499). production MOVE_CORRECTION_LOCATION.
pub fn placement_correction_body(move_number: u32, location: [f64; 3]) -> RepResult<Bits> {
    rpc::pawn_rpc_body(&rpc::correction_rpc(move_number, location, MOVE_RESPONSE_FIELD, PAWN_CLASS_MAX, [0.0; 3])?)
}

/// every later move ack: ClientAckInputAndMovement(0xFFFFFFFF, move), seed 10 (hs:4493-4499).
pub fn move_ack_body(move_number: u32) -> RepResult<Bits> {
    rpc::pawn_rpc_body(&rpc::input_ack(NO_INPUT_EVENT, move_number, ACK_INPUT_AND_MOVEMENT_FIELD, PAWN_CLASS_MAX)?)
}

/// input-ack trial: ClientAckInputAndMovement(latest event id, 0), seed 10 (hs:1541-1546).
pub fn input_ack_body(latest_event_id: u32) -> RepResult<Bits> {
    rpc::pawn_rpc_body(&rpc::input_ack(latest_event_id, 0, ACK_INPUT_AND_MOVEMENT_FIELD, PAWN_CLASS_MAX)?)
}

// ---- per-packet bodies after bring-up (spec 02 section 9.1, 9.2) ----

/// ch8: the input-resume correction (hs:1734-1741). CurrentEquippable is the
/// purchased guid for the live class, else equippable_guid_for_class, else left
/// out (python `a or b`, so a purchased guid of 0 also falls through). pass ""
/// when the live class is unknown (python `class or ""`).
pub fn input_resume_correction_body(correction_index: u32, purchased_guid: Option<u32>, equippable_class: &str) -> RepResult<Bits> {
    let equippable_guid = purchased_guid.filter(|&guid| guid != 0).or_else(|| actors::equippable_guid_for_class(equippable_class));
    bodies::inventory_correction_body(correction_index, equippable_guid)
}

/// ch4, exports: a GameState class property, seed 2 (hs:1789-1792).
pub fn range_class_property_body(property: &ClassProperty) -> RepResult<Bits> {
    bodies::actor_class_property_body(GAME_STATE_GUID, property.class_name, property.package, property.class_guid, property.package_guid, property.handle)
}

/// ch5: ClientSetHUD(109) again without the export, seed 4 (hs:1793-1796).
pub fn hud_rebuild_body(field_index: u32, class_max_index: u32) -> RepResult<Bits> {
    rpc::controller_rpc_body(&rpc::client_set_hud(field_index, class_max_index, ARES_HUD_CLASS.class_guid)?)
}

/// ch5: ClientUpdateMultipleLevelsStreamingStatus(range sublevels), seed 4 (hs:1813-1820).
pub fn levels_streaming_body() -> RepResult<Bits> {
    let payload = rpc::levels_streaming_status(&rpc::RANGE_GAMEPLAY_SUBLEVELS, rpc::LEVELS_STREAMING_STATUS_FIELD, rpc::CONTROLLER_CLASS_MAX, true, true)?;
    rpc::controller_rpc_body(&payload)
}

/// ch5: ClientFlushLevelStreaming, seed 4 (hs:1816-1820).
pub fn flush_level_streaming_body() -> RepResult<Bits> {
    rpc::controller_rpc_body(&rpc::flush_level_streaming(rpc::FLUSH_LEVEL_STREAMING_FIELD, rpc::CONTROLLER_CLASS_MAX)?)
}

/// ch6: NumUltimatePoints (197) = points, seed 6 (hs:1842-1843).
pub fn ultimate_points_body(points: i32) -> RepResult<Bits> {
    bodies::actor_int_property_body(PLAYER_STATE_GUID, bodies::PLAYER_STATE_ULTIMATE_POINTS_HANDLE, points)
}

/// ch5: the range settings request (field 170), seed 4 (hs:1922-1923).
pub fn range_settings_request_body() -> RepResult<Bits> {
    rpc::controller_rpc_body(&rpc::update_range_settings(rpc::UPDATE_RANGE_SETTINGS_FIELD, rpc::CONTROLLER_CLASS_MAX)?)
}

/// ch4: GameInfo infinite ammo (46), seed 2 (hs:1943-1944).
pub fn infinite_ammo_body(enabled: bool) -> RepResult<Bits> {
    bodies::game_state_bool_body(bodies::GAME_INFO_INFINITE_AMMO_HANDLE, enabled)
}

/// ch6: SpawnedCharacter (194) = null, seed 6; the hud refresh sends this then
/// player_state_spawned_character_link (hs:2012-2021).
pub fn spawned_character_null_body() -> RepResult<Bits> {
    actor_rep_body(&props::null_object_property(SPAWNED_CHARACTER_HANDLE)?, PLAYER_STATE_GUID)
}
