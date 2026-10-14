//! guns and shop against the python (server/tools/golden/world_guns.py):
//! store helpers, the purchase allocator, equippable / melee builders,
//! attachments and the gun loader. fixtures: server/fixtures/guns_*.json.

use std::collections::HashMap;

use aries_bits::{BitWriter, Bits};
use aries_rep::{RepError, RepResult};
use aries_world::attachments::{self, AttachmentAsset};
use aries_world::equippables::{self, EquippableActor, ExtraStateMachine, ResourceSubobject, StateGuids};
use aries_world::loader::{self, GunLoad, ItemMachine, PurchaseAllocator};
use aries_world::melee;
use aries_world::skins::{CosmeticCdoPath, CosmeticKind, EquippedCdoPaths, SkinError};
use aries_world::store;
use serde_json::{Value, json};

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).unwrap()
}

fn u(value: &Value) -> u32 {
    value.as_u64().unwrap_or_else(|| panic!("not a u32: {value}")) as u32
}

fn s(value: &Value) -> &str {
    value.as_str().unwrap_or_else(|| panic!("not a str: {value}"))
}

/// a builder result against {hex, bit_count} or {error, message}. our explicit
/// checks carry python's message; bit-writer failures only need python to fail too.
fn check(result: RepResult<Bits>, case: &Value, label: &str) {
    match case.get("error") {
        Some(kind) => {
            let error = result.expect_err(label);
            match &error {
                RepError::Invalid(message) => assert_eq!(message, s(&case["message"]), "{label}"),
                // bit-writer failures: python raises ValueError (or its UnicodeEncodeError subclass)
                _ => assert!(matches!(s(kind), "ValueError" | "UnicodeEncodeError"), "{label}: {error}"),
            }
        }
        None => {
            let bits = result.unwrap_or_else(|e| panic!("{label}: {e}"));
            assert_eq!(bits.bit_count as u64, case["bit_count"].as_u64().unwrap(), "{label} bits");
            assert_eq!(bits.hex(), s(&case["hex"]), "{label}");
        }
    }
}

fn state_guids(pairs: &Value) -> StateGuids {
    pairs.as_array().unwrap().iter().map(|p| (s(&p[0]).to_string(), u(&p[1]))).collect()
}

fn state_guids_value(guids: &StateGuids) -> Value {
    guids.iter().map(|(name, guid)| json!([name, guid])).collect()
}

/// python eq_dict, from a rust equippable.
fn eq_value(eq: &EquippableActor) -> Value {
    json!({
        "key": eq.key, "class": eq.class_name, "channel": eq.channel, "actor": eq.actor_guid, "slot": eq.slot,
        "state_machine": eq.state_machine_guid, "states": state_guids_value(&eq.state_guids),
        "resources": eq.resources.iter().map(|r| json!([r.name, r.guid, r.item_offset, r.default_offset, r.refill_after])).collect::<Vec<_>>(),
        "extra": eq.extra_machines.iter().map(|m| json!({"name": m.name, "guid": m.guid, "starting": m.starting_state, "states": state_guids_value(&m.state_guids)})).collect::<Vec<_>>(),
        "mirror_states": eq.mirror_states,
    })
}

/// an equippable from a python eq_dict, as written (no validation).
fn eq_from(d: &Value) -> EquippableActor {
    EquippableActor {
        key: s(&d["key"]).into(),
        class_name: s(&d["class"]).into(),
        channel: u(&d["channel"]),
        actor_guid: u(&d["actor"]),
        slot: u(&d["slot"]),
        state_machine_guid: u(&d["state_machine"]),
        state_guids: state_guids(&d["states"]),
        resources: d["resources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| ResourceSubobject { name: s(&r[0]).into(), guid: u(&r[1]), item_offset: u(&r[2]), default_offset: u(&r[3]), refill_after: r[4].as_f64() })
            .collect(),
        extra_machines: d["extra"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| ExtraStateMachine { name: s(&m["name"]).into(), guid: u(&m["guid"]), starting_state: s(&m["starting"]).into(), state_guids: state_guids(&m["states"]) })
            .collect(),
        mirror_states: d["mirror_states"].as_bool().unwrap(),
    }
}

fn load_value(load: &GunLoad) -> Value {
    json!({
        "class": load.item.class_name, "package": load.package, "archetype": load.archetype, "channel": load.channel,
        "actor": load.actor_guid, "archetype_guid": load.archetype_guid, "package_guid": load.package_guid,
        "key": load.key(), "state_machine": load.state_machine_guid, "magazine": load.magazine_guid, "reserve": load.reserve_guid,
        "cosmetic_pairs": load.cosmetic_guid_pairs.iter().map(|&(a, b)| json!([a, b])).collect::<Vec<_>>(),
        "attachment_pairs": load.attachment_guid_pairs.iter().map(|&(a, b)| json!([a, b])).collect::<Vec<_>>(),
        "state_guids": state_guids_value(&load.state_guids),
    })
}

fn item_load_value(load: &loader::ItemLoad) -> Value {
    json!({
        "class": load.item.class_name, "package": load.package, "archetype": load.archetype, "channel": load.channel,
        "actor": load.actor_guid, "archetype_guid": load.archetype_guid, "package_guid": load.package_guid,
    })
}

fn cdo_path(d: &Value) -> CosmeticCdoPath {
    CosmeticCdoPath {
        kind: CosmeticKind::from_name(s(&d["kind"])).unwrap(),
        asset_id: s(&d["asset_id"]).into(),
        source_asset: s(&d["source_asset"]).into(),
        package_path: s(&d["package_path"]).into(),
        object_name: s(&d["object_name"]).into(),
        packaged: d["packaged"].as_bool().unwrap(),
    }
}

fn cdo_paths(d: &Value) -> Option<EquippedCdoPaths> {
    (!d.is_null()).then(|| EquippedCdoPaths { weapon_id: s(&d["weapon_id"]).into(), skin: cdo_path(&d["skin"]), level: cdo_path(&d["level"]), chroma: cdo_path(&d["chroma"]) })
}

/// a recorded python selection ({value} or {error}) as the rust selection result.
fn selection(d: &Value) -> Result<Option<EquippedCdoPaths>, SkinError> {
    match d.get("value") {
        Some(value) => Ok(cdo_paths(value)),
        None => Err(SkinError::Inventory(s(&d["message"]).into())),
    }
}

#[test]
fn store_matches_python() {
    let f = fixture("guns_store");
    assert_eq!(f["slots"], json!([store::SLOT_PRIMARY, store::SLOT_SECONDARY, store::SLOT_ARMOR]));
    let items: Vec<Value> = store::STORE_ITEMS
        .iter()
        .map(|i| json!({"display": i.display_name, "class": i.class_name, "cost": i.cost, "weapon_id": i.weapon_id, "slot": i.slot, "is_gun": i.is_gun}))
        .collect();
    assert_eq!(f["items"], json!(items));
    for case in f["split"].as_array().unwrap() {
        let path = s(&case["path"]);
        match store::split_class_path(path) {
            Ok(split) => assert_eq!(case["ok"], json!([split.package, split.class_name, split.archetype]), "{path}"),
            Err(error) => assert_eq!(error.to_string(), s(&case["message"]), "{path}"),
        }
    }
    for case in f["item_for_path"].as_array().unwrap() {
        let path = s(&case["path"]);
        match store::store_item_for_path(path) {
            Ok(item) => assert_eq!(case["class"], json!(item.map(|i| i.class_name)), "{path}"),
            Err(error) => assert_eq!(error.to_string(), s(&case["message"]), "{path}"),
        }
    }
    for case in f["resolve"].as_array().unwrap() {
        let package_guids: HashMap<u32, String> = case["package_guids"].as_array().unwrap().iter().map(|p| (u(&p[0]), s(&p[1]).into())).collect();
        let seen_paths: HashMap<String, String> = case["seen_paths"].as_array().unwrap().iter().map(|p| (s(&p[0]).into(), s(&p[1]).into())).collect();
        let result = store::resolve_class_path(case["item_path"].as_str(), &package_guids, &seen_paths);
        assert_eq!(json!(result), case["result"], "{}", case["item_path"]);
    }
}

#[test]
fn allocator_matches_python() {
    let f = fixture("guns_alloc");
    assert_eq!(f["first"], json!([loader::FIRST_PURCHASE_CHANNEL, loader::FIRST_PURCHASE_GUID, loader::SETUP_BUNCH_COUNT]));
    for case in f["fresh"].as_array().unwrap() {
        let path = s(&case["path"]);
        let item = store::store_item(s(&case["load"]["class"])).unwrap();
        let mut alloc = PurchaseAllocator::default();
        if item.is_gun {
            let mut load = loader::plan_gun_load(path, item, &mut alloc).unwrap();
            assert_eq!(alloc.next_guid, u(&case["next_guid_after_plan"]), "{path}");
            let eq = loader::gun_equippable::<&str>(&mut load, Some(&mut alloc), &[], Vec::new()).unwrap();
            assert_eq!(eq.state_guids.get("InactiveState"), Some(u(&case["inactive"])), "{path}");
            assert_eq!(load_value(&load), case["load"], "{path}");
        } else {
            let load = loader::plan_item_load(path, item, &mut alloc).unwrap();
            assert_eq!(item_load_value(&load), case["load"], "{path}");
            assert!(case["inactive"].is_null());
        }
        assert_eq!((alloc.next_channel, alloc.next_guid), (u(&case["next_channel"]), u(&case["next_guid"])), "{path}");
    }
    for case in f["raw"].as_array().unwrap() {
        let mut alloc = PurchaseAllocator::new(u(&case["start"][0]), u(&case["start"][1]));
        for step in case["steps"].as_array().unwrap() {
            let result = match s(&step[0]) {
                "channel" => json!(alloc.channel()),
                "dynamic" => json!(alloc.dynamic_guid()),
                _ => {
                    let (a, b) = alloc.static_pair();
                    json!([a, b])
                }
            };
            assert_eq!(result, step[1], "{step}");
            assert_eq!((alloc.next_channel, alloc.next_guid), (u(&step[2]), u(&step[3])), "{step}");
        }
    }
}

#[test]
fn equippable_builders_match_python() {
    let f = fixture("guns_equippables");
    let [pistol, knife] = equippables::starting_equippables();
    assert_eq!(f["starting"], json!([eq_value(&pistol), eq_value(&knife)]));
    let abilities = equippables::starting_ability_resources();
    assert_eq!(f["abilities"], json!(abilities.iter().map(eq_value).collect::<Vec<_>>()));
    assert_eq!(f["armor"], json!(equippables::ARMOR_SHIELD_VALUES.iter().map(|(c, v)| json!([c, v])).collect::<Vec<_>>()));
    assert_eq!(f["pistol_state_guids"], json!(equippables::BASE_PISTOL_STATE_GUIDS.iter().map(|(n, g)| json!([n, g])).collect::<Vec<_>>()));
    for (class, shield) in equippables::ARMOR_SHIELD_VALUES {
        assert_eq!(equippables::armor_shield_value(class), Some(shield));
    }

    let vandal = eq_from(&f["vandal"]);
    let by_key = |key: &str| -> EquippableActor {
        match key {
            "BasePistol" => pistol.clone(),
            "Melee" => knife.clone(),
            "vandal" => vandal.clone(),
            other => abilities.iter().find(|a| a.key == other).unwrap().clone(),
        }
    };
    for case in f["auth"].as_array().unwrap() {
        let eq = by_key(s(&case["eq"]));
        let (state, export) = (s(&case["state"]), case["export"].as_bool().unwrap());
        let result = equippables::build_auth_state_body(&eq, state, export);
        if case.get("error").is_some() {
            assert!(result.is_err(), "unknown state {state}");
        } else {
            check(result, case, &format!("auth {} {state} {export}", eq.key));
        }
    }
    for case in f["machine_open"].as_array().unwrap() {
        check(equippables::build_machine_open_body(u(&case["actor"]), u(&case["machine"]), s(&case["name"])), case, "machine open");
    }
    for case in f["machine_state"].as_array().unwrap() {
        let (state, export) = (s(&case["state"]), case["export"].as_bool().unwrap());
        let result = equippables::build_machine_state_body(&vandal, &vandal.extra_machines[0], state, export);
        if case.get("error").is_some() {
            assert!(result.is_err(), "unknown zoom state {state}");
        } else {
            check(result, case, &format!("machine state {state} {export}"));
        }
    }
    for case in f["resource"].as_array().unwrap() {
        let eq = by_key(s(&case["eq"]));
        let resource = &eq.resources[case["resource"].as_u64().unwrap() as usize];
        let amount = case["amount"].as_i64().unwrap();
        let export = case["export"].as_bool().unwrap();
        check(equippables::build_resource_amount_body(&eq, resource, amount, export), case, &format!("resource {} {} {amount}", eq.key, resource.name));
    }
    for case in f["shield"].as_array().unwrap() {
        let (life, maximum) = (case["life"].as_f64().unwrap(), case["maximum"].as_f64().unwrap());
        check(equippables::build_shield_section_body(life, maximum, case["export"].as_bool().unwrap()), case, &format!("shield {life} {maximum}"));
    }
    for case in f["validate"].as_array().unwrap() {
        let args = &case["args"];
        let (Some(actor), Some(slot)) = (args["actor_guid"].as_u64(), args["slot"].as_u64()) else {
            // negative python ints have no u32 form; python refused them
            assert!(case.get("error").is_some());
            continue;
        };
        let result = EquippableActor::new(s(&args["key"]), s(&args["class_name"]), u(&args["channel"]), actor as u32, slot as u32, u(&args["state_machine_guid"]), state_guids(&args["state_guids"]));
        match case.get("error") {
            Some(_) => assert_eq!(result.unwrap_err(), RepError::Invalid(s(&case["message"]).into()), "{args}"),
            None => assert!(result.is_ok(), "{args}"),
        }
    }

    let m = &f["melee"];
    assert_eq!((u(&m["channel"]), u(&m["actor"]), u(&m["state_machine"])), (melee::MELEE_CHANNEL, melee::MELEE_ACTOR_GUID, melee::MELEE_STATE_MACHINE_GUID));
    assert_eq!(m["states"], state_guids_value(&melee::melee_state_guids()));
    check(melee::build_melee_autonomous_role_body(), &m["role"], "melee role");
    check(melee::build_melee_state_machine_open(), &m["sm_open"], "melee sm open");
    for case in m["auth"].as_array().unwrap() {
        let (state, export) = (s(&case["state"]), case["export"].as_bool().unwrap());
        check(melee::build_melee_auth_state_body(state, export), case, &format!("melee auth {state} {export}"));
    }
}

#[test]
fn attachments_match_python() {
    let f = fixture("guns_attachments");
    assert_eq!(f["handles"], json!([attachments::ATTACHMENT_ASSETS_HANDLE, attachments::ATTACHMENT_ASSET_IDS_HANDLE]));
    assert_eq!(u(&f["guard"]) as usize, attachments::GUN_OPEN_BODY_GUARD_BITS);
    let table: Vec<Value> = attachments::FORCED_ATTACHMENTS
        .iter()
        .map(|(class, assets)| json!([class, assets.iter().map(|a| json!([a.package_path, a.cdo_name, a.uuid_words])).collect::<Vec<_>>()]))
        .collect();
    assert_eq!(f["table"], json!(table));
    for case in f["lookup"].as_array().unwrap() {
        assert_eq!(attachments::forced_attachments(s(&case[0])).len() as u64, case[1].as_u64().unwrap(), "{case}");
    }
    for case in f["object_arrays"].as_array().unwrap() {
        let guids: Vec<u32> = case["guids"].as_array().unwrap().iter().map(u).collect();
        let mut w = BitWriter::new();
        attachments::append_object_array_property(&mut w, u(&case["handle"]), &guids);
        check(Ok(w.finish().unwrap()), case, "object array");
    }
    for case in f["guid_arrays"].as_array().unwrap() {
        let words: Vec<[u32; 4]> = case["words"].as_array().unwrap().iter().map(|w| std::array::from_fn(|i| u(&w[i]))).collect();
        let mut w = BitWriter::new();
        attachments::append_guid_array_property(&mut w, u(&case["handle"]), &words);
        check(Ok(w.finish().unwrap()), case, "guid array");
    }
    for case in f["pre_export"].as_array().unwrap() {
        let assets: &[AttachmentAsset] = attachments::forced_attachments(s(&case["class"]));
        let pairs: Vec<(u32, u32)> = case["pairs"].as_array().unwrap().iter().map(|p| (u(&p[0]), u(&p[1]))).collect();
        check(attachments::attachment_pre_export_body(assets, &pairs), case, &format!("pre-export {}", case["class"]));
    }
}

#[test]
fn gun_loader_matches_python() {
    let f = fixture("guns_loader");
    for gun in f["guns"].as_array().unwrap() {
        let path = s(&gun["path"]);
        let item = store::store_item(s(&gun["load"]["class"])).unwrap();
        let mut alloc = PurchaseAllocator::default();
        let mut load = loader::plan_gun_load(path, item, &mut alloc).unwrap();
        assert_eq!(load_value(&load), gun["load"], "{path}");
        let assets = attachments::forced_attachments(item.class_name);
        assert_eq!(gun["attachments"], json!(assets.iter().map(|a| json!([a.package_path, a.cdo_name, a.uuid_words])).collect::<Vec<_>>()));
        check(attachments::attachment_pre_export_body(assets, &load.attachment_guid_pairs), &gun["pre_export"], &format!("{path} pre-export"));
        check(loader::gun_open_body(&load, None), &gun["open_unskinned"], &format!("{path} open"));
        let saved = selection(&gun["saved"]);
        let standard = selection(&gun["standard"]);
        if let Ok(Some(paths)) = &saved {
            check(loader::gun_open_body(&load, Some(paths)), &gun["open_saved"], &format!("{path} open saved"));
        }
        if let Ok(Some(paths)) = &standard {
            check(loader::gun_open_body(&load, Some(paths)), &gun["open_standard"], &format!("{path} open standard"));
        }
        let chosen = loader::gun_open_with_fallback(&load, saved, standard).unwrap();
        check(Ok(chosen.body), &gun["chosen"], &format!("{path} chosen"));
        assert_eq!(json!(chosen.cosmetics.map(|p| p.skin.asset_id)), gun["chosen"]["skin"], "{path} chosen skin");
        assert_eq!(chosen.errors.len(), gun["chosen"]["errors"].as_array().unwrap().len(), "{path} chosen errors");
        let unskinned = loader::gun_open_with_fallback(&load, Ok(None), Ok(None)).unwrap();
        check(Ok(unskinned.body), &gun["chosen_unskinned_only"], &format!("{path} unskinned only"));
        check(loader::gun_owner_body(&load), &gun["owner"], &format!("{path} owner"));
        check(loader::gun_role_body(&load), &gun["role"], &format!("{path} role"));
        check(loader::gun_state_machine_open_body(&load), &gun["sm_open"], &format!("{path} sm open"));
        check(loader::gun_initial_state_body(&mut load, &mut alloc), &gun["initial"], &format!("{path} initial"));
        assert_eq!(load.state_guids.get("InactiveState"), Some(u(&gun["inactive"])), "{path}");
        assert_eq!(alloc.next_guid, u(&gun["next_guid"]), "{path}");
        if let Some(case) = gun.get("open_missing_pair") {
            let mut broken = load.clone();
            broken.attachment_guid_pairs.pop();
            check(loader::gun_open_body(&broken, None), case, &format!("{path} missing pair"));
        }
    }
    for armor in f["armor"].as_array().unwrap() {
        let item = store::store_item(s(&armor["load"]["class"])).unwrap();
        let mut alloc = PurchaseAllocator::default();
        let load = loader::plan_item_load(s(&armor["path"]), item, &mut alloc).unwrap();
        assert_eq!(item_load_value(&load), armor["load"]);
        check(loader::item_open_body(&load), &armor["open"], "armor open");
        assert_eq!(alloc.next_guid, u(&armor["next_guid"]));
    }
    for case in f["plan_errors"].as_array().unwrap() {
        let item = store::store_item(s(&case["class"])).unwrap();
        let mut alloc = PurchaseAllocator::default();
        let error = match s(&case["kind"]) {
            "gun" => loader::plan_gun_load(s(&case["path"]), item, &mut alloc).map(|_| ()).unwrap_err(),
            _ => loader::plan_item_load(s(&case["path"]), item, &mut alloc).map(|_| ()).unwrap_err(),
        };
        assert_eq!(error, RepError::Invalid(s(&case["message"]).into()), "{case}");
        assert_eq!((alloc.next_channel, alloc.next_guid), (u(&case["next_channel"]), u(&case["next_guid"])));
    }
    let mut alloc = PurchaseAllocator::default();
    let mut load = loader::plan_gun_load("/Game/Equippables/Guns/Sidearms/Revolver/RevolverPistol.RevolverPistol_C", store::store_item("RevolverPistol_C").unwrap(), &mut alloc).unwrap();
    let error = loader::gun_equippable(&mut load, None, &["InactiveState"], Vec::new()).unwrap_err();
    assert_eq!(error, RepError::Invalid(s(&f["equippable_without_allocator"]["message"]).into()));

    for sequence in f["sequences"].as_array().unwrap() {
        let name = s(&sequence["name"]);
        let mut alloc = PurchaseAllocator::default();
        // python's load list: armor plans hold a place too
        let mut loads: Vec<Option<GunLoad>> = Vec::new();
        for (index, step) in sequence["steps"].as_array().unwrap().iter().enumerate() {
            let label = format!("{name} step {index}");
            match s(&step["op"]) {
                "plan" => {
                    let path = s(&step["path"]);
                    let item = store::store_item(s(&step["load"]["class"])).unwrap();
                    if item.is_gun {
                        let load = loader::plan_gun_load(path, item, &mut alloc).unwrap();
                        assert_eq!(load_value(&load), step["load"], "{label}");
                        check(attachments::attachment_pre_export_body(attachments::forced_attachments(item.class_name), &load.attachment_guid_pairs), &step["pre_export"], &label);
                        check(loader::gun_open_body(&load, None), &step["open"], &label);
                        loads.push(Some(load));
                    } else {
                        let load = loader::plan_item_load(path, item, &mut alloc).unwrap();
                        assert_eq!(item_load_value(&load), step["load"], "{label}");
                        check(loader::item_open_body(&load), &step["open"], &label);
                        loads.push(None);
                    }
                }
                "stage3" => {
                    let load = loads[step["load_index"].as_u64().unwrap() as usize].as_mut().unwrap();
                    check(loader::gun_initial_state_body(load, &mut alloc), &step["body"], &label);
                    assert_eq!(state_guids_value(&load.state_guids), step["state_guids"], "{label}");
                }
                _ => {
                    let load = loads[step["load_index"].as_u64().unwrap() as usize].as_mut().unwrap();
                    let names: Vec<&str> = step["names"].as_array().unwrap().iter().map(s).collect();
                    let machines: Vec<ItemMachine> = step["machines"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|m| ItemMachine {
                            name: s(&m["name"]).into(),
                            address: m["address"].as_u64().unwrap(),
                            starting: m["starting"].as_str().map(str::to_string),
                            state_names: m["states"].as_array().unwrap().iter().map(|n| s(n).to_string()).collect(),
                        })
                        .collect();
                    let extra = loader::plan_extra_machines(&mut alloc, &machines, step["main_sm"].as_u64().unwrap());
                    let eq = loader::gun_equippable(load, Some(&mut alloc), &names, extra).unwrap();
                    assert_eq!(eq_value(&eq), step["equippable"], "{label}");
                    for (machine, bodies) in eq.extra_machines.iter().zip(step["extra_bodies"].as_array().unwrap()) {
                        check(equippables::build_machine_open_body(eq.actor_guid, machine.guid, &machine.name), &bodies[0], &label);
                        check(equippables::build_machine_state_body(&eq, machine, &machine.starting_state, true), &bodies[1], &label);
                    }
                    for case in step["auth"].as_array().unwrap() {
                        let state = s(&case[0]);
                        check(equippables::build_auth_state_body(&eq, state, state != "InactiveState"), &case[1], &format!("{label} {state}"));
                    }
                    for (resource, (case, amount)) in eq.resources.iter().zip(step["resources"].as_array().unwrap().iter().zip([25, 50])) {
                        check(equippables::build_resource_amount_body(&eq, resource, amount, true), case, &label);
                    }
                }
            }
            assert_eq!((alloc.next_channel, alloc.next_guid), (u(&step["next_channel"]), u(&step["next_guid"])), "{label}");
        }
    }
}
