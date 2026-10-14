//! the ability jobs (spec 08 section 2.4) against server/fixtures/world_ability_systems.json:
//! ruh's own job block, advance_ability_job, plan_extra_machines and
//! send_extra_machines, run per datagram on scripted slot and item reads.

mod golden_ab_common;

use aries_world::abilities::{AbilityJobs, AbilityOracle, ItemMachine, JOB_DONE, SlotMachine};
use golden_ab_common::{Sequences, check_sends, f64_of, fixture, u32_of};
use serde_json::{Value, json};

/// replays python's oracle calls in order, checking rust asks the same thing.
struct Replay<'a> {
    calls: std::slice::Iter<'a, Value>,
}

fn strings(value: &Value) -> Vec<String> {
    value.as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
}

impl AbilityOracle for Replay<'_> {
    fn slot_machine(&mut self, slot: u32) -> Result<Option<SlotMachine>, String> {
        let call = self.calls.next().expect("python made no further read");
        assert_eq!((call["call"].as_str(), u32_of(&call["slot"])), (Some("slot"), slot), "slot read order");
        if call.get("error").is_some() {
            return Err("slot read failed".into());
        }
        let machine = &call["machine"];
        if machine.is_null() {
            return Ok(None);
        }
        Ok(Some(SlotMachine {
            item: machine["item"].as_u64().unwrap(),
            class_name: machine["class_name"].as_str().map(str::to_string),
            state_machine: machine["state_machine"].as_u64().unwrap(),
            state_names: strings(&machine["state_names"]),
        }))
    }

    fn item_machines(&mut self, item: u64) -> Result<Vec<ItemMachine>, String> {
        let call = self.calls.next().expect("python made no further read");
        assert_eq!((call["call"].as_str(), call["item"].as_u64()), (Some("items"), Some(item)), "item read order");
        if call.get("error").is_some() {
            return Err("item machines read failed".into());
        }
        Ok(call["machines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| ItemMachine {
                name: m["name"].as_str().unwrap().to_string(),
                address: m["address"].as_u64().unwrap(),
                starting: m["starting"].as_str().map(str::to_string),
                state_names: strings(&m["state_names"]),
            })
            .collect())
    }
}

#[test]
fn ability_jobs_match_python() {
    let f = fixture("world_ability_systems");
    assert_eq!(f["lines"]["jobs"], 2066);
    let mut sequences = Sequences::new(&f);
    let mut jobs: Option<AbilityJobs> = None;
    let mut ready_events = Vec::new();
    let mut gave_up = None;
    for (k, datagram) in f["datagrams"].as_array().unwrap().iter().enumerate() {
        let now = f64_of(&datagram["now"]);
        let event = &datagram["event"];
        let label = format!("datagram {k}");
        // ruh:2068-2074: planned on the first datagram past the gate
        let jobs = jobs.get_or_insert_with(|| AbilityJobs::new(now).unwrap());
        let calls = datagram["oracle"].as_array().unwrap();
        let mut replay = Replay { calls: calls.iter() };
        let step = jobs.step(now, &mut replay);
        assert!(step.error.is_none(), "{label}: {:?}", step.error);
        assert!(replay.calls.next().is_none(), "{label}: python read more");
        check_sends(&step.sends, &datagram["sends"], &mut sequences, &label);
        let ready: Vec<Value> = step
            .ready
            .iter()
            .map(|e| json!([e.class_name, e.state_guids.len(), e.extra_machines.iter().map(|x| x.name.as_str()).collect::<Vec<_>>()]))
            .collect();
        assert_eq!(json!(ready), event.get("ability_ready").cloned().unwrap_or(json!([])), "{label} ability_ready");
        assert_eq!(!step.register_errors.is_empty(), event.get("ability_register_error").is_some(), "{label} register error");
        assert_eq!(!step.extra_machine_errors.is_empty(), event.get("extra_machines_error").is_some(), "{label} extra machines error");
        assert_eq!(step.gave_up.last().map(|c| json!(c)), event.get("ability_register_gave_up").cloned(), "{label} gave up");
        gave_up = gave_up.or(step.gave_up.last().copied());
        ready_events.extend(step.ready);
    }
    assert_eq!(gave_up, Some("Ability_Wushu_X_Dagger_Production_C"));

    let jobs = jobs.unwrap();
    for (job, want) in jobs.jobs.iter().zip(f["jobs"].as_array().unwrap()) {
        assert_eq!(job.stage as u64, want["stage"].as_u64().unwrap());
        assert_eq!(job.registered_tries as u64, want["registered_tries"].as_u64().unwrap());
    }
    assert!(jobs.done() && jobs.jobs.iter().all(|job| job.stage == JOB_DONE));
    assert_eq!(jobs.allocator.next_guid, u32_of(&f["next_guid"]));

    // the registered equippables python kept for the mirror
    let ready = f["ready"].as_array().unwrap();
    assert_eq!(ready_events.len(), ready.len());
    let mut mirror_keys = Vec::new();
    for (equippable, want) in ready_events.iter().zip(ready) {
        assert_eq!(equippable.key, want["key"].as_str().unwrap());
        assert_eq!(equippable.class_name, want["class_name"].as_str().unwrap());
        assert_eq!(equippable.channel, u32_of(&want["channel"]));
        assert_eq!(equippable.actor_guid, u32_of(&want["actor_guid"]));
        assert_eq!(equippable.slot, u32_of(&want["slot"]));
        assert_eq!(equippable.state_machine_guid, u32_of(&want["state_machine_guid"]));
        assert_eq!(equippable.mirror_states, want["mirror_states"].as_bool().unwrap());
        assert_eq!(json!(equippable.state_guids), want["state_guids"], "{} state guids", equippable.key);
        let extra: Vec<Value> = equippable
            .extra_machines
            .iter()
            .map(|x| json!({"name": x.name, "guid": x.guid, "starting_state": x.starting_state, "state_guids": x.state_guids}))
            .collect();
        assert_eq!(json!(extra), want["extra"], "{} extra machines", equippable.key);
        mirror_keys.extend(equippable.extra_machines.iter().map(|x| format!("{}/{}", equippable.key, x.name)));
    }
    mirror_keys.sort();
    assert_eq!(json!(mirror_keys), f["mirror_states"]);
}
