//! every production oracle read against the python readers (fixtures/oracle_reads.json,
//! tools/golden/oracle_reads.py): each scenario replays the same memory ops into
//! FakeMemory and runs the same calls in the same order, caches included.

use std::collections::BTreeMap;

use aries_oracle::client::{LiveClient, WatchList, derive_names};
use aries_oracle::equippable::{GUN_RESOURCES, InputReplication, ItemMachine, ResourceAmounts, ResourceSlot, SMOKE_RESOURCES, SlotMachine};
use aries_oracle::input::read_pending_input_bounds;
use aries_oracle::moves::{SavedMoves, read_saved_moves};
use aries_oracle::probes::{HudState, ProjectileSnapshot, read_hud_state, read_ult_required};
use aries_oracle::rpc_index::{RpcActor, resolve_rpc_index};
use aries_oracle::tags::predict_net_indices;
use aries_oracle::world::{Camera, find_controller, find_pawn, find_world, read_camera};
use aries_oracle::{FakeMemory, Memory, OracleError};
use serde_json::{Value, json};

/// the python reader is created lazily by the first call that needs it and is
/// retried until it exists (H:1591-1598); until then reads go to the raw memory.
enum Target {
    Raw(FakeMemory),
    Client(Box<LiveClient<FakeMemory>>),
}

impl Target {
    fn memory(&self) -> &FakeMemory {
        match self {
            Self::Raw(memory) => memory,
            Self::Client(client) => &client.memory,
        }
    }

    fn memory_mut(&mut self) -> &mut FakeMemory {
        match self {
            Self::Raw(memory) => memory,
            Self::Client(client) => &mut client.memory,
        }
    }

    fn client(&mut self) -> Result<&mut LiveClient<FakeMemory>, String> {
        if let Self::Raw(memory) = self {
            let names = derive_names(memory).map_err(|e| e.to_string())?;
            let memory = std::mem::take(memory);
            *self = Self::Client(Box::new(LiveClient::with_names(memory, names)));
        }
        match self {
            Self::Client(client) => Ok(client),
            Self::Raw(_) => unreachable!(),
        }
    }
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
}

fn map(memory: &mut FakeMemory, address: u64, size: u64) {
    let mut page = address & !0xFFF;
    while page < address + size {
        if memory.read_bytes(page, 1).is_err() {
            memory.write(page, &[0u8; 0x1000]);
        }
        page += 0x1000;
    }
}

/// applies a memory op; false for a call step.
fn apply(target: &mut Target, step: &Value) -> bool {
    match step["op"].as_str() {
        Some("map") => map(target.memory_mut(), step["address"].as_u64().unwrap(), step["size"].as_u64().unwrap()),
        Some("write") => target.memory_mut().write(step["address"].as_u64().unwrap(), &hex(step["hex"].as_str().unwrap())),
        Some(other) => panic!("unknown op {other}"),
        None => return false,
    }
    true
}

fn u64_arg(args: &Value, key: &str) -> u64 {
    args[key].as_u64().unwrap_or_else(|| panic!("arg {key}"))
}

fn result<T>(value: Result<T, OracleError>, to_json: impl FnOnce(T) -> Value) -> Result<Value, String> {
    value.map(to_json).map_err(|error| error.to_string())
}

// json in the shapes tools/golden/oracle_reads.py exports

fn camera_json(camera: Option<Camera>) -> Value {
    camera.map_or(Value::Null, |c| json!([c.location, c.rotation]))
}

fn slots_json(machines: &[SlotMachine]) -> Value {
    json!(machines
        .iter()
        .map(|m| json!({"slot": m.slot, "item": m.item, "class_name": m.class_name, "state_machine": m.state_machine,
                        "predicted": m.predicted, "authoritative": m.authoritative, "is_current": m.is_current}))
        .collect::<Vec<_>>())
}

fn machines_json(machines: &[ItemMachine]) -> Value {
    json!(machines
        .iter()
        .map(|m| json!({"name": m.name, "address": m.address, "predicted": m.predicted, "authoritative": m.authoritative,
                        "starting": m.starting, "state_names": m.state_names}))
        .collect::<Vec<_>>())
}

fn amounts_json(amounts: &BTreeMap<String, ResourceAmounts>) -> Value {
    json!(amounts.iter().map(|(name, a)| (name.clone(), json!([a.predicted, a.authoritative, a.default_amount]))).collect::<serde_json::Map<_, _>>())
}

fn replication_json(replication: Option<InputReplication>) -> Value {
    replication.map_or(Value::Null, |r| {
        json!({"stopped": r.stopped, "requested_correction": r.requested_correction,
               "inventory_correction": r.inventory_correction, "current_equippable_class": r.current_equippable_class})
    })
}

fn moves_json(moves: Option<SavedMoves>) -> Value {
    moves.map_or(Value::Null, |m| {
        json!({"count": m.count, "oldest": m.oldest, "newest": m.newest, "last_acked": m.last_acked, "numbers": m.numbers,
               "newest_timestamp_bits": m.newest_timestamp_bits, "newest_timestamp": m.newest_timestamp().map(f64::from),
               "move_handler_gate_open": m.move_handler_gate_open, "dropping_moves": m.dropping_moves})
    })
}

fn projectile_json(snapshot: Option<ProjectileSnapshot>) -> Value {
    snapshot.map_or(Value::Null, |s| {
        json!({"class_name": s.class_name, "location": s.location, "collisions": s.collisions,
               "hit_location": s.hit_location, "hit_normal": s.hit_normal})
    })
}

/// the responder's inline blocks keep going on failure with fixed values (H:2005-2007, 1838-1840)
fn hud_json(read: Result<HudState, OracleError>) -> Value {
    let state = read.as_ref().copied().unwrap_or(HudState::ON_ERROR);
    json!({"collapsed": state.collapsed, "cinematic": state.cinematic, "error": read.is_err()})
}

fn ult_json(read: Result<i32, OracleError>) -> Value {
    json!({"ult_required": read.as_ref().copied().unwrap_or(0), "error": read.is_err()})
}

fn call(target: &mut Target, name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "derive_name_key" => result(derive_names(target.memory()), |names| json!(names.key().iter().map(|b| format!("{b:02x}")).collect::<String>())),
        "decode_name" => {
            let index = u64_arg(args, "index") as u32;
            result(target.client()?.name(index), |name| json!(name))
        }
        "find_world" => result(find_world(target.memory()), |v| json!(v)),
        "find_controller" => result(find_controller(target.memory()), |v| json!(v)),
        "find_pawn" => result(find_pawn(target.memory()), |v| json!(v)),
        "read_camera" => result(read_camera(target.memory()), camera_json),
        "read_slot_machines" => {
            let slots: Vec<u32> = args["slots"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect();
            result(target.client()?.slot_machines(&slots), |m| slots_json(&m))
        }
        "cached_state_names" => {
            let sm = u64_arg(args, "state_machine");
            Ok(match target.client()?.caches.state_names.get(&sm) {
                Some(states) => json!(states.iter().map(|(n, p)| json!([n, p])).collect::<Vec<_>>()),
                None => Value::Null,
            })
        }
        "state_names" => {
            let sm = u64_arg(args, "state_machine");
            result(target.client()?.read_state_names(sm), |states| json!(states.iter().map(|(n, p)| json!([n, p])).collect::<Vec<_>>()))
        }
        "read_item_machines" => result(target.client()?.item_machines(u64_arg(args, "item")), |m| machines_json(&m)),
        "read_resource_amounts" => {
            let item = u64_arg(args, "item");
            let specs: Vec<(String, u64, u64)> = args["resources"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| (r[0].as_str().unwrap().to_string(), r[1].as_u64().unwrap(), r[2].as_u64().unwrap()))
                .collect();
            let resources: Vec<ResourceSlot> = specs.iter().map(|(name, item_offset, default_offset)| ResourceSlot { name, item_offset: *item_offset, default_offset: *default_offset }).collect();
            result(target.client()?.resource_amounts(item, &resources), |a| amounts_json(&a))
        }
        "read_firing_seed" => {
            let (sm, key) = (u64_arg(args, "state_machine"), args["key"].as_str().unwrap());
            // the responder swallows every error as None (H:963-964)
            Ok(json!(target.client()?.firing_seed(sm, key).unwrap_or(None)))
        }
        "read_input_replication" => result(target.client()?.input_replication(), replication_json),
        "read_pending_input_bounds" => result(read_pending_input_bounds(target.memory()), |b| match b {
            Some(b) => json!({"component": b.component, "stopped": b.stopped, "cursor": b.cursor, "count": b.count, "latest": b.latest}),
            None => Value::Null,
        }),
        "read_saved_moves" => result(read_saved_moves(target.memory()), moves_json),
        "resolve_rpc_index" => {
            let kind = match args["kind"].as_str().unwrap() {
                "controller" => RpcActor::Controller,
                _ => RpcActor::Pawn,
            };
            result(resolve_rpc_index(target.memory(), u64_arg(args, "thunk"), kind), |r| match r {
                Some(r) => json!({"field_index": r.field_index, "class_max_index": r.class_max_index, "pawn": r.pawn, "controller": r.controller}),
                None => Value::Null,
            })
        }
        "read_projectile" => result(target.client()?.projectile(u64_arg(args, "channel") as u32), projectile_json),
        "hud_refresh" => Ok(hud_json(read_hud_state(&target.client()?.memory))),
        "ultimate_points" => Ok(ult_json(read_ult_required(&target.client()?.memory))),
        "predict_net_indices" => {
            // its own temporary reader: a fresh key derivation (H:4054-4059)
            let memory = target.memory();
            let mut names = derive_names(memory).map_err(|e| e.to_string())?;
            result(predict_net_indices(memory, &mut names), |t| {
                json!({"indices": t.indices, "true_bit_num": t.true_bit_num, "first_bit_segment": t.first_bit_segment, "constructed": t.constructed})
            })
        }
        other => panic!("unknown fixture call {other}"),
    }
}

fn runs() -> Vec<Value> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/oracle_reads.json");
    let runs: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    runs.as_array().unwrap().clone()
}

#[test]
fn every_read_matches_python() {
    let mut checked = 0;
    let mut errors = 0;
    for run in runs() {
        let name = run["name"].as_str().unwrap();
        let mut target = Target::Raw(FakeMemory::new(run["image_base"].as_u64().unwrap()));
        for (index, step) in run["steps"].as_array().unwrap().iter().enumerate() {
            if apply(&mut target, step) {
                continue;
            }
            let call_name = step["call"].as_str().unwrap();
            let got = call(&mut target, call_name, &step["args"]);
            let context = format!("{name} step {index} {call_name}({})", step["args"]);
            match (&got, step.get("ok")) {
                (Ok(value), Some(expected)) => assert_eq!(value, expected, "{context}"),
                (Err(error), Some(expected)) => panic!("{context}: rust failed ({error}), python returned {expected}"),
                (Ok(value), None) => panic!("{context}: python raised {}, rust returned {value}", step["error"]),
                (Err(_), None) => errors += 1,
            }
            checked += 1;
        }
    }
    assert!(checked > 300, "only {checked} calls checked");
    assert!(errors > 100, "only {errors} failure cases checked");
}

/// the python result of the first `name(args)` call in a run: Err when it raised.
fn python_step(run: &Value, name: &str, args: Value) -> Result<Value, String> {
    let step = run["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["call"] == name && s["args"] == args)
        .unwrap_or_else(|| panic!("no {name}({args}) in {}", run["name"]));
    step.get("ok").cloned().ok_or_else(|| step["error"].to_string())
}

fn python_result(run: &Value, name: &str, args: Value) -> Value {
    python_step(run, name, args).unwrap_or_else(|error| panic!("python {name} raised {error}"))
}

/// same value, or both failed (error texts differ by language)
fn assert_same(rust: Result<Value, String>, python: Result<Value, String>, context: &str) {
    match (rust, python) {
        (Ok(rust), Ok(python)) => assert_eq!(rust, python, "{context}"),
        (Err(_), Err(_)) => {}
        (rust, python) => panic!("{context}: rust {rust:?}, python {python:?}"),
    }
}

/// a snapshot is the same reads as the separate python calls (spec 09 §7.2).
#[test]
fn snapshot_matches_python_calls() {
    for run in runs().iter().filter(|r| ["healthy", "unpossessed", "no_controller"].contains(&r["name"].as_str().unwrap())) {
        let mut target = Target::Raw(FakeMemory::new(run["image_base"].as_u64().unwrap()));
        for step in run["steps"].as_array().unwrap() {
            apply(&mut target, step);
        }
        let expected_slots = python_result(run, "read_slot_machines", json!({"slots": [1, 2, 3, 4, 5, 9]}));
        let pistol = expected_slots.as_array().unwrap().first().cloned();
        let client = target.client().unwrap();
        let watch = WatchList {
            slots: vec![1, 2, 3, 4, 5, 9],
            resources: vec![(1, GUN_RESOURCES.to_vec()), (3, SMOKE_RESOURCES.to_vec())],
            item_machine_slots: vec![1],
            firing: vec![(1, "BasePistol".into())],
            projectile_channels: vec![10000],
        };
        let snapshot = client.snapshot(&watch);
        let name = run["name"].as_str().unwrap();
        assert_eq!(slots_json(snapshot.slots.as_ref().unwrap()), expected_slots, "{name} slots");
        let chain = snapshot.chain.as_ref().unwrap();
        assert_eq!(json!(chain.world), python_result(run, "find_world", json!({})), "{name} world");
        assert_eq!(json!(chain.controller), python_result(run, "find_controller", json!({})), "{name} controller");
        assert_eq!(json!(chain.pawn), python_result(run, "find_pawn", json!({})), "{name} pawn");
        assert_same(result(snapshot.camera, camera_json), python_step(run, "read_camera", json!({})), &format!("{name} camera"));
        assert_same(result(snapshot.input_replication, replication_json), python_step(run, "read_input_replication", json!({})), &format!("{name} replication"));
        assert_same(result(snapshot.saved_moves, moves_json), python_step(run, "read_saved_moves", json!({})), &format!("{name} moves"));
        assert_eq!(hud_json(snapshot.hud), python_result(run, "hud_refresh", json!({})), "{name} hud");
        assert_eq!(ult_json(snapshot.ult_required), python_result(run, "ultimate_points", json!({})), "{name} ult");
        let (channel, projectile) = snapshot.projectiles.into_iter().next().unwrap();
        assert_same(result(projectile, projectile_json), python_step(run, "read_projectile", json!({"channel": channel})), &format!("{name} projectile"));
        let Some(pistol) = pistol else {
            // no slots: nothing per slot is read
            assert!(snapshot.resources.is_empty() && snapshot.item_machines.is_empty() && snapshot.firing_seeds.is_empty(), "{name}");
            continue;
        };
        let item = pistol["item"].as_u64().unwrap();
        let resources = json!([["MagazineAmmo", 0x1208, 0x12C], ["ReserveAmmo", 0x1200, 0x12C]]);
        let (slot, amounts) = &snapshot.resources[0];
        assert_eq!(*slot, 1);
        assert_eq!(amounts_json(amounts.as_ref().unwrap()), python_result(run, "read_resource_amounts", json!({"item": item, "resources": resources})), "{name} resources");
        assert_eq!(machines_json(snapshot.item_machines[0].1.as_ref().unwrap()), python_result(run, "read_item_machines", json!({"item": item})), "{name} machines");
        let seed_args = json!({"state_machine": pistol["state_machine"], "key": "BasePistol"});
        assert_eq!(json!(snapshot.firing_seeds[0].1), python_result(run, "read_firing_seed", seed_args), "{name} firing seed");
        // slot 3 (cloudburst) has resources on the watch list too
        assert_eq!(snapshot.resources.len(), 2, "{name}");
    }
}

/// Q4 off: an empty TMap is not cached, so the machine's states show up once the
/// client fills them, equal to the python's uncached state_names (RES:77-87).
#[test]
fn empty_state_names_can_be_reread() {
    let run = runs().into_iter().find(|r| r["name"] == "state_caches").unwrap();
    let steps = run["steps"].as_array().unwrap();
    let mut target = Target::Raw(FakeMemory::new(run["image_base"].as_u64().unwrap()));
    let mut calls = steps.iter().enumerate().filter(|(_, s)| s.get("call").is_some());
    let (first_call, first) = calls.next().unwrap();
    for step in &steps[..first_call] {
        apply(&mut target, step);
    }
    let smoke_sm = first["ok"][0]["state_machine"].as_u64().unwrap();
    let client = target.client().unwrap();
    client.keep_empty_state_names = false;
    let before = client.slot_machines(&[3]).unwrap();
    assert_eq!(before[0].predicted, None);
    assert!(!client.caches.state_names.contains_key(&smoke_sm), "empty TMap must not be cached");
    // replay to python's uncached read after the TMap filled
    let (uncached_at, uncached) = steps.iter().enumerate().find(|(_, s)| s["call"] == "state_names").unwrap();
    for step in &steps[first_call..uncached_at] {
        apply(&mut target, step);
    }
    let client = target.client().unwrap();
    let after = client.slot_machines(&[3]).unwrap();
    let states = &client.caches.state_names[&smoke_sm];
    assert_eq!(json!(states.iter().map(|(n, p)| json!([n, p])).collect::<Vec<_>>()), uncached["ok"]);
    let predicted = client.memory.u64(smoke_sm + 0x1B0).unwrap();
    assert_eq!(after[0].predicted, aries_oracle::equippable::state_name_of(states, predicted));
    assert!(after[0].predicted.is_some());
}
