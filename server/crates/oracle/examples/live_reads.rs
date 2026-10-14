//! read-only live check of every LiveClient read (spec 09) against the running
//! client. prints one json object on stdout, shaped like the python readers'
//! results so the two can be diffed. ReadProcessMemory only: no writes, no
//! injection, no packets.
//!
//! usage: cargo run -p aries-oracle --example live_reads [-- --keys | --serve]
//!   --keys   also runs the handler scan (prints fingerprints, never key bytes)
//!   --serve  one compact sample (no tags) per stdin line, for lockstep diffs
//!            against the python readers while someone is playing

use aries_oracle::client::{LiveClient, WatchList, client_process_alive};
use aries_oracle::equippable::{GUN_RESOURCES, ResourceSlot, SMOKE_RESOURCES, TAILWIND_RESOURCES, UPDRAFT_RESOURCES};
use aries_oracle::keys::fingerprint;
use aries_oracle::{OracleError, Process, find_processes};
use serde_json::{Map, Value, json};

/// the slots the mirror polls in production: pistol 1, melee 2, C 3, Q 4, E 5, X 9 (spec 09 §4.5)
const SLOTS: [u32; 6] = [1, 2, 3, 4, 5, 9];
/// the tags the ability open bodies use (range_abilities.py:148-155)
const TUNING_TAGS: [&str; 6] = [
    "AbilityTuning.Wushu.4.SmokeDuration",
    "AbilityTuning.Wushu.4.SmokeMaxCharges",
    "AbilityTuning.Wushu.E.DashVelocityMultiplier",
    "AbilityTuning.Wushu.E.PrimeDuration",
    "AbilityTuning.Wushu.E.MaxCharges",
    "AbilityTuning.Wushu.Q.MaxCharges",
];

fn wrap<T>(value: Result<T, OracleError>, to_json: impl FnOnce(T) -> Value) -> Value {
    match value {
        Ok(value) => json!({"ok": to_json(value)}),
        Err(error) => json!({"error": error.to_string()}),
    }
}

/// resources by slot, as the equippables declare them (RE:272-274, 326-327)
fn resources_for(slot: u32) -> &'static [ResourceSlot<'static>] {
    match slot {
        1 => &GUN_RESOURCES,
        3 => &SMOKE_RESOURCES,
        4 => &UPDRAFT_RESOURCES,
        5 => &TAILWIND_RESOURCES,
        _ => &[],
    }
}

/// every read once, in the order a python authority pass reaches them.
fn sample(client: &mut LiveClient<Process>, with_tags: bool) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("pid".into(), json!(client.pid()));
    out.insert("image_base".into(), json!(client.image_base()));
    out.insert("name_key".into(), json!(client.names.key().iter().map(|b| format!("{b:02x}")).collect::<String>()));
    out.insert("world_chain".into(), wrap(client.world_chain(), |c| {
        json!({"engine": c.engine, "viewport": c.viewport, "world": c.world, "net_driver": c.net_driver, "game_instance": c.game_instance,
               "local_player": c.local_player, "controller": c.controller, "pawn": c.pawn, "inventory": c.inventory,
               "current_equippable": c.current_equippable})
    }));
    out.insert("find_world".into(), wrap(client.find_world(), |v| json!(v)));
    out.insert("find_controller".into(), wrap(client.find_controller(), |v| json!(v)));
    out.insert("find_pawn".into(), wrap(client.find_pawn(), |v| json!(v)));
    out.insert("read_camera".into(), wrap(client.camera(), |c| c.map_or(Value::Null, |c| json!([c.location, c.rotation]))));

    // R4/R5: the slot machines, then the cached state names of each machine
    let machines = client.slot_machines(&SLOTS);
    let machine_list = machines.clone().unwrap_or_default();
    out.insert("read_slot_machines".into(), wrap(machines, |machines| {
        json!(machines
            .iter()
            .map(|m| json!({"slot": m.slot, "item": m.item, "class_name": m.class_name, "state_machine": m.state_machine,
                            "predicted": m.predicted, "authoritative": m.authoritative, "is_current": m.is_current}))
            .collect::<Vec<_>>())
    }));
    let mut cached = Map::new();
    let mut item_machines = Map::new();
    let mut resources = Map::new();
    let mut seeds = Map::new();
    for machine in &machine_list {
        let states = client.caches.state_names.get(&machine.state_machine).cloned();
        cached.insert(machine.slot.to_string(), json!(states.map(|s| s.iter().map(|(n, p)| json!([n, p])).collect::<Vec<_>>())));
        // R6
        item_machines.insert(machine.slot.to_string(), wrap(client.item_machines(machine.item), |list| {
            json!(list
                .iter()
                .map(|m| json!({"name": m.name, "address": m.address, "predicted": m.predicted, "authoritative": m.authoritative,
                                "starting": m.starting, "state_names": m.state_names}))
                .collect::<Vec<_>>())
        }));
        // R7
        let declared = resources_for(machine.slot);
        if !declared.is_empty() {
            resources.insert(machine.slot.to_string(), wrap(client.resource_amounts(machine.item, declared), |amounts| {
                json!(amounts.iter().map(|(n, a)| (n.clone(), json!([a.predicted, a.authoritative, a.default_amount]))).collect::<Map<_, _>>())
            }));
        }
        // R8, read for every machine here (the responder: the current one with resources)
        let key = machine.class_name.trim_end_matches("_C").to_string();
        seeds.insert(machine.slot.to_string(), json!(client.firing_seed(machine.state_machine, &key).unwrap_or(None)));
    }
    out.insert("cached_state_names".into(), Value::Object(cached));
    out.insert("read_item_machines".into(), Value::Object(item_machines));
    out.insert("read_resource_amounts".into(), Value::Object(resources));
    out.insert("read_firing_seed".into(), Value::Object(seeds));

    out.insert("read_input_replication".into(), wrap(client.input_replication(), |r| {
        r.map_or(Value::Null, |r| {
            json!({"stopped": r.stopped, "requested_correction": r.requested_correction, "inventory_correction": r.inventory_correction,
                   "current_equippable_class": r.current_equippable_class})
        })
    }));
    out.insert("read_pending_input_bounds".into(), wrap(client.pending_input_bounds(), |b| {
        b.map_or(Value::Null, |b| json!({"component": b.component, "stopped": b.stopped, "cursor": b.cursor, "count": b.count, "latest": b.latest}))
    }));
    out.insert("read_saved_moves".into(), wrap(client.saved_moves(), |m| {
        m.map_or(Value::Null, |m| {
            json!({"count": m.count, "oldest": m.oldest, "newest": m.newest, "last_acked": m.last_acked, "numbers": m.numbers,
                   "newest_timestamp_bits": m.newest_timestamp_bits, "newest_timestamp": m.newest_timestamp(),
                   "move_handler_gate_open": m.move_handler_gate_open, "dropping_moves": m.dropping_moves})
        })
    }));
    out.insert("resolve_rpc_index".into(), wrap(client.client_set_hud_index(), |r| {
        r.map_or(Value::Null, |r| json!({"field_index": r.field_index, "class_max_index": r.class_max_index, "pawn": r.pawn, "controller": r.controller}))
    }));
    let mut projectiles = Map::new();
    for channel in [10000u32, 10001, 10002, 10003] {
        projectiles.insert(channel.to_string(), wrap(client.projectile(channel), |s| {
            s.map_or(Value::Null, |s| {
                json!({"class_name": s.class_name, "location": s.location, "collisions": s.collisions, "hit_location": s.hit_location,
                       "hit_normal": s.hit_normal})
            })
        }));
    }
    out.insert("read_projectile".into(), Value::Object(projectiles));
    out.insert("hud_refresh".into(), wrap(client.hud_state(), |h| json!({"collapsed": h.collapsed, "cinematic": h.cinematic})));
    out.insert("ultimate_points".into(), wrap(client.ult_required(), |v| json!(v)));
    if with_tags {
        let tags_started = std::time::Instant::now();
        out.insert("predict_net_indices".into(), wrap(client.tag_net_indices(), |t| {
            let used: Map<String, Value> = TUNING_TAGS.iter().map(|tag| (tag.to_string(), json!(t.indices.get(*tag)))).collect();
            json!({"count": t.indices.len(), "used": used, "true_bit_num": t.true_bit_num, "first_bit_segment": t.first_bit_segment,
                   "constructed": t.constructed})
        }));
        out.insert("tag_read_ms".into(), json!(tags_started.elapsed().as_millis() as u64));
    }
    out.insert("alive".into(), json!(client.alive()));
    out.insert("client_process_alive".into(), json!(client_process_alive(client.pid())));
    out
}

fn main() {
    let scan_keys = std::env::args().any(|a| a == "--keys");
    let serve = std::env::args().any(|a| a == "--serve");
    let found = find_processes("VALORANT-Win64-Shipping.exe");
    if found.len() != 1 {
        eprintln!("expected one client, found {}", found.len());
        std::process::exit(1);
    }
    let target = &found[0];
    eprintln!("client pid {} image base {:#x}", target.pid, target.image_base);
    let started = std::time::Instant::now();
    let mut client = match LiveClient::<Process>::open(target.pid, target.image_base) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("reader open failed: {error}");
            std::process::exit(1);
        }
    };
    if serve {
        let mut line = String::new();
        while std::io::stdin().read_line(&mut line).map(|n| n > 0).unwrap_or(false) {
            line.clear();
            let sampled = std::time::Instant::now();
            let mut out = sample(&mut client, false);
            out.insert("read_us".into(), json!(sampled.elapsed().as_micros() as u64));
            println!("{}", Value::Object(out));
        }
        return;
    }
    let mut out = sample(&mut client, true);
    // one §7.2 snapshot with the production watch list, timed
    let watch = WatchList {
        slots: SLOTS.to_vec(),
        resources: [1, 3, 4, 5].iter().map(|&slot| (slot, resources_for(slot).to_vec())).collect(),
        item_machine_slots: vec![1, 4, 5],
        firing: vec![(1, "BasePistol".into())],
        projectile_channels: vec![10000],
    };
    let snapshot = client.snapshot(&watch);
    let field = |ok: bool| if ok { "ok" } else { "error" };
    out.insert("snapshot".into(), json!({
        "read_us": snapshot.read_us,
        "chain": field(snapshot.chain.is_ok()),
        "slots": snapshot.slots.as_ref().map(|s| s.len()).ok(),
        "resources": snapshot.resources.iter().map(|(slot, r)| json!([slot, field(r.is_ok())])).collect::<Vec<_>>(),
        "item_machines": snapshot.item_machines.iter().map(|(slot, r)| json!([slot, r.as_ref().map(|m| m.len()).unwrap_or(0)])).collect::<Vec<_>>(),
        "firing_seeds": snapshot.firing_seeds,
        "camera": field(snapshot.camera.is_ok()),
        "input_replication": field(snapshot.input_replication.is_ok()),
        "ult_required": snapshot.ult_required.as_ref().ok(),
        "hud": snapshot.hud.as_ref().ok().map(|h| [h.collapsed, h.cinematic]),
        "saved_moves_newest": snapshot.saved_moves.as_ref().ok().and_then(|m| m.as_ref()).map(|m| m.newest),
    }));
    if scan_keys {
        let scan_started = std::time::Instant::now();
        out.insert("handler_keys".into(), wrap(client.handler_keys(30.0), |k| {
            json!({"handler": k.handler, "client_send_fingerprint": fingerprint(&k.client_send), "server_send_fingerprint": fingerprint(&k.server_send)})
        }));
        out.insert("key_scan_ms".into(), json!(scan_started.elapsed().as_millis() as u64));
    }
    out.insert("total_ms".into(), json!(started.elapsed().as_millis() as u64));
    println!("{}", serde_json::to_string_pretty(&Value::Object(out)).unwrap());
}
