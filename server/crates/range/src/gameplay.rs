//! the range's moving parts: bots and the spray-wall dummy (python 2034-2062,
//! 969-1012), cloudburst flights (1016-1100) and the starting abilities'
//! activation jobs (2066-2078, 1130-1170). the world crate plans every bunch;
//! this sends them on the connection and keeps the python's event keys.
//! shots are hit-tested through lag compensation (docs/port/10 section 4.8);
//! bots never move, so every rewind lands where the python tested.

use serde_json::json;

use aries_interp::lagcomp::{HitShape, LagCompConfig, LagCompensation, Pose};
use aries_interp::value::Vec3;
use aries_world::abilities::{AbilityEquippable, AbilityJobs, AbilityOracle, ItemMachine, SlotMachine as AbilitySlot};
use aries_world::bots::{Bot, BotField, Camera as ViewCamera, ChannelSend, ShotOutcome, py_round};
use aries_world::equippables::{EquippableActor, ExtraStateMachine, MirrorState, StateGuids};
use aries_world::projectiles::ProjectileField;

use crate::authority::Env;
use crate::bringup::{STARTING_EQUIPPABLES, Stage};
use crate::config::LagCompSettings;
use crate::mirror::starting_abilities;
use crate::send::Reply;

/// per-connection bots and their lag compensation history.
#[derive(Clone)]
pub struct BotSystem {
    pub field: BotField,
    pub lagcomp: LagCompensation,
}

impl BotSystem {
    pub fn new(count: usize, settings: &LagCompSettings) -> Self {
        let config = LagCompConfig { enabled: settings.enabled, max_unlag: settings.max_unlag, teleport_dist: settings.teleport_dist, claim_tolerance: settings.claim_tolerance };
        Self { field: BotField::new(count), lagcomp: LagCompensation::new(config) }
    }

    /// the opened bots and the dummy as lag compensation sees them now.
    pub fn poses(&self) -> Vec<(u32, Pose)> {
        let bots = self.field.bots.iter().filter(|b| b.opened).map(|b| (b.actor_guid, pose(b, 0.0)));
        let dummy = self.field.dummy.iter().filter(|d| d.opened).map(|d| (d.actor_guid, pose(d, aries_world::bots::DUMMY_MESH_LIFT)));
        bots.chain(dummy).collect()
    }
}

fn pose(bot: &Bot, lift: f64) -> Pose {
    let [x, y, z] = bot.location;
    Pose { origin: Vec3 { x, y, z }, yaw: bot.yaw, shape: HitShape::TrainingBot { lift }, alive: bot.alive, serial: 0 }
}

/// the world's camera from the oracle's (pitch, yaw, roll in degrees).
fn view_camera(camera: &crate::live::Camera) -> ViewCamera {
    let [pitch, yaw, roll] = camera.rotation;
    ViewCamera { location: camera.location, pitch, yaw, roll }
}

/// sends planned bunches in order; an open starts its channel at +0.
pub fn apply(reply: &mut Reply, sends: Vec<ChannelSend>) -> Result<(), String> {
    for send in sends {
        if send.is_open {
            reply.conn.register_dynamic_channel(send.channel);
        }
        reply.send_reliable(send.channel, &send.body, send.has_exports, send.is_open, send.is_close).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------------------------------------------------------------- bots

/// python bot_shots (969-1012): one shot per firing-seed change, along the
/// camera at poll time (05 quirk 13), against rewound bot positions.
pub fn bot_shots(reply: &mut Reply, env: &mut Env, class_name: &str) {
    if reply.conn.authority.bots.is_none() || !env.live.attached() {
        return;
    }
    let camera = match env.live.camera() {
        Ok(Some(camera)) => camera,
        Ok(None) => return,
        Err(error) => {
            reply.event.set("bot_camera_error", error);
            return;
        }
    };
    let now = reply.now;
    let settings = &env.config.lagcomp;
    let srtt = reply.conn.rtt.srtt_or_zero();
    let system = reply.conn.authority.bots.as_mut().expect("checked above");
    let target = system.lagcomp.target_time(now, srtt, settings.view_interp, None);
    let lagcomp = &system.lagcomp;
    let view = view_camera(&camera);
    let shots = system.field.shots_with(Some(class_name), 1, &view, now, |bot| {
        if !lagcomp.config.enabled {
            return Some(bot.location);
        }
        // pose() adds no lift; the dummy's is added after locating
        let current = pose(bot, 0.0);
        lagcomp.rewind(bot.actor_guid, &current, now, target.t).effective(&current).map(|p| [p.origin.x, p.origin.y, p.origin.z])
    });
    if let Some(error) = &shots.error {
        reply.event.set("bot_shot_error", error.to_string());
    }
    log::debug!(target: "lagcomp", "shot now={now:.4} target={:.4} ({:.1} ms) srtt {:.1} ms", target.t, (target.t - now) * 1000.0, srtt * 1000.0);
    if let Err(error) = apply(reply, shots.sends) {
        reply.event.set("bot_shot_error", error);
    }
    for outcome in shots.outcomes {
        match outcome {
            ShotOutcome::Dummy { region, damage, distance } => reply.event.push("dummy_hits", json!([region.name(), damage, py_round(distance)])),
            ShotOutcome::Missed => {
                let missed = reply.event.get("bot_shot_missed").and_then(|v| v.as_u64()).unwrap_or(0);
                reply.event.set("bot_shot_missed", missed + 1);
            }
            ShotOutcome::Bot { actor_guid, region, damage, life, distance, .. } => {
                reply.event.push("bot_hits", json!([actor_guid, region.name(), damage, py_round(life), py_round(distance)]))
            }
        }
    }
}

/// python 2034-2062: dummy once, retire bots dead 1.5 s, spawn one at a time.
pub fn bot_tick(reply: &mut Reply, env: &mut Env) {
    let count = env.config.bot_count;
    let system = reply.conn.authority.bots.get_or_insert_with(|| BotSystem::new(count, &env.config.lagcomp));
    if !system.field.tick_due(reply.now) {
        return;
    }
    // python raises out of the block mid-tick: what was planned before still goes out
    let tick = system.field.tick(reply.now);
    if let Some(error) = &tick.error {
        reply.event.set("bot_tick_error", error.to_string());
    }
    for actor_guid in &tick.retired {
        system.lagcomp.forget(*actor_guid);
    }
    if let Err(error) = apply(reply, tick.sends) {
        reply.event.set("bot_tick_error", error);
    }
    if let Some((channel, actor_guid)) = tick.dummy_spawned {
        reply.event.set("dummy_spawned", json!([channel, actor_guid]));
    }
    for actor_guid in tick.retired {
        reply.event.push("bot_retired", actor_guid);
    }
    for (channel, actor_guid, location) in tick.spawned {
        reply.event.push("bot_spawned", json!([channel, actor_guid, location]));
    }
}

/// the command queue's {"spawn_bot": [x, y, z]} (python 1958-1971): a bot at an
/// exact spot; the yaw keeps the random placement (05 quirk 12).
pub fn spawn_bot_at(reply: &mut Reply, env: &Env, location: [f64; 3]) -> Result<(), String> {
    let count = env.config.bot_count;
    let system = reply.conn.authority.bots.get_or_insert_with(|| BotSystem::new(count, &env.config.lagcomp));
    let tick = system.field.spawn_at(location);
    apply(reply, tick.sends)?;
    for (channel, actor_guid, location) in tick.spawned {
        reply.event.push("bot_spawned", json!([channel, actor_guid, location]));
    }
    match tick.error {
        Some(error) => Err(error.to_string()),
        None => Ok(()),
    }
}

// ---------------------------------------------------------------- cloudburst

/// python cloudburst_state (1016-1044), from the mirror's smoke read: on the
/// poll that enters a throw state, open the projectile from the camera.
pub fn smoke_state(reply: &mut Reply, env: &mut Env, predicted: Option<&str>) {
    let field = reply.conn.authority.projectiles.get_or_insert_with(ProjectileField::default);
    if !field.smoke_state(predicted) || !env.live.attached() {
        return;
    }
    let camera = match env.live.camera() {
        Ok(Some(camera)) => camera,
        Ok(None) => return,
        Err(error) => {
            reply.event.set("cloudburst_camera_error", error);
            return;
        }
    };
    let field = reply.conn.authority.projectiles.as_mut().expect("inserted above");
    match field.throw(&view_camera(&camera), reply.now) {
        Ok(open) => {
            let event = json!([open.channel, open.actor_guid, open.location, open.velocity, "A"]);
            match apply(reply, vec![open.send]) {
                Ok(()) => reply.event.set("cloudburst_open", event),
                Err(error) => reply.event.set("cloudburst_error", error),
            }
        }
        Err(error) => reply.event.set("cloudburst_error", error.to_string()),
    }
}

/// python tick_projectiles (1046-1100) every 50 ms once a field exists.
pub fn projectile_tick(reply: &mut Reply, env: &mut Env) {
    let now = reply.now;
    let Some(field) = reply.conn.authority.projectiles.as_mut().filter(|f| f.tick_due(now)) else {
        return;
    };
    let mut read_errors = Vec::new();
    let attached = env.live.attached();
    let live = &mut *env.live;
    let tick = field.tick(
        now,
        |channel| {
            if !attached {
                return None;
            }
            live.projectile(channel).unwrap_or_else(|error| {
                read_errors.push(error);
                None
            })
        },
        // python random.uniform(0.0, 360.0) on the unseeded global rng (05 quirk 8)
        || crate::random_unit() * 360.0,
    );
    if let Some(error) = read_errors.pop() {
        reply.event.set("cloudburst_read_error", error);
    }
    if let Some(error) = &tick.error {
        reply.event.set("cloudburst_error", error.to_string());
    }
    if let Err(error) = apply(reply, tick.sends) {
        reply.event.set("cloudburst_error", error);
    }
    // python keeps only the last zone of a pass
    if let Some(zone) = tick.zones.last() {
        reply.event.set("cloudburst_zone", json!([zone.channel, zone.zone_guid, zone.location, zone.reason.name()]));
    }
    for channel in tick.zones_closed {
        reply.event.push("cloudburst_zone_closed", channel);
    }
}

// ---------------------------------------------------------------- abilities

/// the ability jobs' reads (python shared EquippableStateReader; none when it is not open).
struct AbilityReads<'a, 'b> {
    env: &'a mut Env<'b>,
}

impl AbilityOracle for AbilityReads<'_, '_> {
    fn slot_machine(&mut self, slot: u32) -> Result<Option<AbilitySlot>, String> {
        if !self.env.live.attached() {
            return Ok(None);
        }
        let Some(machine) = self.env.live.slot_machines(&[slot])?.into_iter().find(|m| m.slot == slot) else {
            return Ok(None);
        };
        let state_names = self.env.live.state_names(machine.state_machine).unwrap_or_default();
        Ok(Some(AbilitySlot { item: machine.item, class_name: Some(machine.class_name), state_machine: machine.state_machine, state_names }))
    }

    fn item_machines(&mut self, item: u64) -> Result<Vec<ItemMachine>, String> {
        let machines = self.env.live.item_machines(item)?;
        Ok(machines.into_iter().map(|m| ItemMachine { name: m.name, address: m.address, starting: m.starting, state_names: m.state_names }).collect())
    }
}

/// python 2066-2078: once the knife chain is done, each starting ability gets
/// RemoteRole, its state machine and InactiveState, then joins the mirror.
pub fn ability_jobs(reply: &mut Reply, env: &mut Env) {
    let conn = &*reply.conn;
    if !conn.bringup.sent(Stage::MeleeInactiveState) || conn.bringup.starting_equippable_index < STARTING_EQUIPPABLES {
        return;
    }
    let now = reply.now;
    if reply.conn.authority.abilities.is_none() {
        match AbilityJobs::new(now) {
            Ok(jobs) => reply.conn.authority.abilities = Some(jobs),
            Err(error) => {
                reply.event.set("ability_register_error", error.to_string());
                return;
            }
        }
    }
    let jobs = reply.conn.authority.abilities.as_mut().expect("planned above");
    let step = jobs.step(now, &mut AbilityReads { env });
    if let Some(error) = &step.error {
        reply.event.set("ability_register_error", error.to_string());
    }
    if let Err(error) = apply(reply, step.sends) {
        reply.event.set("ability_register_error", error);
    }
    if let Some(error) = step.register_errors.last() {
        reply.event.set("ability_register_error", error.as_str());
    }
    if let Some(error) = step.extra_machine_errors.last() {
        reply.event.set("extra_machines_error", error.as_str());
    }
    if let Some(class) = step.gave_up.last() {
        reply.event.set("ability_register_gave_up", *class);
    }
    for ready in step.ready {
        let extra_names: Vec<&str> = ready.extra_machines.iter().map(|e| e.name.as_str()).collect();
        reply.event.push("ability_ready", json!([ready.class_name, ready.state_guids.len(), extra_names]));
        match mirror_entry(&ready) {
            Ok(equippable) => {
                // send_extra_machines leaves each extra machine confirmed at its starting state
                for extra in &equippable.extra_machines {
                    reply.conn.authority.mirror.states.insert(format!("{}/{}", equippable.key, extra.name), MirrorState::extra_machine_sent(&extra.starting_state));
                }
                reply.conn.authority.mirror.set_ability(equippable);
            }
            Err(error) => reply.event.set("ability_register_error", error),
        }
    }
}

/// a registered ability as the mirror holds it: keyed by class so the charge
/// state of its resource-only entry carries over (python 1161-1167).
fn mirror_entry(ready: &AbilityEquippable) -> Result<EquippableActor, String> {
    let state_guids: StateGuids = ready.state_guids.iter().map(|(name, guid)| (name.as_str(), *guid)).collect();
    let resources = starting_abilities().iter().find(|e| e.key == ready.class_name).map(|e| e.resources.clone()).unwrap_or_default();
    let extras = ready
        .extra_machines
        .iter()
        .map(|e| ExtraStateMachine { name: e.name.clone(), guid: e.guid, starting_state: e.starting_state.clone(), state_guids: e.state_guids.iter().map(|(n, g)| (n.as_str(), *g)).collect() })
        .collect();
    let equippable = EquippableActor::new(ready.key, ready.class_name, ready.channel, ready.actor_guid, ready.slot, ready.state_machine_guid, state_guids).map_err(|e| e.to_string())?;
    Ok(equippable.with_resources(resources).with_extra_machines(extras))
}
