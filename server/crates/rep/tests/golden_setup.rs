//! the per-stage bring-up bodies against server/fixtures/rep_setup.json, which
//! tools/golden/rep_setup.py makes by re-running run_probe's inline statements.

use aries_bits::Bits;
use aries_rep::{RepResult, bodies, props, setup};
use serde_json::Value;

fn fixture() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/rep_setup.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("rep_setup.json")).expect("json")
}

fn cases<'a>(f: &'a Value, key: &str) -> &'a Vec<Value> {
    let list = f[key].as_array().unwrap_or_else(|| panic!("missing {key}"));
    assert!(!list.is_empty(), "{key} has no cases");
    list
}

fn check(result: RepResult<Bits>, case: &Value, label: &str) {
    let bits = result.unwrap_or_else(|e| panic!("{label}: {e} for {case}"));
    assert_eq!(bits.bit_count as u64, case["bit_count"].as_u64().unwrap(), "{label} bits for {case}");
    assert_eq!(bits.hex(), case["hex"].as_str().unwrap(), "{label} for {case}");
}

/// a stage with exactly one production body.
fn single(f: &Value, key: &str, result: RepResult<Bits>) {
    let list = cases(f, key);
    assert_eq!(list.len(), 1, "{key}");
    check(result, &list[0], key);
}

fn u(v: &Value) -> u32 {
    v.as_u64().unwrap_or_else(|| panic!("not u32: {v}")) as u32
}

fn idx(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}

fn guid(v: &Value) -> [u32; 4] {
    let list: Vec<u32> = v.as_array().unwrap().iter().map(u).collect();
    list.try_into().unwrap()
}

fn vec3(v: &Value) -> [f64; 3] {
    let list: Vec<f64> = v.as_array().unwrap().iter().map(|c| c.as_f64().unwrap()).collect();
    list.try_into().unwrap()
}

#[test]
fn part_a_bodies_match_python() {
    let f = fixture();
    single(&f, "a01_world_settings_open", setup::world_settings_open());
    single(&f, "a02_game_state_open", setup::game_state_open());
    single(&f, "a03_controller_open", setup::controller_open());
    single(&f, "a04_player_state_open", setup::player_state_open());
    single(&f, "a05_player_info_open", setup::player_info_open());
    single(&f, "a06_pawn_open", setup::pawn_open());
    for c in cases(&f, "a07_game_state_bool") {
        check(setup::game_state_bool_h18_body(c["value"].as_bool().unwrap()), c, "a07");
    }
    single(&f, "a08_controller_player_state", setup::controller_player_state_link());
    single(&f, "a09_controller_pawn", setup::controller_pawn_link());
    single(&f, "a10_player_state_player_info", setup::player_state_player_info_link());
    single(&f, "a11_spawned_character", setup::player_state_spawned_character_link());
    single(&f, "a11_possessed_character", setup::player_state_possessed_character_link());
    for c in cases(&f, "a12_desired_class") {
        check(setup::player_state_desired_class_body(guid(&c["guid"])), c, "a12");
    }
    assert_eq!(guid(&cases(&f, "a12_desired_class")[0]["guid"]), props::WUSHU_CLASS_GUID);
    single(&f, "a13_player_info_controller", setup::player_info_controller_link());
    single(&f, "a14_pawn_player_state", setup::pawn_player_state_link());
    single(&f, "a15_pawn_controller", setup::pawn_controller_link());
    single(&f, "a16_pawn_role", setup::pawn_remote_role_body());
    single(&f, "a17_inventory_open", setup::inventory_open_body());
    let creates = cases(&f, "a18_a20_item_slot_create");
    let links = cases(&f, "a19_a20_item_slot_link");
    assert_eq!((creates.len(), links.len()), (16, 16));
    for c in creates {
        check(setup::item_slot_create_body(u(&c["slot"])), c, "item slot create");
    }
    for c in links {
        check(setup::item_slot_link_body(u(&c["slot"])), c, "item slot link");
    }
    assert!(setup::item_slot_create_body(16).is_err() && setup::item_slot_link_body(16).is_err());
    single(&f, "a21_base_pistol_open", setup::base_pistol_open_body());
    single(&f, "a22_pistol_slot_contents", setup::pistol_slot_contents_body());
    single(&f, "a23_pistol_owner", setup::pistol_owner_body());
    single(&f, "a24_pistol_role", setup::pistol_remote_role_body());
    single(&f, "a25_pistol_state_machine_open", setup::pistol_state_machine_open_body());
    single(&f, "a26_inactive_state_link", setup::pistol_inactive_state_body());
    for c in cases(&f, "a27_auth_correct") {
        check(setup::pistol_correction_body(u(&c["index"])), c, "a27");
    }
}

#[test]
fn part_b_bodies_match_python() {
    let f = fixture();
    single(&f, "b1_client_restart", setup::client_restart_body());
    for c in cases(&f, "b2_game_state_phase") {
        check(setup::game_state_phase_body(u(&c["phase"])), c, "b2");
    }
    for c in cases(&f, "b3_team_state_open") {
        check(setup::team_state_open_body(idx(&c["team"])), c, "b3");
    }
    single(&f, "b4_team_state_array", setup::team_state_array_body());
    for c in cases(&f, "b5_team_state_link") {
        check(setup::team_state_link_body(idx(&c["team"])), c, "b5");
    }
    assert!(setup::team_state_open_body(2).is_err() && setup::team_state_link_body(2).is_err());
    single(&f, "b6_player_state_team", setup::player_state_team_body());
    single(&f, "b7_bomb_team_open", setup::bomb_team_open_body());
    for c in cases(&f, "b8_bomb_team_number") {
        check(setup::bomb_team_number_body(u(&c["value"]), u(&c["bits"])), c, "b8");
    }
    for c in cases(&f, "b9_accessible_character") {
        check(setup::accessible_character_list_body(guid(&c["guid"])), c, "b9");
    }
    for c in cases(&f, "b10_unique_id") {
        check(setup::player_unique_id_body(c["subject"].as_str().unwrap()), c, "b10");
    }
    assert_eq!(cases(&f, "b10_unique_id")[0]["subject"].as_str().unwrap(), props::OFFLINE_PLAYER_ID);
    single(&f, "b11_money", setup::range_money_body());
    single(&f, "b12_pawn_owner", setup::pawn_owner_body());
    for c in cases(&f, "b13_hud_set") {
        check(setup::hud_set_body(u(&c["field"]), u(&c["max"])), c, "b13");
    }
    single(&f, "b14_match_state", setup::match_state_body());
    single(&f, "b15_gameplay_start", setup::gameplay_start_body());
    single(&f, "b16_player_array", setup::player_array_body());
    single(&f, "b17_range_shop", setup::range_shop_body());
    let equippables = cases(&f, "b18_starting_equippable");
    assert_eq!(equippables.len(), 7);
    for c in equippables {
        let i = idx(&c["index"]);
        check(setup::starting_equippable_open_body(i), &c["open"], "b18 open");
        check(setup::starting_equippable_owner_body(i), &c["owner"], "b18 owner");
        check(setup::starting_equippable_contents_body(i), &c["contents"], "b18 contents");
    }
    assert!(setup::starting_equippable_owner_body(7).is_err() && setup::starting_equippable_contents_body(7).is_err());
    single(&f, "b19_knife_role", setup::knife_remote_role_body());
    single(&f, "b20_knife_state_machine", setup::knife_state_machine_open_body());
    single(&f, "b21_knife_inactive_state", setup::knife_inactive_state_body());
    single(&f, "b22_viewtarget", setup::viewtarget_body());
}

#[test]
fn move_and_input_acks_match_python() {
    let f = fixture();
    for c in cases(&f, "m_correction") {
        check(setup::placement_correction_body(u(&c["move"]), vec3(&c["location"])), c, "correction");
    }
    assert_eq!(vec3(&cases(&f, "m_correction")[0]["location"]), setup::MOVE_CORRECTION_LOCATION);
    for c in cases(&f, "m_move_ack") {
        check(setup::move_ack_body(u(&c["move"])), c, "move ack");
    }
    for c in cases(&f, "i_input_ack") {
        check(setup::input_ack_body(u(&c["event"])), c, "input ack");
    }
}

#[test]
fn per_packet_bodies_match_python() {
    let f = fixture();
    for c in cases(&f, "c_range_class_property") {
        let property = match c["which"].as_str().unwrap() {
            "game_mode" => bodies::RANGE_GAME_MODE_CLASS,
            "hud_config" => bodies::RANGE_HUD_CONFIG_CLASS,
            other => panic!("unknown {other}"),
        };
        check(setup::range_class_property_body(&property), c, "class property");
    }
    for c in cases(&f, "c_hud_rebuild") {
        check(setup::hud_rebuild_body(u(&c["field"]), u(&c["max"])), c, "hud rebuild");
    }
    single(&f, "c_levels_streaming", setup::levels_streaming_body());
    single(&f, "c_flush_level_streaming", setup::flush_level_streaming_body());
    for c in cases(&f, "c_ultimate_points") {
        check(setup::ultimate_points_body(u(&c["points"]) as i32), c, "ultimate points");
    }
    single(&f, "c_range_settings_request", setup::range_settings_request_body());
    for c in cases(&f, "c_infinite_ammo") {
        check(setup::infinite_ammo_body(c["enabled"].as_bool().unwrap()), c, "infinite ammo");
    }
    single(&f, "c_hud_refresh_null", setup::spawned_character_null_body());
    single(&f, "c_hud_refresh_pawn", setup::player_state_spawned_character_link());
}
