//! wushu's starting knife on ch13: its role, state machine and states
//! (range_melee_equippable.py) and its saved-skin open (range_melee_cosmetics.py).
//! spec 07 2.3 / 7.1 / 7.3, bring-up b18-b21 in 04.
//!
//! the knife is Ability_Melee_Base_C, opened as netguid 204 by the starting
//! equippables stage. its live 13.06 FRepLayout matches BasePistol's through
//! every cosmetic field (pointers 18/19/21, FGuid leaves 24/28/41), so it takes
//! the same initial actor-open shape as the pistol (range_melee_cosmetics.py:3-11).

use aries_bits::Bits;
use aries_rep::RepResult;
use aries_rep::block::dynamic_actor_reference;

use crate::cosmetics::{COSMETIC_POINTER_HANDLES, COSMETIC_UUID_HANDLES, CosmeticActorLayout, RANGE_LEVEL_GUID, build_catalog_cosmetic_actor_open};
use crate::equippables::{STARTING_EQUIPPABLES, StateGuids, auth_state_body, build_machine_open_body, remote_role_body, starting_equippable_guids};
use crate::skins::{EquippedCdoPaths, SkinError, SkinInventory, resolve_equipped_cdo_paths};

/// index of the knife in wushu's starting equippables (range_melee_cosmetics.py:26).
pub const MELEE_STARTING_INDEX: usize = 1;
/// range_melee_equippable.py:17.
pub const MELEE_CHANNEL: u32 = 13;
/// starting_equippable_guids(1)[0] (range_melee_equippable.py:18).
pub const MELEE_ACTOR_GUID: u32 = starting_equippable_guids(MELEE_STARTING_INDEX).0;
/// the actor+2 gap left by starting_equippable_guids(1) (range_melee_equippable.py:23).
pub const MELEE_STATE_MACHINE_GUID: u32 = MELEE_ACTOR_GUID + 2;
/// EAresItemSlot Melee (range_equippables.py:329).
pub const MELEE_SLOT: u32 = 2;
/// range_melee_equippable.py:57.
pub const MELEE_AUTH_CURRENT_STATE_HANDLE: u32 = 3;
pub const MELEE_INACTIVE_STATE_GUID: u32 = 256;
pub const MELEE_EQUIP_STATE_GUID: u32 = 258;
/// further states, read from the live 13.06 Ability_Melee_Base_C machine; they
/// export as 260 + 2*i in this fixed order (range_melee_equippable.py:26-56).
pub const ADDITIONAL_MELEE_STATE_NAMES: [&str; 23] = [
    "ReadyingState",
    "IdleWaitingForEventState",
    "RespondToEventState",
    "FirstAttackIdle",
    "AnimTriggeredState1",
    "MeleeAttackState1",
    "WaitForComboInput1",
    "MeleeAttackState2",
    "WaitForComboInput2",
    "AnimTriggeredState2",
    "WaitForComboInput3",
    "AnimTriggeredState3",
    "MeleeAttackState3",
    "AnimTriggeredState4",
    "MeleeAttackState4",
    "WaitForComboInput4",
    "AnimTriggeredState_Alt",
    "MeleeAttackState_Alt",
    "WaitForAltAttack",
    "GhostEquipState",
    "AnimTriggeredState5",
    "MeleeAttackState5",
    "WaitForComboInput5",
];
const FIRST_ADDITIONAL_MELEE_STATE_GUID: u32 = 260;

/// python melee_state_guids (range_melee_equippable.py:51-56), in order.
pub fn melee_state_guids() -> StateGuids {
    let mut guids = StateGuids::from_iter([("InactiveState", MELEE_INACTIVE_STATE_GUID), ("EquipState", MELEE_EQUIP_STATE_GUID)]);
    for (index, name) in ADDITIONAL_MELEE_STATE_NAMES.iter().enumerate() {
        guids.insert(*name, FIRST_ADDITIONAL_MELEE_STATE_GUID + 2 * index as u32);
    }
    guids
}

/// python build_melee_autonomous_role_body (range_melee_equippable.py:60-74):
/// RemoteRole=2 on ch13, giving the client Role 2.
pub fn build_melee_autonomous_role_body() -> RepResult<Bits> {
    remote_role_body(MELEE_ACTOR_GUID)
}

/// python build_melee_state_machine_open (range_melee_equippable.py:89-109):
/// exports EquippableStateMachine with an empty layout (creates its replicator).
pub fn build_melee_state_machine_open() -> RepResult<Bits> {
    build_machine_open_body(MELEE_ACTOR_GUID, MELEE_STATE_MACHINE_GUID, "EquippableStateMachine")
}

/// python build_melee_auth_state_body (range_melee_equippable.py:112-139).
pub fn build_melee_auth_state_body(state_name: &str, export: bool) -> RepResult<Bits> {
    let Some(state_guid) = melee_state_guids().get(state_name) else {
        return Err(aries_rep::RepError::Invalid("melee state must be a live-verified named subobject".into()));
    };
    auth_state_body(MELEE_ACTOR_GUID, MELEE_STATE_MACHINE_GUID, state_guid, state_name, export)
}

/// the knife's weapon uuid (range_melee_cosmetics.py:25).
pub const MELEE_WEAPON_ID: &str = "2f59173c-4bed-b6c3-2191-dea9b58be9c7";
/// odd static guids for the knife's skin / level / chroma cdos and packages;
/// free in the responder: pistol cosmetics are 229-239 (range_melee_cosmetics.py:31-34).
pub const MELEE_COSMETIC_GUID_PAIRS: [(u32, u32); 3] = [(241, 243), (245, 247), (249, 251)];

/// python starting_melee_cosmetic_layout (range_melee_cosmetics.py:36-48).
pub fn starting_melee_cosmetic_layout() -> CosmeticActorLayout {
    let (archetype, package, _) = STARTING_EQUIPPABLES[MELEE_STARTING_INDEX];
    let (actor, archetype_guid, package_guid) = starting_equippable_guids(MELEE_STARTING_INDEX);
    CosmeticActorLayout {
        weapon_id: MELEE_WEAPON_ID.to_string(),
        channel_index: MELEE_CHANNEL,
        actor_net_guid: actor,
        archetype_name: archetype.to_string(),
        archetype_package: package.to_string(),
        archetype_net_guid: archetype_guid,
        package_net_guid: package_guid,
        level_net_guid: RANGE_LEVEL_GUID,
        pointer_handles: COSMETIC_POINTER_HANDLES,
        uuid_handles: COSMETIC_UUID_HANDLES,
        asset_guid_pairs: MELEE_COSMETIC_GUID_PAIRS,
    }
}

/// python select_saved_melee_cdo_paths (range_melee_cosmetics.py:53-75): the
/// owned/equipped knife skin, None when unskinned or any part is unpackaged.
pub fn select_saved_melee_cdo_paths(inventory: &SkinInventory) -> Result<Option<EquippedCdoPaths>, SkinError> {
    Ok(resolve_equipped_cdo_paths(inventory, MELEE_WEAPON_ID)?.filter(EquippedCdoPaths::all_packaged))
}

/// python build_starting_melee_cosmetic_open (range_melee_cosmetics.py:78-80).
pub fn build_starting_melee_cosmetic_open(cdo_paths: &EquippedCdoPaths) -> RepResult<Bits> {
    build_catalog_cosmetic_actor_open(cdo_paths, &starting_melee_cosmetic_layout(), &[])
}

/// the plain knife open, python build_starting_equippable_open(1)
/// (range_server_bunch.py:715-718): the fallback when no skin applies.
pub fn melee_plain_open() -> RepResult<Bits> {
    let (archetype, package, _) = STARTING_EQUIPPABLES[MELEE_STARTING_INDEX];
    let (actor, archetype_guid, package_guid) = starting_equippable_guids(MELEE_STARTING_INDEX);
    Ok(dynamic_actor_reference((archetype, package), actor, archetype_guid, package_guid, RANGE_LEVEL_GUID, None)?)
}

/// which knife open b18 i=1 sent (python events melee_initial_*).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeleeOpen {
    /// the saved skin (melee_initial_skin_id / _level_id / _chroma_id)
    Cosmetic(Box<EquippedCdoPaths>),
    /// no knife skin equipped or not packaged (melee_initial_cosmetic_unselected)
    Unselected,
    /// selection or build failed; the plain open went out (melee_initial_cosmetic_error)
    Error(String),
}

/// the b18 i=1 knife open decision (range_udp_handshake.py:4024-4044): the saved
/// knife skin when it resolves and builds, else the plain open. any error falls
/// back. `inventory` is the cached SkinInventory (or its load error).
pub fn melee_open(inventory: Result<&SkinInventory, &SkinError>) -> RepResult<(Bits, MeleeOpen)> {
    let selection = inventory.map_err(|error| error.to_string()).and_then(|inventory| select_saved_melee_cdo_paths(inventory).map_err(|error| error.to_string()));
    match selection {
        Ok(Some(paths)) => match build_starting_melee_cosmetic_open(&paths) {
            Ok(body) => Ok((body, MeleeOpen::Cosmetic(Box::new(paths)))),
            Err(error) => Ok((melee_plain_open()?, MeleeOpen::Error(error.to_string()))),
        },
        Ok(None) => Ok((melee_plain_open()?, MeleeOpen::Unselected)),
        Err(error) => Ok((melee_plain_open()?, MeleeOpen::Error(error))),
    }
}
