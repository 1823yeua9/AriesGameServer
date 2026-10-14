//! team-state and BombTeam bodies (spec 08 section 6, V.5) against
//! server/fixtures/world_teams.json. the BombTeam bodies were asserted equal to
//! the inline production statements (ruh:3548-3568, 3601-3615) by the exporter.

mod golden_ab_common;

use aries_world::teams::{
    BOMB_TEAM_GUID, BOMB_TEAM_NUMBER_HANDLE, PLAYER_TEAM_NUMBER, TEAM_NUMBER_BITS, base_team_state_open, bomb_team_number_body, bomb_team_open_body,
    object_array_property, player_state_team_body, team_state_component_update, team_state_link_body, team_states_array_body,
};
use golden_ab_common::{check, fixture, python_error, u32_of};

#[test]
fn team_state_bodies_match_python() {
    let f = fixture("world_teams");
    for case in f["team_state_open"].as_array().unwrap() {
        let guid = u32_of(&case["guid"]);
        check(base_team_state_open(guid), case, &format!("team state open {guid}"));
    }
    check(team_states_array_body(), &f["team_states_array"], "team states array");
    for (k, case) in f["object_arrays"].as_array().unwrap().iter().enumerate() {
        let guids: Vec<u32> = case["guids"].as_array().unwrap().iter().map(u32_of).collect();
        check(object_array_property(u32_of(&case["handle"]), &guids), case, &format!("object array {k}"));
    }
    for (k, case) in f["team_links"].as_array().unwrap().iter().enumerate() {
        let role = case["role"].as_u64().map(|role| role as u32);
        let result = team_state_component_update(u32_of(&case["actor"]), u32_of(&case["component"]), case["name"].as_str().unwrap(), role);
        check(result, case, &format!("team link {k}"));
    }
    // the production links are the first two cases (ruh:3480-3484)
    check(team_state_link_body(0), &f["team_links"][0], "blue link");
    check(team_state_link_body(1), &f["team_links"][1], "red link");
    assert!(team_state_link_body(2).is_err());
    check(player_state_team_body(), &f["player_state_team"], "player state team");
}

#[test]
fn bomb_team_bodies_match_python() {
    let f = fixture("world_teams");
    check(bomb_team_open_body(BOMB_TEAM_GUID), &f["bomb_team_open"], "bomb team open (production)");
    for case in f["bomb_team_opens"].as_array().unwrap() {
        let subobj = u32_of(&case["subobj"]);
        check(bomb_team_open_body(subobj), case, &format!("bomb team open {subobj}"));
    }
    // inline production statements at 8 bits (value 1 is production)
    for case in f["bomb_team_number_inline"].as_array().unwrap() {
        assert_eq!(case["bits"], 8);
        let value = u8::try_from(case["value"].as_u64().unwrap()).unwrap();
        check(bomb_team_number_body(BOMB_TEAM_GUID, value, BOMB_TEAM_NUMBER_HANDLE), case, &format!("inline number {value}"));
    }
    // the production value/width come from ruh's argparse defaults
    let production = &f["bomb_team_number_production"];
    assert_eq!(production["value"], PLAYER_TEAM_NUMBER as u64);
    assert_eq!(production["bits"], TEAM_NUMBER_BITS as u64);
    assert_eq!(f["bomb_team_number_inline"][0]["value"], PLAYER_TEAM_NUMBER as u64);
    for case in f["bomb_team_number"].as_array().unwrap() {
        if python_error(case).is_some() || case.get("subobj").is_none() {
            // -1 and 256 have no u8 form; the type is the guard
            let team = case["team"].as_i64().unwrap();
            assert!(u8::try_from(team).is_err() == python_error(case).is_some(), "team {team}");
            continue;
        }
        let team = u8::try_from(case["team"].as_u64().unwrap()).unwrap();
        let result = bomb_team_number_body(u32_of(&case["subobj"]), team, u32_of(&case["handle"]));
        check(result, case, &format!("number {team}"));
    }
}
