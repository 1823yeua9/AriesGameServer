//! actor-open bodies: static/dynamic actor references and class exports (spec 02 section 3). owner: I1.
//! every open body starts with netguid exports, so its bunch carries has_exports.

use aries_bits::{BitWriter, Bits};

use crate::block::{Spawn, append_export_header, append_static_export_entry, dynamic_actor_reference};
use crate::props::check;
use crate::RepResult;

/// the Range PersistentLevel every dynamic actor opens in (rsb:143 level_net_guid default).
pub const LEVEL_GUID: u32 = 5;
/// AresWorldSettings and its outers, actor first (hs:2196-2197); exported as 3, 5, 7, 9.
pub const WORLD_SETTINGS_CHAIN: [&str; 4] = ["AresWorldSettings", "PersistentLevel", "Range", "/Game/Maps/Poveglia/Range"];
/// rsb:118 actor_net_guid default.
pub const WORLD_SETTINGS_GUID: u32 = 3;
/// pawn spawn literal (hs:2357-2360), also the --move-correction-location (launch.ps1:176).
pub const PAWN_SPAWN_LOCATION: [f64; 3] = [3581.634, -12019.322, 1294.153];

pub const GAME_STATE_GUID: u32 = 2;
pub const CONTROLLER_GUID: u32 = 4;
pub const PLAYER_STATE_GUID: u32 = 6;
pub const PLAYER_INFO_GUID: u32 = 8;
/// rsb:755 base_pistol_actor_guid.
pub const BASE_PISTOL_GUID: u32 = 94;

/// a dynamic actor: its class-default archetype path and the guids we export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActorArchetype {
    pub archetype: &'static str,
    pub package: &'static str,
    pub actor_guid: u32,
    pub archetype_guid: u32,
    pub package_guid: u32,
}

impl ActorArchetype {
    /// python build_dynamic_actor_reference(chain, actor, archetype, package, level 5, spawn).
    pub fn reference(&self, spawn: Option<Spawn>) -> RepResult<Bits> {
        Ok(dynamic_actor_reference((self.archetype, self.package), self.actor_guid, self.archetype_guid, self.package_guid, LEVEL_GUID, spawn)?)
    }
}

/// rsb:198-203
pub const GAME_STATE: ActorArchetype = ActorArchetype {
    archetype: "Default__ShootingRangeGameState_C",
    package: "/Game/GameModes/ShootingRange/ShootingRangeGameState",
    actor_guid: GAME_STATE_GUID,
    archetype_guid: 11,
    package_guid: 13,
};
/// rsb:1468-1473
pub const CONTROLLER: ActorArchetype = ActorArchetype {
    archetype: "Default__ShootingRangeController_C",
    package: "/Game/Characters/_Core/ShootingRangeController",
    actor_guid: CONTROLLER_GUID,
    archetype_guid: 15,
    package_guid: 17,
};
/// rsb:1480-1485
pub const PLAYER_STATE: ActorArchetype = ActorArchetype {
    archetype: "Default__ShootingRangePlayerState_C",
    package: "/Game/GameModes/ShootingRange/ShootingRangePlayerState",
    actor_guid: PLAYER_STATE_GUID,
    archetype_guid: 19,
    package_guid: 21,
};
/// rsb:1488-1492
pub const PLAYER_INFO: ActorArchetype = ActorArchetype {
    archetype: "Default__OwnerExclusivePlayerInfo",
    package: "/Script/ShooterGame",
    actor_guid: PLAYER_INFO_GUID,
    archetype_guid: 23,
    package_guid: 25,
};
/// rsb:1495-1502 (actor 10 = bodies::PAWN_GUID)
pub const WUSHU_PAWN: ActorArchetype = ActorArchetype {
    archetype: "Default__Wushu_PC_C",
    package: "/Game/Characters/Wushu/Wushu_PC",
    actor_guid: 10,
    archetype_guid: 27,
    package_guid: 29,
};
/// hs:3002-3007 (plain open, no packaged cosmetic)
pub const BASE_PISTOL: ActorArchetype = ActorArchetype {
    archetype: "Default__BasePistol_C",
    package: "/Game/Equippables/Guns/Sidearms/BasePistol/BasePistol",
    actor_guid: BASE_PISTOL_GUID,
    archetype_guid: 95,
    package_guid: 97,
};
/// rsb:206-218: archetype 101, package 103 shared by both team actors.
pub const BASE_TEAM_STATE_ARCHETYPE: (&str, &str) = ("Default__BaseTeamState", "/Script/ShooterGame");
pub const TEAM_STATE_GUIDS: [u32; 2] = [100, 102];

/// python build_exported_static_actor_reference (rsb:118): the outer chain
/// exported as actor + 2i, names reversed, then the cached actor guid.
pub fn exported_static_actor_reference(outer_chain: &[&str], actor_net_guid: u32) -> RepResult<Bits> {
    check((2..=8).contains(&outer_chain.len()) && actor_net_guid >= 3 && actor_net_guid & 1 == 1, "invalid static actor export")?;
    let mut w = BitWriter::new();
    append_export_header(&mut w, 1);
    for index in 0..outer_chain.len() as u64 {
        w.append_packed_uint(actor_net_guid as u64 + 2 * index); // fails past u32 like python
        w.append_bits(5, 8); // inline path plus checksum
    }
    w.append_packed_uint(0);
    for component in outer_chain.iter().rev() {
        w.append_fstring_ascii(component);
        w.append_bits(0, 32);
    }
    w.append_packed_uint(actor_net_guid as u64);
    Ok(w.finish()?)
}

/// A01 ch3 open: hs:2196.
pub fn world_settings_reference() -> RepResult<Bits> {
    exported_static_actor_reference(&WORLD_SETTINGS_CHAIN, WORLD_SETTINGS_GUID)
}

/// python build_dynamic_range_game_state_reference (rsb:198).
pub fn game_state_reference() -> RepResult<Bits> {
    GAME_STATE.reference(None)
}

/// python build_dynamic_range_controller_reference (rsb:1468): the dynamic open
/// plus 8 zero bits, the primary local-player index the open override reads.
pub fn controller_reference() -> RepResult<Bits> {
    let open = CONTROLLER.reference(None)?;
    let mut w = BitWriter::new();
    w.append_bits_value(&open);
    w.append_bits(0, 8);
    Ok(w.finish()?)
}

/// python build_dynamic_range_player_state_reference (rsb:1480).
pub fn player_state_reference() -> RepResult<Bits> {
    PLAYER_STATE.reference(None)
}

/// python build_dynamic_player_info_reference (rsb:1488).
pub fn player_info_reference() -> RepResult<Bits> {
    PLAYER_INFO.reference(None)
}

/// python build_dynamic_wushu_pawn_reference (rsb:1495): location only, no yaw.
pub fn wushu_pawn_reference(spawn_location: Option<[f64; 3]>) -> RepResult<Bits> {
    WUSHU_PAWN.reference(spawn_location.map(|location| Spawn { location, yaw: 0.0 }))
}

/// the plain BasePistol open (hs:3002-3007); the cosmetic open is range_weapon_cosmetics'.
pub fn base_pistol_reference() -> RepResult<Bits> {
    BASE_PISTOL.reference(None)
}

/// python build_dynamic_base_team_state_reference (rsb:206): guid 100 or 102 only.
pub fn base_team_state_reference(actor_net_guid: u32) -> RepResult<Bits> {
    check(TEAM_STATE_GUIDS.contains(&actor_net_guid), "team actor GUID is not reserved for this session")?;
    Ok(dynamic_actor_reference(BASE_TEAM_STATE_ARCHETYPE, actor_net_guid, 101, 103, LEVEL_GUID, None)?)
}

/// a class path exported for an rpc parameter or a TSubclassOf property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassPath {
    pub class_name: &'static str,
    pub package_path: &'static str,
    pub class_guid: u32,
    pub package_guid: u32,
}

/// the ClientSetHUD class (hs:3789-3790).
pub const ARES_HUD_CLASS: ClassPath = ClassPath { class_name: "AresHUD_C", package_path: "/Game/UI/InGame/HUD/AresHUD", class_guid: 109, package_guid: 111 };

/// python build_class_netguid_export (rsb:221): one /Game/ class path, both
/// guids odd, distinct and >= 3.
pub fn class_netguid_export(class_name: &str, package_path: &str, class_net_guid: u32, package_net_guid: u32) -> RepResult<Bits> {
    check(
        !class_name.is_empty()
            && package_path.starts_with("/Game/")
            && class_net_guid.min(package_net_guid) >= 3
            && class_net_guid != package_net_guid
            && class_net_guid & 1 == 1
            && package_net_guid & 1 == 1,
        "invalid class NetGUID export",
    )?;
    let mut w = BitWriter::new();
    append_export_header(&mut w, 1);
    append_static_export_entry(&mut w, class_name, package_path, class_net_guid, package_net_guid);
    Ok(w.finish()?)
}

/// one of Wushu's StartingEquippableClasses (+0xBC8) and its EquippableSlot (+0x4A0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartingEquippable {
    pub archetype: &'static str,
    pub package: &'static str,
    pub slot: u32,
}

/// python wushu_starting_equippables (rsb:680-691), live 13.06 order.
pub const STARTING_EQUIPPABLES: [StartingEquippable; 7] = [
    StartingEquippable { archetype: "Default__Equippable_Unarmed_C", package: "/Game/Characters/_Core/Equippable_Unarmed", slot: 10 },
    StartingEquippable { archetype: "Default__Ability_Melee_Base_C", package: "/Game/Equippables/Melee/Ability_Melee_Base", slot: 2 },
    StartingEquippable { archetype: "Default__Ability_Wushu_4_Smoke_C", package: "/Game/Characters/Wushu/S0/Ability_4/Ability_Wushu_4_Smoke", slot: 3 },
    StartingEquippable { archetype: "Default__Ability_Wushu_E_Dash_C", package: "/Game/Characters/Wushu/S0/Ability_E/Ability_Wushu_E_Dash", slot: 5 },
    StartingEquippable { archetype: "Default__Ability_Wushu_Q_CycloneBoost_C", package: "/Game/Characters/Wushu/S0/Ability_Q/Ability_Wushu_Q_CycloneBoost", slot: 4 },
    StartingEquippable { archetype: "Default__Ability_Wushu_X_Dagger_Production_C", package: "/Game/Characters/Wushu/S0/Ability_X/Ability_Wushu_X_Dagger_Production", slot: 9 },
    StartingEquippable { archetype: "Default__Ability_Wushu_Passive_Glide_C", package: "/Game/Characters/Wushu/S0/Glide/Ability_Wushu_Passive_Glide", slot: 6 },
];
/// rsb:692 starting_equippable_first_channel.
pub const STARTING_EQUIPPABLE_FIRST_CHANNEL: u32 = 12;
/// rsb:693 starting_equippable_first_guid.
pub const STARTING_EQUIPPABLE_FIRST_GUID: u32 = 200;

/// python starting_equippable_guids (rsb:701): actor 200 + 4i, archetype +1, package +3.
pub fn starting_equippable_guids(index: usize) -> RepResult<(u32, u32, u32)> {
    check(index < STARTING_EQUIPPABLES.len(), "starting equippable index out of range")?;
    let actor = STARTING_EQUIPPABLE_FIRST_GUID + 4 * index as u32;
    Ok((actor, actor + 1, actor + 3))
}

/// python build_starting_equippable_open (rsb:715): the default open of equippable i.
pub fn starting_equippable_open(index: usize) -> RepResult<Bits> {
    let (actor, archetype, package) = starting_equippable_guids(index)?;
    let item = STARTING_EQUIPPABLES[index];
    Ok(dynamic_actor_reference((item.archetype, item.package), actor, archetype, package, LEVEL_GUID, None)?)
}

/// python equippable_guid_for_class (rsb:803): the actor guid we replicated for a
/// live equippable class name.
pub fn equippable_guid_for_class(class_name: &str) -> Option<u32> {
    if class_name == "BasePistol_C" {
        return Some(BASE_PISTOL_GUID);
    }
    let index = STARTING_EQUIPPABLES.iter().position(|item| item.archetype.strip_prefix("Default__") == Some(class_name))?;
    Some(STARTING_EQUIPPABLE_FIRST_GUID + 4 * index as u32)
}
