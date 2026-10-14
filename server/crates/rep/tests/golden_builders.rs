//! actor opens, rep-layout payloads, rpc payloads and complete bodies against
//! server/fixtures/rep_builders.json (tools/golden/rep_builders.py). a case
//! with "error" is a python guard that raised; rust must fail there too.

use aries_bits::Bits;
use aries_rep::{RepResult, Spawn, actors, bodies, props, rpc, setup};
use serde_json::Value;

fn fixture() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/rep_builders.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("rep_builders.json")).expect("json")
}

fn cases<'a>(f: &'a Value, key: &str) -> &'a Vec<Value> {
    let list = f[key].as_array().unwrap_or_else(|| panic!("missing {key}"));
    assert!(!list.is_empty(), "{key} has no cases");
    list
}

fn check(result: RepResult<Bits>, case: &Value, label: &str) {
    if case.get("error").is_some() {
        assert!(result.is_err(), "{label}: python raised, rust gave {:?} for {case}", result.ok());
        return;
    }
    let bits = result.unwrap_or_else(|e| panic!("{label}: {e} for {case}"));
    assert_eq!(bits.bit_count as u64, case["bit_count"].as_u64().unwrap(), "{label} bits for {case}");
    assert_eq!(bits.hex(), case["hex"].as_str().unwrap(), "{label} for {case}");
}

fn u(v: &Value) -> u32 {
    v.as_u64().unwrap_or_else(|| panic!("not u32: {v}")) as u32
}

fn uv(v: &Value) -> Vec<u32> {
    v.as_array().unwrap().iter().map(u).collect()
}

fn guid(v: &Value) -> [u32; 4] {
    uv(v).try_into().unwrap()
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

fn float(v: &Value) -> f64 {
    match v {
        Value::String(text) => match text.as_str() {
            "nan" => f64::NAN,
            "inf" => f64::INFINITY,
            "-inf" => f64::NEG_INFINITY,
            other => panic!("bad float {other}"),
        },
        _ => v.as_f64().unwrap(),
    }
}

fn vec3(v: &Value) -> [f64; 3] {
    let list: Vec<f64> = v.as_array().unwrap().iter().map(float).collect();
    list.try_into().unwrap()
}

fn plain(case: &Value) -> Bits {
    Bits::from_hex(s(&case["plain"]), case["bits"].as_u64().unwrap() as usize).unwrap()
}

#[test]
fn actor_opens_match_python() {
    let f = fixture();
    for c in cases(&f, "static_reference") {
        let chain: Vec<&str> = c["chain"].as_array().unwrap().iter().map(s).collect();
        check(actors::exported_static_actor_reference(&chain, u(&c["actor"])), c, "static");
    }
    for c in cases(&f, "named_reference") {
        let result = match s(&c["name"]) {
            "world_settings" => actors::world_settings_reference(),
            "game_state" => actors::game_state_reference(),
            "controller" => actors::controller_reference(),
            "player_state" => actors::player_state_reference(),
            "player_info" => actors::player_info_reference(),
            "pawn_spawn" => actors::wushu_pawn_reference(Some(actors::PAWN_SPAWN_LOCATION)),
            "pawn_none" => actors::wushu_pawn_reference(None),
            "pawn_origin" => actors::wushu_pawn_reference(Some([0.0, -0.0, 0.04])),
            "base_pistol" => actors::base_pistol_reference(),
            other => panic!("unknown {other}"),
        };
        check(result, c, s(&c["name"]));
    }
    for c in cases(&f, "pawn_location") {
        check(actors::wushu_pawn_reference(Some(vec3(&c["location"]))), c, "pawn location");
    }
    for c in cases(&f, "base_team_state") {
        check(actors::base_team_state_reference(u(&c["actor"])), c, "team");
    }
    for c in cases(&f, "class_export") {
        check(actors::class_netguid_export(s(&c["class_name"]), s(&c["package"]), u(&c["class_guid"]), u(&c["package_guid"])), c, "class");
    }
    for c in cases(&f, "starting_open") {
        check(actors::starting_equippable_open(c["index"].as_u64().unwrap() as usize), c, "starting open");
    }
    for c in cases(&f, "starting_guids") {
        let result = actors::starting_equippable_guids(c["index"].as_u64().unwrap() as usize);
        match c.get("guids") {
            Some(g) => assert_eq!(result.unwrap(), (u(&g[0]), u(&g[1]), u(&g[2]))),
            None => assert!(result.is_err()),
        }
    }
    let table = cases(&f, "starting_table");
    assert_eq!(table.len(), actors::STARTING_EQUIPPABLES.len());
    for (c, item) in table.iter().zip(actors::STARTING_EQUIPPABLES) {
        assert_eq!((s(&c["archetype"]), s(&c["package"]), u(&c["slot"])), (item.archetype, item.package, item.slot));
    }
    for c in cases(&f, "guid_for_class") {
        assert_eq!(actors::equippable_guid_for_class(s(&c["class_name"])), c["guid"].as_u64().map(|g| g as u32), "{c}");
    }
    // the shared constants the range crate keys on
    assert_eq!(actors::GAME_STATE.reference(None).unwrap(), actors::game_state_reference().unwrap());
    assert_eq!(bodies::RANGE_GAME_MODE_CLASS.class_guid, 261);
}

#[test]
fn rep_payloads_match_python() {
    let f = fixture();
    for c in cases(&f, "desired_class") {
        check(props::desired_class(guid(&c["guid"]), u(&c["base"])), c, "desired class");
    }
    for c in cases(&f, "object_array") {
        check(props::object_array(u(&c["handle"]), &uv(&c["guids"])), c, "object array");
    }
    for c in cases(&f, "guid_array") {
        let guids: Vec<[u32; 4]> = c["guids"].as_array().unwrap().iter().map(guid).collect();
        check(props::guid_array(u(&c["handle"]), &guids), c, "guid array");
    }
    for c in cases(&f, "ordered") {
        let fields: Vec<(u32, Bits)> = c["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|field| (u(&field[0]), Bits::from_hex(s(&field[1]), field[2].as_u64().unwrap() as usize).unwrap()))
            .collect();
        check(props::ordered(&fields), c, "ordered");
    }
    for c in cases(&f, "phase") {
        check(props::phase(u(&c["value"])), c, "phase");
    }
    for c in cases(&f, "match_state") {
        check(props::match_state(s(&c["name"])), c, "match state");
    }
    for c in cases(&f, "unique_id") {
        check(props::player_unique_id(s(&c["subject"])), c, "unique id");
    }
    for c in cases(&f, "gameplay_start") {
        check(props::gameplay_start(float(&c["start_time"])), c, "gameplay start");
    }
    for c in cases(&f, "shop") {
        check(props::range_shop_settings(&uv(&c["handles"])), c, "shop");
    }
    for c in cases(&f, "pistol_role") {
        check(props::base_pistol_autonomous_role(u(&c["width"])), c, "pistol role");
    }
    for c in cases(&f, "pawn_role") {
        check(props::pawn_autonomous_role(u(&c["width"]), u(&c["value"])), c, "pawn role");
    }
    for c in cases(&f, "int_property") {
        check(props::int_property(u(&c["handle"]), u(&c["value"])), c, "int property");
    }
    for c in cases(&f, "bits_property") {
        check(props::bits_property(u(&c["handle"]), c["value"].as_u64().unwrap(), u(&c["width"])), c, "bits property");
    }
    for c in cases(&f, "null_object") {
        check(props::null_object_property(u(&c["handle"])), c, "null object");
    }
    assert_eq!(props::RANGE_SHOP_SETTING_HANDLES, [44, 48, 49]);
}

#[test]
fn rpc_payloads_match_python() {
    let f = fixture();
    for c in cases(&f, "client_restart") {
        check(rpc::client_restart(), c, "client restart");
    }
    for c in cases(&f, "set_hud") {
        check(rpc::client_set_hud(u(&c["field"]), u(&c["max"]), u(&c["guid"])), c, "set hud");
    }
    for c in cases(&f, "input_ack") {
        check(rpc::input_ack(u(&c["latest"]), u(&c["index"]), u(&c["field"]), u(&c["max"])), c, "input ack");
    }
    for c in cases(&f, "packed_bits") {
        check(rpc::packed_bits_struct(&plain(c)), c, "packed bits");
    }
    for c in cases(&f, "streaming") {
        let names: Vec<&str> = c["names"].as_array().unwrap().iter().map(s).collect();
        let result = rpc::levels_streaming_status(&names, u(&c["field"]), u(&c["max"]), c["loaded"].as_bool().unwrap(), c["visible"].as_bool().unwrap());
        check(result, c, "streaming");
    }
    for c in cases(&f, "flush") {
        check(rpc::flush_level_streaming(u(&c["field"]), u(&c["max"])), c, "flush");
    }
    for c in cases(&f, "range_settings") {
        check(rpc::update_range_settings(u(&c["field"]), u(&c["max"])), c, "range settings");
    }
    for c in cases(&f, "correction_container") {
        check(rpc::correction_container(u(&c["move"]), vec3(&c["location"]), vec3(&c["velocity"])), c, "correction container");
    }
    for c in cases(&f, "correction_rpc") {
        let result = rpc::correction_rpc(u(&c["move"]), vec3(&c["location"]), u(&c["field"]), u(&c["max"]), vec3(&c["velocity"]));
        check(result, c, "correction rpc");
    }
    for c in cases(&f, "controller_rpc_body") {
        check(rpc::controller_rpc_body(&plain(c)), c, "controller rpc body");
    }
    let sublevels: Vec<&str> = rpc::RANGE_GAMEPLAY_SUBLEVELS.to_vec();
    assert_eq!(sublevels, cases(&f, "streaming")[0]["names"].as_array().unwrap().iter().map(s).collect::<Vec<_>>());
}

#[test]
fn complete_bodies_match_python() {
    let f = fixture();
    for c in cases(&f, "actor_int_body") {
        let value = i32::try_from(c["value"].as_i64().unwrap()).unwrap();
        check(bodies::actor_int_property_body(u(&c["actor"]), u(&c["handle"]), value), c, "int body");
    }
    for c in cases(&f, "game_state_bool_body") {
        check(bodies::game_state_bool_body(u(&c["handle"]), c["value"].as_bool().unwrap()), c, "bool body");
    }
    for c in cases(&f, "class_property_body") {
        let result = bodies::actor_class_property_body(u(&c["actor"]), s(&c["class_name"]), s(&c["package"]), u(&c["class_guid"]), u(&c["package_guid"]), u(&c["handle"]));
        check(result, c, "class property body");
    }
    for c in cases(&f, "money_body") {
        let money = i32::try_from(c["money"].as_i64().unwrap()).unwrap();
        check(bodies::money_component_body(money, u(&c["component"])), c, "money body");
    }
    for c in cases(&f, "team_component") {
        let role = c["role"].as_u64().map(|r| r as u32);
        check(bodies::team_state_component_update(u(&c["actor"]), u(&c["component"]), s(&c["name"]), role), c, "team component");
    }
    for c in cases(&f, "state_link") {
        check(bodies::equippable_state_link_body(s(&c["name"]), u(&c["guid"])), c, "state link");
    }
    // the spawn location the pawn open uses (hs:2357-2360) feeds Spawn with no yaw
    let spawn = Spawn { location: actors::PAWN_SPAWN_LOCATION, yaw: 0.0 };
    assert_eq!(actors::WUSHU_PAWN.reference(Some(spawn)).unwrap(), actors::wushu_pawn_reference(Some(actors::PAWN_SPAWN_LOCATION)).unwrap());
}

fn class_property(v: &Value) -> (String, String, u32, u32, u32) {
    (s(&v[0]).to_owned(), s(&v[1]).to_owned(), u(&v[2]), u(&v[3]), u(&v[4]))
}

#[test]
fn constants_match_python_modules() {
    let f = fixture();
    let k = &f["constants"];
    assert_eq!(uv(&k["range_shop_setting_handles"]), props::RANGE_SHOP_SETTING_HANDLES);
    assert_eq!(u(&k["starting_equippable_first_channel"]), actors::STARTING_EQUIPPABLE_FIRST_CHANNEL);
    assert_eq!(u(&k["starting_equippable_first_guid"]), actors::STARTING_EQUIPPABLE_FIRST_GUID);
    assert_eq!(u(&k["inventory_net_guid"]), bodies::INVENTORY_GUID);
    assert_eq!(u(&k["item_slot_class_net_guid"]), bodies::ITEM_SLOT_CLASS_GUID);
    assert_eq!(u(&k["first_item_slot_net_guid"]), bodies::FIRST_ITEM_SLOT_GUID);
    assert_eq!(u(&k["actor_owner_handle"]), bodies::ACTOR_OWNER_HANDLE);
    assert_eq!(u(&k["item_slot_contents_handle"]), bodies::ITEM_SLOT_CONTENTS_HANDLE);
    assert_eq!(u(&k["base_pistol_actor_guid"]), actors::BASE_PISTOL_GUID);
    assert_eq!(u(&k["base_pistol_state_machine_guid"]), bodies::BASE_PISTOL_STATE_MACHINE_GUID);
    assert_eq!(u(&k["base_pistol_inactive_state_guid"]), bodies::BASE_PISTOL_INACTIVE_STATE_GUID);
    assert_eq!(u(&k["auth_current_state_handle"]), bodies::AUTH_CURRENT_STATE_HANDLE);
    assert_eq!(u(&k["player_state_ultimate_points_handle"]), bodies::PLAYER_STATE_ULTIMATE_POINTS_HANDLE);
    for (key, rust) in [("range_game_mode_class", bodies::RANGE_GAME_MODE_CLASS), ("range_hud_config_class", bodies::RANGE_HUD_CONFIG_CLASS)] {
        let rust = (rust.class_name.to_owned(), rust.package.to_owned(), rust.class_guid, rust.package_guid, rust.handle);
        assert_eq!(class_property(&k[key]), rust, "{key}");
    }
    let sublevels: Vec<&str> = k["range_gameplay_sublevels"].as_array().unwrap().iter().map(s).collect();
    assert_eq!(sublevels, rpc::RANGE_GAMEPLAY_SUBLEVELS);
    assert_eq!(u(&k["client_update_multiple_levels_streaming_status_field"]), rpc::LEVELS_STREAMING_STATUS_FIELD);
    assert_eq!(u(&k["client_flush_level_streaming_field"]), rpc::FLUSH_LEVEL_STREAMING_FIELD);
    assert_eq!(u(&k["shooting_range_controller_class_max"]), rpc::CONTROLLER_CLASS_MAX);
    assert_eq!(u(&k["client_update_range_settings_field"]), rpc::UPDATE_RANGE_SETTINGS_FIELD);
    assert_eq!(u(&k["game_info_infinite_ammo_handle"]), bodies::GAME_INFO_INFINITE_AMMO_HANDLE);
    assert_eq!(u(&k["money_component_guid"]), bodies::MONEY_COMPONENT_GUID);
    assert_eq!(u(&k["money_handle"]), bodies::MONEY_HANDLE);
    assert_eq!(k["range_money"].as_i64().unwrap(), bodies::RANGE_MONEY as i64);
    assert_eq!(u(&k["melee_channel"]), actors::STARTING_EQUIPPABLE_FIRST_CHANNEL + 1);
    assert_eq!(u(&k["melee_actor_guid"]), setup::KNIFE_GUID);
    assert_eq!(u(&k["melee_state_machine_guid"]), setup::KNIFE_STATE_MACHINE_GUID);
    assert_eq!(u(&k["melee_inactive_state_guid"]), setup::KNIFE_INACTIVE_STATE_GUID);
    assert_eq!(u(&k["melee_auth_current_state_handle"]), bodies::AUTH_CURRENT_STATE_HANDLE);
}
