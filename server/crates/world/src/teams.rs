//! team-state and BombTeam bodies of bring-up stages B3-B8 (spec 08 section 6,
//! scheduling in 04_bringup_b.md): the two BaseTeamState actors, GameState
//! TeamStates, the BlueTeam/RedTeam component links, PlayerState's team, and the
//! PlayerState BombTeam subobject open and TeamNumber (range_bomb_team_open.py,
//! byte-identical to the inline production bodies).
//!
//! rsb = range_server_bunch.py, rbt = range_bomb_team_open.py, ruh = range_udp_handshake.py.

use aries_bits::{BitWriter, Bits};
use aries_rep::block::{Target, append_export_header, append_rep_block, append_stable_child_entry, dynamic_actor_reference};
use aries_rep::props::{empty_rep, object_property};
use aries_rep::{RepError, RepResult};

/// rsb:206-218: the two team actors, their shared archetype/package, and the class path.
pub const TEAM_STATE_GUIDS: [u32; 2] = [100, 102];
pub const TEAM_STATE_ARCHETYPE_GUID: u32 = 101;
pub const TEAM_STATE_PACKAGE_GUID: u32 = 103;
pub const BASE_TEAM_STATE: (&str, &str) = ("Default__BaseTeamState", "/Script/ShooterGame");
/// ruh:3482-3484, rsb:1362-1365: GameState components linked to each team, and their roles.
pub const TEAM_COMPONENT_GUIDS: [u32; 2] = [104, 106];
pub const TEAM_COMPONENT_NAMES: [&str; 2] = ["BlueTeam", "RedTeam"];
pub const TEAM_ROLES: [u32; 2] = [1, 2];
/// rsb:1355-1356: BaseTeamState TeamComponent, TeamInfo.InitialRole, TeamInfo.TeamRole.
pub const TEAM_COMPONENT_HANDLE: u32 = 16;
pub const INITIAL_ROLE_HANDLE: u32 = 17;
pub const TEAM_ROLE_HANDLE: u32 = 18;
/// rsb:1371-1374: EAresTeamRole (MAX 6) serializes in 3 bits.
pub const TEAM_ROLE_BITS: u32 = 3;
/// ruh:3449-3452: GameState (guid 2) TeamStates array handle.
pub const GAME_STATE_GUID: u32 = 2;
pub const GAME_STATE_TEAM_STATES_HANDLE: u32 = 34;
/// ruh:3512-3514: PlayerState (guid 6) team handle -> blue team 100.
pub const PLAYER_STATE_GUID: u32 = 6;
pub const PLAYER_STATE_TEAM_HANDLE: u32 = 27;
/// rbt:22, ruh:3548: PlayerState "BombTeam" (UNumberedTeamComponent) subobject.
pub const BOMB_TEAM_GUID: u32 = 108;
pub const BOMB_TEAM_NAME: &str = "BombTeam";
/// ruh:3603 (verified live: UNumberedTeamComponent parent 2 at +0xE8): TeamNumber.
pub const BOMB_TEAM_NUMBER_HANDLE: u32 = 3;
/// ruh:4670-4673 argparse defaults, never passed by the launchers:
/// --bomb-team-number-value 1 (BlueTeam) in --bomb-team-number-bits 8.
pub const PLAYER_TEAM_NUMBER: u8 = 1;
pub const TEAM_NUMBER_BITS: u32 = 8;
/// rsb:143: the Range level.
const LEVEL_GUID: u32 = 5;

fn invalid(message: &str) -> RepError {
    RepError::Invalid(message.into())
}

/// a rep block on the channel actor from a plain payload (python
/// append_actor_rep_layout_payload of an encoded payload, seed = the actor).
fn actor_rep_body(plain: &Bits, actor_guid: u32) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    append_rep_block(&mut w, Target::Actor, plain, actor_guid)?;
    Ok(w.finish()?)
}

/// rsb:206-218, B3 (ch10/ch11): open a native BaseTeamState, guid 100 or 102 only.
pub fn base_team_state_open(actor_guid: u32) -> RepResult<Bits> {
    if !TEAM_STATE_GUIDS.contains(&actor_guid) {
        return Err(invalid("team actor GUID is not reserved for this session"));
    }
    Ok(dynamic_actor_reference(BASE_TEAM_STATE, actor_guid, TEAM_STATE_ARCHETYPE_GUID, TEAM_STATE_PACKAGE_GUID, LEVEL_GUID, None)?)
}

/// rsb:1300-1322: an ordinary TArray<UObject*> rep payload: [0][handle][16: n]
/// ([child i+1][guid])... [0][0].
pub fn object_array_property(handle: u32, net_guids: &[u32]) -> RepResult<Bits> {
    if !(1..=0xFFFF).contains(&handle) || net_guids.len() > 256 {
        return Err(invalid("array handle or element count outside supported range"));
    }
    if net_guids.iter().any(|guid| !(1..=0xFFFF).contains(guid)) {
        return Err(invalid("array object GUID outside supported range"));
    }
    let mut w = BitWriter::new();
    w.append_bits(0, 1);
    w.append_packed_uint(handle as u64);
    w.append_bits(net_guids.len() as u64, 16);
    for (child, guid) in net_guids.iter().enumerate() {
        w.append_packed_uint(child as u64 + 1);
        w.append_packed_uint(*guid as u64);
    }
    w.append_packed_uint(0);
    w.append_packed_uint(0);
    Ok(w.finish()?)
}

/// ruh:3449-3454, B4 (ch4): GameState.TeamStates = [100, 102], seed 2.
pub fn team_states_array_body() -> RepResult<Bits> {
    actor_rep_body(&object_array_property(GAME_STATE_TEAM_STATES_HANDLE, &TEAM_STATE_GUIDS)?, GAME_STATE_GUID)
}

/// rsb:1351-1392, B5 (ch10 + i): export the GameState's team component (outer
/// GameState 2), then TeamComponent and, with a role, InitialRole/TeamRole on
/// the team actor.
pub fn team_state_component_update(actor_guid: u32, component_guid: u32, component_name: &str, role: Option<u32>) -> RepResult<Bits> {
    if !TEAM_STATE_GUIDS.contains(&actor_guid) || !TEAM_COMPONENT_GUIDS.contains(&component_guid) {
        return Err(invalid("team or component GUID is not reserved"));
    }
    if !TEAM_COMPONENT_NAMES.contains(&component_name) || !matches!(role, None | Some(1 | 2)) {
        return Err(invalid("team component name or role outside verified Range set"));
    }
    // rsb:1395-1416 ordered payload: [0] then ascending (handle, value) and [0]
    let mut rep = BitWriter::new();
    rep.append_bits(0, 1);
    rep.append_packed_uint(TEAM_COMPONENT_HANDLE as u64);
    rep.append_packed_uint(component_guid as u64);
    if let Some(role) = role {
        rep.append_packed_uint(INITIAL_ROLE_HANDLE as u64);
        rep.append_bits(role as u64, TEAM_ROLE_BITS);
        rep.append_packed_uint(TEAM_ROLE_HANDLE as u64);
        rep.append_bits(role as u64, TEAM_ROLE_BITS);
    }
    rep.append_packed_uint(0);
    let mut w = BitWriter::new();
    append_export_header(&mut w, 1);
    append_stable_child_entry(&mut w, component_guid, GAME_STATE_GUID, component_name);
    append_rep_block(&mut w, Target::Actor, &rep.finish()?, actor_guid)?;
    Ok(w.finish()?)
}

/// B5 for team `index` (0 blue, 1 red) with the production arguments (ruh:3480-3484).
pub fn team_state_link_body(index: usize) -> RepResult<Bits> {
    if index >= TEAM_STATE_GUIDS.len() {
        return Err(invalid("team index out of range"));
    }
    team_state_component_update(TEAM_STATE_GUIDS[index], TEAM_COMPONENT_GUIDS[index], TEAM_COMPONENT_NAMES[index], Some(TEAM_ROLES[index]))
}

/// ruh:3512-3516, B6 (ch6): PlayerState team = the blue team actor 100, seed 6.
pub fn player_state_team_body() -> RepResult<Bits> {
    actor_rep_body(&object_property(PLAYER_STATE_TEAM_HANDLE, TEAM_STATE_GUIDS[0])?, PLAYER_STATE_GUID)
}

/// rbt:42-64 (= ruh:3548-3568), B7 (ch6, exports): introduce the stably-named
/// BombTeam subobject (outer PlayerState 6, cached) with an empty layout so the
/// client allocates its replicator. production: BOMB_TEAM_GUID.
pub fn bomb_team_open_body(subobj_guid: u32) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    append_export_header(&mut w, 1);
    append_stable_child_entry(&mut w, subobj_guid, PLAYER_STATE_GUID, BOMB_TEAM_NAME);
    append_rep_block(&mut w, Target::Subobject(subobj_guid), &empty_rep(), PLAYER_STATE_GUID)?;
    Ok(w.finish()?)
}

/// rbt:101-119 (= ruh:3601-3615 at 8 bits), B8 (ch6): BombTeam.TeamNumber (uint8
/// EAresTeam: BlueTeam 1, RedTeam 0, Invalid 0xFE). production: (108, 1, 3).
pub fn bomb_team_number_body(subobj_guid: u32, team_number: u8, wire_handle: u32) -> RepResult<Bits> {
    let mut rep = BitWriter::new();
    rep.append_bits(0, 1);
    rep.append_packed_uint(wire_handle as u64);
    rep.append_bits(team_number as u64, TEAM_NUMBER_BITS);
    rep.append_packed_uint(0);
    let mut w = BitWriter::new();
    append_rep_block(&mut w, Target::Subobject(subobj_guid), &rep.finish()?, PLAYER_STATE_GUID)?;
    Ok(w.finish()?)
}
