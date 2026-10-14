//! mirror policies against recorded python decisions (server/fixtures/guns_mirror.json):
//! mirror_target, machine_mirror_target and resource_target, as single-call
//! tables and as traces with the responder's send bookkeeping.

use std::collections::BTreeSet;

use aries_world::equippables::{self, EquippableActor, MirrorState, ResourceMirrorState, ResourceSubobject, StateGuids};
use serde_json::{Value, json};

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// the tables store each row as one compact json string.
fn rows(value: &Value) -> Vec<Value> {
    value.as_array().unwrap().iter().map(|row| serde_json::from_str(row.as_str().unwrap()).unwrap()).collect()
}

fn s(value: &Value) -> &str {
    value.as_str().unwrap()
}

fn state_guids(pairs: &Value) -> StateGuids {
    pairs.as_array().unwrap().iter().map(|p| (s(&p[0]).to_string(), p[1].as_u64().unwrap() as u32)).collect()
}

fn eq_from(d: &Value) -> EquippableActor {
    let u = |v: &Value| v.as_u64().unwrap() as u32;
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
        extra_machines: Vec::new(),
        mirror_states: d["mirror_states"].as_bool().unwrap(),
    }
}

fn exported(state: &MirrorState) -> Value {
    json!(state.exported.iter().collect::<Vec<_>>())
}

fn opt_i64(value: &Value) -> Option<i64> {
    value.as_i64()
}

#[test]
fn mirror_target_matches_python() {
    let f = fixture("guns_mirror");
    let equippable = |name: &str| eq_from(&f["equippables"][name]);
    let [pistol, knife] = equippables::starting_equippables();
    assert_eq!(equippable("BasePistol"), pistol);
    assert_eq!(equippable("Melee"), knife);
    assert_eq!(equippable("resource_only"), equippables::starting_ability_resources()[0]);

    for row in rows(&f["mirror_table"]) {
        let eq = equippable(s(&row[0]));
        let mut state = MirrorState {
            auth: s(&row[1]).into(),
            exported: BTreeSet::from(["InactiveState".to_string()]),
            active: row[2].as_bool().unwrap(),
            kicked: row[3].as_bool().unwrap(),
        };
        let target = equippables::mirror_target(&eq, &mut state, row[4].as_str(), row[5].as_bool().unwrap());
        assert_eq!(json!(target), row[6], "{row}");
        assert_eq!(json!([state.auth, state.active, state.kicked]), json!([row[7], row[8], row[9]]), "{row}");
    }

    for trace in f["mirror_traces"].as_array().unwrap() {
        let eq = equippable(s(&trace["eq"]));
        let mut state = MirrorState::default();
        for row in rows(&trace["rows"]) {
            let target = equippables::mirror_target(&eq, &mut state, row[0].as_str(), row[1].as_bool().unwrap());
            // the responder's send bookkeeping (range_udp_handshake.py:1704-1708)
            let export = target.as_deref().map(|target| {
                let export = state.needs_export(target);
                state.confirm(target);
                export
            });
            assert_eq!(json!([target, export, state.auth, state.active, state.kicked]), json!([row[2], row[3], row[4], row[5], row[6]]), "{} {row}", eq.key);
            assert_eq!(exported(&state), row[7], "{} {row}", eq.key);
        }
    }
}

#[test]
fn machine_mirror_target_matches_python() {
    let f = fixture("guns_mirror");
    let zoom = state_guids(&f["zoom"]);
    for row in rows(&f["machine_table"]) {
        let state = MirrorState::starting_at(s(&row[0]));
        assert_eq!(json!(equippables::machine_mirror_target(&state, row[1].as_str(), &zoom)), row[2], "{row}");
    }
    for trace in f["machine_traces"].as_array().unwrap() {
        let mut state = MirrorState::extra_machine_sent("ZoomStateDisabled");
        for row in rows(trace) {
            let target = equippables::machine_mirror_target(&state, row[0].as_str(), &zoom);
            // range_udp_handshake.py:1691-1696
            let export = target.as_deref().map(|target| {
                let export = state.needs_export(target);
                state.confirm(target);
                export
            });
            assert_eq!(json!([target, export, state.auth]), json!([row[1], row[2], row[3]]), "{row}");
            assert_eq!(exported(&state), row[4], "{row}");
        }
    }
}

#[test]
fn resource_target_matches_python() {
    let f = fixture("guns_mirror");
    for row in rows(&f["resource_table"]) {
        let mut state = ResourceMirrorState { auth: opt_i64(&row[0]), baseline: opt_i64(&row[1]), sent_at: row[2].as_f64().unwrap() };
        let target = equippables::resource_target(
            &mut state,
            row[3].as_i64().unwrap(),
            row[4].as_i64().unwrap(),
            row[5].as_i64().unwrap(),
            row[6].as_f64().unwrap(),
            row[7].as_f64(),
        );
        assert_eq!(json!([target, state.auth, state.baseline, state.sent_at]), json!([row[8], row[9], row[10], row[11]]), "{row}");
    }
    for trace in f["resource_traces"].as_array().unwrap() {
        let default = trace["default"].as_i64().unwrap();
        let refill = trace["refill"].as_f64();
        let mut state = ResourceMirrorState::default();
        for row in rows(&trace["rows"]) {
            let now = row[0].as_f64().unwrap();
            let target = equippables::resource_target(&mut state, row[1].as_i64().unwrap(), row[2].as_i64().unwrap(), default, now, refill);
            // range_udp_handshake.py:1660-1664
            let export = target.map(|amount| {
                let export = state.needs_export();
                equippables::resource_confirmed(&mut state, amount, now);
                export
            });
            assert_eq!(json!([target, export, state.auth, state.baseline, state.sent_at]), json!([row[3], row[4], row[5], row[6], row[7]]), "default {default} {row}");
        }
    }
}
