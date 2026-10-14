//! the equippable mirror (python 1600-1711, 1316-1331): every 50 ms, read each
//! replicated equippable's predicted state and resources off the client and
//! confirm them as authoritative, so its prediction is never rolled back.
//! docs/port/05_authority_loop.md 6.3.

use std::collections::HashMap;
use std::sync::OnceLock;

use aries_world::equippables::{
    EquippableActor, ExtraStateMachine, MirrorState, ResourceMirrorState, build_auth_state_body, build_machine_open_body, build_machine_state_body, build_resource_amount_body,
    machine_mirror_target, mirror_target, resource_confirmed, resource_target, starting_ability_resources, starting_equippables,
};
use serde_json::{Map, Value, json};

use crate::bringup::{STARTING_EQUIPPABLES, Stage};
use crate::config::RangeConfig;
use crate::live::{LiveClient, SlotMachine};
use crate::send::Reply;

/// python equippable_mirror_polled gate.
pub const MIRROR_POLL: f64 = 0.05;
/// the smoke ability whose throw opens a cloudburst projectile.
pub const SMOKE_CLASS: &str = "Ability_Wushu_4_Smoke_C";

/// per-connection mirror state (python dicts keyed by source).
#[derive(Debug, Default, Clone)]
pub struct MirrorBook {
    pub polled: Option<f64>,
    /// python equippable_mirror_states: by key, or "key/machine" for extras
    pub states: HashMap<String, MirrorState>,
    /// python equippable_resource_states: by (key, resource)
    pub resources: HashMap<(String, String), ResourceMirrorState>,
    /// python firing_seeds: by key
    pub firing_seeds: HashMap<String, u32>,
    /// the classic was replaced by a bought sidearm (closed, no longer mirrored)
    pub starting_pistol_replaced: bool,
    /// python purchased_ready_equippables, in list order
    pub purchased: Vec<EquippableActor>,
    /// python ability_ready_equippables (key = class), in insertion order
    pub abilities: Vec<EquippableActor>,
}

impl MirrorBook {
    /// python ability_ready_equippables[s][key] = eq: replaces in place, else appends.
    pub fn set_ability(&mut self, equippable: EquippableActor) {
        match self.abilities.iter_mut().find(|e| e.key == equippable.key) {
            Some(entry) => *entry = equippable,
            None => self.abilities.push(equippable),
        }
    }
}

/// the classic and the knife (fixed layouts, built once).
pub fn starting() -> &'static [EquippableActor; 2] {
    static STARTING: OnceLock<[EquippableActor; 2]> = OnceLock::new();
    STARTING.get_or_init(starting_equippables)
}

/// smoke, updraft and tailwind charges (resource-only until the ability registers).
pub fn starting_abilities() -> &'static [EquippableActor; 3] {
    static ABILITIES: OnceLock<[EquippableActor; 3]> = OnceLock::new();
    ABILITIES.get_or_init(starting_ability_resources)
}

/// python mirror_ready_equippables (1316-1331): what has a state machine on the client.
pub fn ready_equippables(reply: &Reply) -> Vec<EquippableActor> {
    let conn = &*reply.conn;
    let book = &conn.authority.mirror;
    let [pistol, melee] = starting();
    let mut ready = Vec::new();
    if conn.bringup.sent(Stage::PistolInactiveState) && !book.starting_pistol_replaced && conn.correction_index >= 5 {
        ready.push(pistol.clone());
    }
    if conn.bringup.sent(Stage::MeleeInactiveState) {
        ready.push(melee.clone());
    }
    if conn.bringup.starting_equippable_index >= STARTING_EQUIPPABLES {
        let abilities = starting_abilities();
        for entry in abilities {
            ready.push(book.abilities.iter().find(|e| e.key == entry.key).unwrap_or(entry).clone());
        }
        ready.extend(book.abilities.iter().filter(|e| abilities.iter().all(|start| start.key != e.key)).cloned());
    }
    ready.extend(book.purchased.iter().cloned());
    ready
}

/// what the mirror pass saw that other systems act on after it.
#[derive(Debug, Default)]
pub struct MirrorPass {
    /// the smoke machine's prediction, when the smoke ability was read (cloudburst)
    pub smoke_state: Option<Option<String>>,
    /// classes whose firing seed advanced: one shot each (bots)
    pub shots: Vec<String>,
}

/// python 1602-1711. the cloudburst and bot hooks run inline in python; they
/// only touch their own channels, so running them right after keeps every
/// channel's order.
pub fn run_mirror(reply: &mut Reply, config: &RangeConfig, live: &mut dyn LiveClient) -> MirrorPass {
    let mut pass = MirrorPass::default();
    if !config.features.equippable_mirror || !live.attached() {
        return pass;
    }
    let ready = ready_equippables(reply);
    let polled = reply.conn.authority.mirror.polled;
    if ready.is_empty() || polled.is_some_and(|t| reply.now - t < MIRROR_POLL) {
        return pass;
    }
    reply.conn.authority.mirror.polled = Some(reply.now);
    let slots: Vec<u32> = ready.iter().map(|e| e.slot).collect();
    let machines = live.slot_machines(&slots).unwrap_or_else(|error| {
        reply.event.set("equippable_mirror_error", error);
        Vec::new()
    });
    let mut sent = Map::new();
    for equippable in &ready {
        let Some(machine) = machines.iter().find(|m| m.slot == equippable.slot) else {
            continue;
        };
        if machine.class_name != equippable.class_name {
            continue;
        }
        if config.features.ability_projectiles && equippable.class_name == SMOKE_CLASS {
            pass.smoke_state = Some(machine.predicted.clone());
        }
        // a shot consumes the firing state's networked seed (works with infinite ammo too)
        if config.features.bots && machine.is_current && !equippable.resources.is_empty() && let Some(seed) = live.firing_seed(machine.state_machine, &equippable.key) {
            let previous = reply.conn.authority.mirror.firing_seeds.insert(equippable.key.clone(), seed);
            if previous.is_some_and(|p| p != seed) {
                pass.shots.push(equippable.class_name.clone());
            }
        }
        // ammo first: a gun at AuthResourceAmount 0 never predicts FiringState
        if !equippable.resources.is_empty() {
            mirror_resources(reply, live, equippable, machine, &mut sent);
        }
        if !equippable.mirror_states {
            continue;
        }
        if !equippable.extra_machines.is_empty() {
            mirror_extra_machines(reply, live, equippable, machine, &mut sent);
        }
        let predicted = machine.predicted.as_deref();
        let state = reply.conn.authority.mirror.states.entry(equippable.key.clone()).or_default();
        let Some(target) = mirror_target(equippable, state, predicted, machine.is_current) else {
            continue;
        };
        let export = state.needs_export(&target);
        if send_state(reply, equippable.channel, build_auth_state_body(equippable, &target, export), export) {
            reply.conn.authority.mirror.states.get_mut(&equippable.key).expect("inserted above").confirm(&target);
            sent.insert(equippable.key.clone(), json!([target, predicted]));
        }
    }
    if !sent.is_empty() {
        reply.event.set("sent_equippable_auth_states", Value::Object(sent));
    }
    pass
}

fn mirror_resources(reply: &mut Reply, live: &mut dyn LiveClient, equippable: &EquippableActor, machine: &SlotMachine, sent: &mut Map<String, Value>) {
    let amounts = live.resource_amounts(machine.item, &equippable.resources).unwrap_or_else(|error| {
        reply.event.set("resource_mirror_error", error);
        Default::default()
    });
    for resource in &equippable.resources {
        let Some(reading) = amounts.get(&resource.name) else {
            continue;
        };
        let key = (equippable.key.clone(), resource.name.clone());
        let state = reply.conn.authority.mirror.resources.entry(key.clone()).or_default();
        let Some(amount) = resource_target(state, reading.predicted, reading.authoritative, reading.default_amount, reply.now, resource.refill_after) else {
            continue;
        };
        let export = state.auth.is_none();
        if send_state(reply, equippable.channel, build_resource_amount_body(equippable, resource, amount, export), export) {
            let now = reply.now;
            resource_confirmed(reply.conn.authority.mirror.resources.get_mut(&key).expect("inserted above"), amount, now);
            let resources = sent.entry("resources").or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(resources) = resources {
                resources.insert(format!("{}.{}", equippable.key, resource.name), json!(amount));
            }
        }
    }
}

fn mirror_extra_machines(reply: &mut Reply, live: &mut dyn LiveClient, equippable: &EquippableActor, machine: &SlotMachine, sent: &mut Map<String, Value>) {
    let live_machines = live.item_machines(machine.item).unwrap_or_else(|error| {
        reply.event.set("machine_mirror_error", error);
        Vec::new()
    });
    for extra in &equippable.extra_machines {
        // python {m.name: m}: the last machine with a name wins
        let predicted = live_machines.iter().rev().find(|m| m.name == extra.name).and_then(|m| m.predicted.as_deref());
        let key = format!("{}/{}", equippable.key, extra.name);
        let state = reply.conn.authority.mirror.states.entry(key.clone()).or_insert_with(|| MirrorState::starting_at(&extra.starting_state));
        let Some(target) = machine_mirror_target(state, predicted, &extra.state_guids) else {
            continue;
        };
        let export = state.needs_export(&target);
        if send_state(reply, equippable.channel, build_machine_state_body(equippable, extra, &target, export), export) {
            reply.conn.authority.mirror.states.get_mut(&key).expect("inserted above").confirm(&target);
            sent.insert(key, json!(target));
        }
    }
}

/// python send_extra_machines (1120-1128): open each extra machine and export
/// its starting state, then mirror it from there.
pub fn send_extra_machines(reply: &mut Reply, equippable: &EquippableActor) {
    for extra in &equippable.extra_machines {
        let opened = send_state(reply, equippable.channel, build_machine_open_body(equippable.actor_guid, extra.guid, &extra.name), true);
        let started = opened && send_state(reply, equippable.channel, build_machine_state_body(equippable, extra, &extra.starting_state, true), true);
        if started {
            reply.conn.authority.mirror.states.insert(format!("{}/{}", equippable.key, extra.name), MirrorState::extra_machine_sent(&extra.starting_state));
        }
    }
}

/// the names of the extra machines (python [extra.name for extra in extra_machines]).
pub fn extra_names(extra: &[ExtraStateMachine]) -> Value {
    json!(extra.iter().map(|e| e.name.as_str()).collect::<Vec<_>>())
}

/// one reliable mirror bunch; false (and the error logged) when it could not be sent.
fn send_state(reply: &mut Reply, channel: u32, body: aries_rep::RepResult<aries_bits::Bits>, export: bool) -> bool {
    let result = body.map_err(|e| e.to_string()).and_then(|body| reply.send_reliable(channel, &body, export, false, false).map_err(|e| e.to_string()));
    match result {
        Ok(_) => true,
        Err(error) => {
            reply.event.set("equippable_mirror_error", error);
            false
        }
    }
}
