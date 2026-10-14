//! complete bunch bodies shared by many senders (range_server_bunch 6.2).

use aries_bits::{BitWriter, Bits};

use crate::actors::{BASE_PISTOL_GUID, GAME_STATE_GUID, PLAYER_STATE_GUID, class_netguid_export};
use crate::block::{Target, append_rep_block, append_stable_child_export};
use crate::props::{bool_property, check, int_property, object_property, ordered};
use crate::RepResult;

/// AActor.Owner rep handle.
pub const ACTOR_OWNER_HANDLE: u32 = 12;
/// UItemSlot.Contents rep handle.
pub const ITEM_SLOT_CONTENTS_HANDLE: u32 = 1;
pub const PAWN_GUID: u32 = 10;
pub const INVENTORY_GUID: u32 = 60;
pub const ITEM_SLOT_CLASS_GUID: u32 = 61;
pub const FIRST_ITEM_SLOT_GUID: u32 = 62;

/// net guid of UItemSlot_<slot> (dynamic subobject of the inventory).
pub fn item_slot_net_guid(slot: u32) -> u32 {
    FIRST_ITEM_SLOT_GUID + 2 * slot
}

/// python build_channel_actor_owner_block: rep block on the channel actor, Owner = owner.
pub fn channel_actor_owner_block(actor_guid: u32, owner_guid: u32) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    append_rep_block(&mut w, Target::Actor, &object_property(ACTOR_OWNER_HANDLE, owner_guid)?, actor_guid)?;
    Ok(w.finish()?)
}

/// python build_item_slot_contents_block: dynamic-subobject block on the slot
/// (class 61, outer inventory 60) setting Contents, encoded with the pawn guid.
pub fn item_slot_contents_block(slot: u32, item_guid: u32, pawn_guid: u32) -> RepResult<Bits> {
    let plain = object_property(ITEM_SLOT_CONTENTS_HANDLE, item_guid)?;
    let encoded = crate::encode_block_payload(&plain, pawn_guid)?;
    let mut w = BitWriter::new();
    w.append_bits(1, 1); // bHasRepLayout
    w.append_bits(0, 1); // subobject
    w.append_packed_uint(item_slot_net_guid(slot) as u64);
    w.append_bits(0, 1); // not stably named
    w.append_bits(0, 1); // extra writer bit
    w.append_packed_uint(ITEM_SLOT_CLASS_GUID as u64);
    w.append_bits(0, 1); // actor is not the outer
    w.append_packed_uint(INVENTORY_GUID as u64);
    w.append_packed_uint(encoded.bit_count as u64);
    w.append_bits_value(&encoded);
    Ok(w.finish()?)
}

/// python build_inventory_correction_body: AuthServerCorrectRepVariables on the
/// inventory (60): CurrentEquippable (25, optional) and CorrectionIndex (26, u32).
pub fn inventory_correction_body(correction_index: u32, current_equippable_guid: Option<u32>) -> RepResult<Bits> {
    if correction_index == 0 {
        return Err(crate::RepError::Invalid("correction index must be > 0".into()));
    }
    let mut rep = BitWriter::new();
    rep.append_bits(0, 1);
    if let Some(guid) = current_equippable_guid {
        rep.append_packed_uint(25);
        rep.append_packed_uint(guid as u64);
    }
    rep.append_packed_uint(26);
    rep.append_u32(correction_index);
    rep.append_packed_uint(0);
    let mut w = BitWriter::new();
    append_rep_block(&mut w, Target::Subobject(INVENTORY_GUID), &rep.finish()?, PAWN_GUID)?;
    Ok(w.finish()?)
}

// ---- complete production bodies built in rsb and the inline subobject shapes (spec 02 section 6.2) ----

/// NumUltimatePoints on PlayerState ch6 (rsb:858).
pub const PLAYER_STATE_ULTIMATE_POINTS_HANDLE: u32 = 197;
/// GameInfo infinite-ammo switch on GameState ch4 (rsb:924).
pub const GAME_INFO_INFINITE_AMMO_HANDLE: u32 = 46;
/// MoneyManagementComponent stable subobject of PlayerState 6, Money handle 3 (rsb:996-998).
pub const MONEY_COMPONENT_GUID: u32 = 254;
pub const MONEY_HANDLE: u32 = 3;
pub const RANGE_MONEY: i32 = 9000;
/// BasePistol EquippableStateMachine and its InactiveState (rsb:756-758).
pub const BASE_PISTOL_STATE_MACHINE_GUID: u32 = 96;
pub const BASE_PISTOL_INACTIVE_STATE_GUID: u32 = 98;
/// EquippableStateMachine.AuthCurrentState.CurrentState (rsb:768).
pub const AUTH_CURRENT_STATE_HANDLE: u32 = 3;

/// a TSubclassOf property set on a channel actor by exporting its class path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassProperty {
    pub class_name: &'static str,
    pub package: &'static str,
    pub class_guid: u32,
    pub package_guid: u32,
    pub handle: u32,
}

/// GameState GameModeClass (live ch4 handle 15, rsb:864).
pub const RANGE_GAME_MODE_CLASS: ClassProperty = ClassProperty {
    class_name: "ShootingRangeGameMode_C",
    package: "/Game/GameModes/ShootingRange/ShootingRangeGameMode",
    class_guid: 261,
    package_guid: 263,
    handle: 15,
};
/// GameState GameStateHUDConfig (live ch4 handle 58, rsb:866).
pub const RANGE_HUD_CONFIG_CLASS: ClassProperty = ClassProperty {
    class_name: "ShootingRangeHUDConfig_C",
    package: "/Game/UI/InGame/HUD/GameStateHUDConfigs/ShootingRangeHUDConfig",
    class_guid: 265,
    package_guid: 267,
    handle: 58,
};

/// rep block on the channel actor: [1][1][pk bits][enc(plain, actor)]
/// (python append_actor_rep_layout_payload(b"", 0, enc, bits), rsb:244).
pub fn actor_rep_body(plain: &Bits, actor_guid: u32) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    append_rep_block(&mut w, Target::Actor, plain, actor_guid)?;
    Ok(w.finish()?)
}

/// rep block on a stably-named subobject: [1][0][pk sub][1][pk bits][enc(plain, actor)]
/// (inline hs:2840-2851, 3601-3615; the seed is the channel actor).
pub fn subobject_rep_body(subobject_guid: u32, plain: &Bits, actor_guid: u32) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    append_rep_block(&mut w, Target::Subobject(subobject_guid), plain, actor_guid)?;
    Ok(w.finish()?)
}

/// export one stably-named object of a known outer, then a rep block on the
/// stably-named `target_guid`: every inline subobject open/link (hs:2706, 3188,
/// 3548, 4281, hs:118, rsb:1001, range_melee_equippable.py:89-139).
pub fn exported_subobject_rep_body(object_guid: u32, outer_guid: u32, object_name: &str, target_guid: u32, plain: &Bits, actor_guid: u32) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    append_stable_child_export(&mut w, object_guid, outer_guid, object_name);
    append_rep_block(&mut w, Target::Subobject(target_guid), plain, actor_guid)?;
    Ok(w.finish()?)
}

/// python build_actor_int_property_body (rsb:842): one int32 leaf on the channel actor.
pub fn actor_int_property_body(actor_guid: u32, handle: u32, value: i32) -> RepResult<Bits> {
    check(value >= 0 && handle > 0, "expected a positive handle and a non-negative int32")?;
    actor_rep_body(&int_property(handle, value as u32)?, actor_guid)
}

/// python build_game_state_bool_body (rsb:944): one GameState bool, seed 2.
pub fn game_state_bool_body(handle: u32, value: bool) -> RepResult<Bits> {
    check((1..=63).contains(&handle), "bool field handle outside bounded one-byte range")?;
    actor_rep_body(&bool_property(handle, value)?, GAME_STATE_GUID)
}

/// python build_actor_class_property_body (rsb:961): class export, then the
/// property set to the class guid on the channel actor.
pub fn actor_class_property_body(actor_guid: u32, class_name: &str, package: &str, class_guid: u32, package_guid: u32, handle: u32) -> RepResult<Bits> {
    let export = class_netguid_export(class_name, package, class_guid, package_guid)?;
    check((1..=0xFFFF).contains(&handle) && (1..=0xFFFF).contains(&class_guid), "object field outside supported range")?;
    let mut w = BitWriter::new();
    w.append_bits_value(&export);
    append_rep_block(&mut w, Target::Actor, &object_property(handle, class_guid)?, actor_guid)?;
    Ok(w.finish()?)
}

/// python build_money_component_body (rsb:1001): export the component (outer
/// PlayerState 6) and set Money, seed 6.
pub fn money_component_body(money: i32, component_guid: u32) -> RepResult<Bits> {
    check(money >= 0, "money must be a non-negative int32")?;
    exported_subobject_rep_body(component_guid, PLAYER_STATE_GUID, "MoneyManagementComponent", component_guid, &int_property(MONEY_HANDLE, money as u32)?, PLAYER_STATE_GUID)
}

/// python build_team_state_component_update (rsb:1351): export the GameState's
/// team component, then TeamComponent (16) and, with a role, InitialRole/TeamRole
/// (17/18, 3-bit EAresTeamRole) on the team actor.
pub fn team_state_component_update(actor_guid: u32, component_guid: u32, component_name: &str, role: Option<u32>) -> RepResult<Bits> {
    check(matches!(actor_guid, 100 | 102) && matches!(component_guid, 104 | 106), "team or component GUID is not reserved")?;
    check(matches!(component_name, "BlueTeam" | "RedTeam") && matches!(role, None | Some(1 | 2)), "team component name or role outside verified Range set")?;
    let mut component = BitWriter::new();
    component.append_packed_uint(component_guid as u64);
    let mut fields = vec![(16, component.finish()?)];
    if let Some(role) = role {
        let mut role_value = BitWriter::new();
        role_value.append_bits(role as u64, 3);
        let role_value = role_value.finish()?;
        fields.push((17, role_value.clone()));
        fields.push((18, role_value));
    }
    let mut w = BitWriter::new();
    append_stable_child_export(&mut w, component_guid, GAME_STATE_GUID, component_name);
    append_rep_block(&mut w, Target::Actor, &ordered(&fields)?, actor_guid)?;
    Ok(w.finish()?)
}

/// python range_udp_handshake.build_equippable_state_link_body (hs:118): export a
/// pistol state (outer 94), then AuthCurrentState = that state on the SM (96), seed 94.
pub fn equippable_state_link_body(state_name: &str, state_guid: u32) -> RepResult<Bits> {
    check(matches!(state_name, "InactiveState" | "EquipState") && matches!(state_guid, 98 | 99), "unsupported equippable state trial")?;
    let plain = object_property(AUTH_CURRENT_STATE_HANDLE, state_guid)?;
    exported_subobject_rep_body(state_guid, BASE_PISTOL_GUID, state_name, BASE_PISTOL_STATE_MACHINE_GUID, &plain, BASE_PISTOL_GUID)
}
