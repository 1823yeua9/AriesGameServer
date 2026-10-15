//! read budgets: the golden "healthy" client memory, counted. every result is
//! still checked against the python by reads_golden.rs; this pins how many
//! ReadProcessMemory calls each production read costs.

use aries_oracle::client::{LiveClient, WatchList, WatchedEquippable, derive_names};
use aries_oracle::equippable::{GUN_RESOURCES, ResourceSlot, SMOKE_RESOURCES, TAILWIND_RESOURCES, UPDRAFT_RESOURCES};
use aries_oracle::{Counted, FakeMemory, Memory};
use serde_json::Value;

fn hex(text: &str) -> Vec<u8> {
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
}

fn runs() -> Vec<Value> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/oracle_reads.json");
    let runs: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    runs.as_array().unwrap().clone()
}

/// the end state of the named golden run's memory
fn memory(name: &str) -> FakeMemory {
    let runs = runs();
    let run = runs.iter().find(|r| r["name"] == name).unwrap();
    let mut memory = FakeMemory::new(run["image_base"].as_u64().unwrap());
    for step in run["steps"].as_array().unwrap() {
        match step["op"].as_str() {
            Some("map") => {
                let (address, size) = (step["address"].as_u64().unwrap(), step["size"].as_u64().unwrap());
                let mut page = address & !0xFFF;
                while page < address + size {
                    if memory.read_bytes(page, 1).is_err() {
                        memory.write(page, &[0u8; 0x1000]);
                    }
                    page += 0x1000;
                }
            }
            Some("write") => memory.write(step["address"].as_u64().unwrap(), &hex(step["hex"].as_str().unwrap())),
            _ => {}
        }
    }
    memory
}

fn client(name: &str) -> LiveClient<Counted<FakeMemory>> {
    let memory = Counted::new(memory(name));
    let names = derive_names(&memory).unwrap();
    let client = LiveClient::with_names(memory, names);
    client.memory.take();
    client
}

fn watched(key: &str, slot: u32, class_name: &str, resources: &[ResourceSlot<'static>], item_machines: bool) -> WatchedEquippable {
    WatchedEquippable { key: key.into(), slot, class_name: class_name.into(), resources: resources.to_vec(), firing: true, item_machines }
}

fn mirror_watch() -> WatchList {
    WatchList {
        equippables: vec![
            watched("BasePistol", 1, "BasePistol_C", &GUN_RESOURCES, true),
            watched("Melee", 2, "Ability_Melee_Base_C", &[], false),
            watched("Smoke", 3, "Ability_Wushu_4_Smoke_C", &SMOKE_RESOURCES, true),
            watched("Updraft", 4, "Ability_Wushu_Q_Updraft_C", &UPDRAFT_RESOURCES, true),
            watched("Tailwind", 5, "Ability_Wushu_E_Tailwind_C", &TAILWIND_RESOURCES, true),
            watched("BladeStorm", 9, "Ability_Wushu_X_C", &[], false),
        ],
        projectile_channels: vec![10000],
    }
}

/// (call, reads) for every production read, warm caches, in snapshot order
fn costs(client: &mut LiveClient<Counted<FakeMemory>>) -> Vec<(&'static str, u64)> {
    let watch = mirror_watch();
    // warm the name, class and state caches like a running session
    let _ = client.snapshot(&watch);
    client.memory.take();
    let mut costs = Vec::new();
    let mut count = |name, client: &mut LiveClient<Counted<FakeMemory>>| costs.push((name, client.memory.take()));
    let slots: Vec<u32> = watch.equippables.iter().map(|e| e.slot).collect();
    let _ = client.slot_machines(&slots);
    count("slot_machines", client);
    let _ = client.input_replication();
    count("input_replication", client);
    let _ = client.pending_input_bounds();
    count("pending_input_bounds", client);
    let _ = client.saved_moves();
    count("saved_moves", client);
    let _ = client.hud_state();
    count("hud_state", client);
    let _ = client.ult_required();
    count("ult_required", client);
    let _ = client.camera();
    count("camera", client);
    let _ = client.snapshot(&watch);
    count("snapshot", client);
    let _ = client.client_set_hud_index();
    count("client_set_hud_index", client);
    let _ = client.tag_net_indices();
    count("tag_net_indices", client);
    costs
}

/// warm-cache reads per production call on a healthy client, with the chain cached
/// (the server's setting). python (one u64 per field, a GObjects walk per call) took
/// snapshot 195, slot_machines 35, input_replication 21, pending_input_bounds 16,
/// saved_moves 42, hud_state 15, ult_required 13, camera 13.
const BUDGET: [(&str, u64); 8] = [
    ("slot_machines", 13),
    ("input_replication", 5),
    ("pending_input_bounds", 2),
    ("saved_moves", 11),
    ("hud_state", 3),
    ("ult_required", 1),
    ("camera", 2),
    ("snapshot", 55),
];

#[test]
fn reads_stay_in_budget() {
    let mut walking = client("healthy");
    let mut cached = client("healthy");
    cached.chain_ttl = Some(std::time::Duration::from_secs(60));
    let costs: Vec<_> = costs(&mut walking).into_iter().zip(costs(&mut cached)).collect();
    for ((name, walk), (_, chain)) in &costs {
        eprintln!("{name:>22}: {walk:>4} walking, {chain:>4} with the chain cached");
    }
    for (name, budget) in BUDGET {
        let (_, (_, reads)) = costs.iter().find(|((call, _), _)| *call == name).unwrap();
        assert!(*reads <= budget, "{name}: {reads} reads, budget {budget}");
    }
}

/// every production read through the cached chain returns what the python-order
/// walk returns, in every golden scenario (complete chains use the cache; the rest
/// fall back to the walk).
#[test]
fn cached_chain_reads_equal_walking_reads() {
    let watch = mirror_watch();
    let slots: Vec<u32> = watch.equippables.iter().map(|e| e.slot).collect();
    let mut compared = 0;
    for run in runs() {
        let name = run["name"].as_str().unwrap();
        let memory = Counted::new(memory(name));
        let Ok(names) = derive_names(&memory) else {
            continue;
        };
        let mut walking = LiveClient::with_names(memory, names);
        let memory = Counted::new(self::memory(name));
        let mut cached = LiveClient::with_names(memory, derive_names(&walking.memory).unwrap());
        cached.chain_ttl = Some(std::time::Duration::from_secs(60));
        for client in [&mut walking, &mut cached] {
            // the same warm-up, so the caches match
            let _ = client.snapshot(&watch);
        }
        let reads = |client: &mut LiveClient<Counted<FakeMemory>>| {
            vec![
                format!("{:?}", client.slot_machines(&slots)),
                format!("{:?}", client.input_replication()),
                format!("{:?}", client.pending_input_bounds()),
                format!("{:?}", client.saved_moves()),
                format!("{:?}", client.hud_state()),
                format!("{:?}", client.ult_required()),
                format!("{:?}", client.camera()),
                format!("{:?}", client.projectile(10000)),
            ]
        };
        let (want, have) = (reads(&mut walking), reads(&mut cached));
        for (want, have) in want.iter().zip(&have) {
            // error texts may name a different address; both failing is the match
            assert!(want == have || (want.starts_with("Err") && have.starts_with("Err")), "{name}: walking {want}, cached {have}");
        }
        compared += 1;
    }
    assert!(compared >= 5, "only {compared} scenarios");
}

/// forgetting the chain (a new range entry) walks again on the next read.
#[test]
fn forget_chain_walks_again() {
    let mut client = client("healthy");
    client.chain_ttl = Some(std::time::Duration::from_secs(60));
    let _ = client.ult_required();
    client.memory.take();
    let _ = client.ult_required();
    assert_eq!(client.memory.take(), 1);
    client.forget_chain();
    let _ = client.ult_required();
    assert!(client.memory.take() > 5);
}
