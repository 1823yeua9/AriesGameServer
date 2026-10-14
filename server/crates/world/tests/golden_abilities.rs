//! abilities (spec 08 section 2) against server/fixtures/world_abilities.json:
//! loads, the three setup bodies, the allocator, AbilityTuning blocks and the
//! B18 opens with tuning for fixed tag-index tables.

mod golden_ab_common;

use std::collections::HashMap;

use aries_world::abilities::{
    ABILITY_INDICES, AbilityAllocator, AbilityLoad, FIRST_ABILITY_GUID, STARTING_EQUIPPABLE_COUNT, TuningEntry, TuningValue, ability_setup_bodies,
    ability_tuning_guid, build_ability_open_with_tuning, build_ability_tuning_body, plan_ability_load, starting_ability_tunings,
    starting_ability, starting_equippable_guids,
};
use golden_ab_common::{check, fixture, python_error, u32_of};
use serde_json::Value;

fn load_matches(load: &AbilityLoad, case: &Value) {
    assert_eq!(load.index as u64, case["index"].as_u64().unwrap());
    assert_eq!(load.class_name, case["class_name"].as_str().unwrap());
    assert_eq!(load.channel, u32_of(&case["channel"]));
    assert_eq!(load.actor_guid, u32_of(&case["actor_guid"]));
    assert_eq!(load.slot, u32_of(&case["slot"]));
    assert_eq!(load.state_machine_guid, u32_of(&case["state_machine_guid"]));
    assert_eq!(load.inactive_guid, u32_of(&case["inactive_guid"]));
}

#[test]
fn constants_and_tuning_rows_match_python() {
    let f = fixture("world_abilities");
    let indices: Vec<usize> = f["constants"]["ability_indices"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as usize).collect();
    assert_eq!(indices, ABILITY_INDICES.to_vec());
    assert_eq!(FIRST_ABILITY_GUID, u32_of(&f["constants"]["first_ability_guid"]));
    for index in ABILITY_INDICES {
        assert_eq!(ability_tuning_guid(index), u32_of(&f["constants"]["tuning_guids"][index.to_string()]));
    }
    assert_eq!(STARTING_EQUIPPABLE_COUNT as u64, f["constants"]["starting_equippable_count"].as_u64().unwrap());
    // the private copy of rsb:683-688 (archetype, package, slot) against the python table
    let table = f["constants"]["starting_equippables"].as_array().unwrap();
    let mut allocator = AbilityAllocator::default();
    for index in ABILITY_INDICES {
        let row = &table[index];
        let load = plan_ability_load(index, &mut allocator).unwrap();
        assert_eq!(Some(load.class_name), row[0].as_str().unwrap().strip_prefix("Default__"));
        assert_eq!(load.slot, u32_of(&row[2]));
        assert_eq!(starting_equippable_guids(index).unwrap().0, load.actor_guid);
        let (archetype, package, slot) = starting_ability(index).unwrap();
        assert_eq!((archetype, package, slot), (row[0].as_str().unwrap(), row[1].as_str().unwrap(), u32_of(&row[2])), "rsb row {index}");
    }
    // rsb:699-706: only the seven starting equippables have guids
    assert_eq!(table.len(), STARTING_EQUIPPABLE_COUNT);
    for (index, guids) in f["constants"]["starting_equippable_guids"].as_array().unwrap().iter().enumerate() {
        let want = (u32_of(&guids[0]), u32_of(&guids[1]), u32_of(&guids[2]));
        assert_eq!(starting_equippable_guids(index), Some(want), "index {index}");
    }
    assert_eq!(f["constants"]["past_the_table"], "ValueError");
    assert_eq!(starting_equippable_guids(STARTING_EQUIPPABLE_COUNT), None);
    for index in [0, 1, 6] {
        assert!(starting_ability(index).is_none());
    }
    let tunings = f["tunings"].as_object().unwrap();
    assert_eq!(tunings.len(), 3);
    for (index, rows) in tunings {
        let ours = starting_ability_tunings(index.parse().unwrap()).expect("rows");
        let rows = rows.as_array().unwrap();
        assert_eq!(ours.len(), rows.len(), "ability {index}");
        for ((tag, value), row) in ours.iter().zip(rows) {
            assert_eq!(*tag, row[0].as_str().unwrap());
            match (row[1].as_str().unwrap(), value) {
                ("int", TuningValue::Int(v)) => assert_eq!(*v, row[2].as_i64().unwrap(), "{tag}"),
                ("float", TuningValue::Float(v)) => assert_eq!(v.to_bits(), row[2].as_f64().unwrap().to_bits(), "{tag}"),
                (kind, value) => panic!("{tag}: python {kind}, rust {value:?}"),
            }
        }
    }
    assert!(starting_ability_tunings(5).is_none());
}

#[test]
fn loads_and_setup_bodies_match_python() {
    let f = fixture("world_abilities");
    let mut allocator = AbilityAllocator::default();
    let loads: Vec<AbilityLoad> = ABILITY_INDICES.iter().map(|&index| plan_ability_load(index, &mut allocator).unwrap()).collect();
    assert_eq!(allocator.next_guid, u32_of(&f["after_plans"]["next_guid"]));
    assert_eq!(allocator.next_channel, u32_of(&f["after_plans"]["next_channel"]));
    let cases = f["loads"].as_array().unwrap();
    assert_eq!(cases.len(), 7);
    for (k, case) in cases.iter().enumerate() {
        let load = if k < loads.len() {
            loads[k].clone()
        } else {
            // the synthetic loads reuse a production class at other guids
            let class = loads.iter().find(|l| l.class_name == case["class_name"].as_str().unwrap()).unwrap().class_name;
            AbilityLoad {
                index: case["index"].as_u64().unwrap() as usize,
                class_name: class,
                channel: u32_of(&case["channel"]),
                actor_guid: u32_of(&case["actor_guid"]),
                slot: u32_of(&case["slot"]),
                state_machine_guid: u32_of(&case["state_machine_guid"]),
                inactive_guid: u32_of(&case["inactive_guid"]),
            }
        };
        load_matches(&load, case);
        let bodies = ability_setup_bodies(&load).unwrap();
        let expected = f["setup"][k].as_array().unwrap();
        assert_eq!(expected.len(), 3);
        for (stage, ((body, exports), want)) in bodies.into_iter().zip(expected).enumerate() {
            assert_eq!(exports, want["exports"].as_bool().unwrap(), "load {k} stage {stage} exports");
            check(Ok::<_, ()>(body), want, &format!("load {k} stage {stage}"));
        }
    }
}

#[test]
fn bad_indices_and_the_allocator_match_python() {
    let f = fixture("world_abilities");
    for case in f["bad_index"].as_array().unwrap() {
        let index = case["index"].as_i64().unwrap();
        assert!(python_error(case).is_some(), "python accepted {index}");
        let Ok(index) = usize::try_from(index) else { continue }; // a negative index has no usize form
        let mut allocator = AbilityAllocator::default();
        assert!(plan_ability_load(index, &mut allocator).is_err(), "index {index}");
        assert_eq!(allocator.next_guid, u32_of(&case["next_guid"]), "index {index} must not consume guids");
    }
    for case in f["allocator"].as_array().unwrap() {
        let mut allocator = AbilityAllocator { next_channel: 0, next_guid: u32_of(&case["start"]) };
        let guids: Vec<u32> = (0..5).map(|_| allocator.dynamic_guid()).collect();
        let want: Vec<u32> = case["guids"].as_array().unwrap().iter().map(u32_of).collect();
        assert_eq!(guids, want);
        assert_eq!(allocator.next_guid, u32_of(&case["next_guid"]));
    }
}

/// a tuning entry from the fixture, or none when python's value has no rust form.
fn entry(value: &Value) -> Option<TuningEntry> {
    let tag = u32::try_from(value["tag"].as_i64().unwrap()).ok()?;
    let value = match value["kind"].as_str().unwrap() {
        "int" => TuningValue::Int(value["value"].as_i64().unwrap()),
        "float" => TuningValue::Float(f64::from_bits(value["value_bits"].as_u64().unwrap())),
        kind => panic!("unexpected kind {kind}"),
    };
    Some(TuningEntry { tag_net_index: tag, value })
}

#[test]
fn tuning_bodies_match_python() {
    let f = fixture("world_abilities");
    let cases = f["tuning"].as_array().unwrap();
    assert!(cases.len() > 50);
    for (k, case) in cases.iter().enumerate() {
        let entries: Option<Vec<TuningEntry>> = case["entries"].as_array().unwrap().iter().map(entry).collect();
        let Some(entries) = entries else {
            assert!(python_error(case).is_some(), "case {k}: a negative tag must be refused");
            continue;
        };
        let result = build_ability_tuning_body(u32_of(&case["actor"]), u32_of(&case["component"]), &entries, u32_of(&case["tag_bits"]));
        check(result, case, &format!("tuning case {k}"));
    }
}

#[test]
fn opens_with_tuning_match_python() {
    let f = fixture("world_abilities");
    let cases = f["open"].as_array().unwrap();
    assert!(cases.len() > 90);
    for (k, case) in cases.iter().enumerate() {
        let tags: HashMap<String, u32> = case["tags"].as_object().unwrap().iter().map(|(name, index)| (name.clone(), u32_of(index))).collect();
        let index = case["index"].as_u64().unwrap() as usize;
        let result = build_ability_open_with_tuning(index, &tags, u32_of(&case["tag_bits"]));
        check(result, case, &format!("open case {k} (ability {index})"));
    }
}
