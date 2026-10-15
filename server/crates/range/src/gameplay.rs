//! the range's moving parts: bots and the spray-wall dummy (python 2034-2062,
//! 969-1012), cloudburst flights (1016-1100) and the starting abilities'
//! activation jobs (2066-2078, 1130-1170). the world crate plans every bunch;
//! this sends them on the connection and keeps the python's event keys.
//! shots use live skeletal collision primitives from the contained client.
//! Historical root-only lag compensation is not mixed with current bone poses.

use serde_json::json;

use aries_interp::lagcomp::{HitShape, LagCompConfig, LagCompensation, Pose};
use aries_interp::value::Vec3;
use aries_world::abilities::{AbilityEquippable, AbilityJobs, AbilityOracle, AbilityPace, ItemMachine, SlotMachine as AbilitySlot};
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
            reply.register_dynamic_channel(send.channel);
        }
        reply.send_reliable(send.channel, &send.body, send.has_exports, send.is_open, send.is_close).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------------------------------------------------------------- bots

/// python bot_shots (969-1012): one shot per firing-seed change, along the
/// completed projectile's shot-time pose, against current collision geometry.
pub fn bot_shots(reply: &mut Reply, env: &mut Env, class_name: &str, camera: &crate::live::Camera) {
    if reply.conn.authority.bots.is_none() || !env.live.attached() {
        return;
    }
    reply.event.set("bot_shot_ray", json!({ "source": "projectile_creation", "origin": camera.location, "rotation": camera.rotation }));
    let now = reply.now;
    let system = reply.conn.authority.bots.as_mut().expect("checked above");
    let channels: Vec<u32> = system.field.bots.iter().filter(|bot| bot.opened && bot.alive)
        .chain(system.field.dummy.iter().filter(|bot| bot.opened)).map(|bot| bot.channel).collect();
    let hits = match env.live.target_hits(&channels, camera) {
        Ok(hits) => hits,
        Err(error) => {
            reply.event.set("bot_collision_error", error);
            return;
        }
    };
    // Do not combine current bone transforms with a rewound actor origin. The
    // feed is a bounded-age current pose; full skeletal rewind is not implemented.
    let shots = system.field.collision_shots(Some(class_name), 1, now, |bot| hits.get(&bot.channel).copied());
    if let Some(error) = &shots.error {
        reply.event.set("bot_shot_error", error.to_string());
    }
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

/// the command queue's {"spawn_bot": [x, y, z]} (python 1958-1971), in its
/// order: allocate (placement draws and yaw), coerce with float(), append the
/// bot, then open it. a failed coercion leaves the allocation spent and no
/// bot; a bad length leaves an unopened bot, as python's raise does.
pub fn spawn_bot_from_command(reply: &mut Reply, env: &Env, spot: &serde_json::Value) -> Result<(), String> {
    spawn_bot_at(reply, env, spot, None)
}

/// Unreal positions are centimetres. Keep pawn height and use view yaw only.
pub fn bot_spot_ahead(position: [f64; 3], yaw: f64) -> Result<[f64; 3], String> {
    if !yaw.is_finite() || position.iter().any(|value| !value.is_finite()) {
        return Err("Player pose is not finite".into());
    }
    let (sine, cosine) = yaw.to_radians().sin_cos();
    Ok([position[0] + 500.0 * cosine, position[1] + 500.0 * sine, position[2]])
}

pub fn spawn_bot_ahead(reply: &mut Reply, env: &mut Env) -> Result<(), String> {
    if !env.live.attached() || !reply.conn.bringup.sent(crate::bringup::Stage::GameplayStart) {
        return Err("Enter the playable Range first".into());
    }
    let track = &reply.conn.player;
    if reply.now - track.newest_snapshot_t > track.config.stale_after {
        return Err("Player position is stale; no bot spawned".into());
    }
    let position = track.value_at(track.newest_snapshot_t).ok_or("Player position unavailable")?.v.to_array();
    let camera = env.live.camera()?.ok_or("Player facing unavailable")?;
    let yaw = camera.rotation[1];
    let spot = bot_spot_ahead(position, yaw)?;
    spawn_bot_at(reply, env, &json!(spot), Some((yaw + 180.0).rem_euclid(360.0)))
}

fn spawn_bot_at(reply: &mut Reply, env: &Env, spot: &serde_json::Value, yaw: Option<f64>) -> Result<(), String> {
    let count = env.config.bot_count;
    let system = reply.conn.authority.bots.get_or_insert_with(|| BotSystem::new(count, &env.config.lagcomp));
    let index = system.field.bots.len() as i64;
    let mut bot = system.field.allocate(index).map_err(|e| e.to_string())?;
    let coords = crate::py::py_iter(spot)?.iter().map(crate::py::py_float).collect::<Result<Vec<f64>, _>>()?;
    let location = <[f64; 3]>::try_from(coords.as_slice()).ok();
    if let Some(location) = location {
        bot.location = location;
    }
    if let Some(yaw) = yaw {
        bot.yaw = yaw;
    }
    let channel = bot.channel;
    system.field.bots.push(bot);
    reply.register_dynamic_channel(channel);
    if location.is_none() {
        return Err(format!("ValueError: expected 3 values to unpack, got {}", coords.len()));
    }
    let system = reply.conn.authority.bots.as_mut().expect("made above");
    let bot = system.field.bots.last().expect("pushed above");
    let open = aries_world::bots::build_bot_open(bot).map_err(|e| e.to_string())?;
    let health = aries_world::bots::build_bot_health_body(bot, true).map_err(|e| e.to_string())?;
    let (actor_guid, location) = (bot.actor_guid, bot.location.map(py_round));
    reply.send_reliable(channel, &open, true, true, false).map_err(|e| e.to_string())?;
    reply.send_reliable(channel, &health, true, false, false).map_err(|e| e.to_string())?;
    if let Some(bot) = reply.conn.authority.bots.as_mut().and_then(|s| s.field.bots.last_mut()) {
        bot.opened = true;
    }
    reply.event.push("bot_spawned", json!([channel, actor_guid, location]));
    Ok(())
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
    let pacing = &env.config.pacing;
    if reply.conn.authority.abilities.is_none() {
        match AbilityJobs::with_stagger(now, pacing.delay(aries_world::abilities::JOB_STAGGER)) {
            Ok(jobs) => reply.conn.authority.abilities = Some(jobs),
            Err(error) => {
                reply.event.set("ability_register_error", error.to_string());
                return;
            }
        }
    }
    let fast = pacing.is_fast();
    let pace = if fast {
        use aries_world::abilities::{JOB_STEP_DELAY, REGISTER_MAX_TRIES, REGISTER_RETRY_DELAY};
        let reliable = &reply.conn.reliable;
        AbilityPace {
            step_delay: pacing.delay(JOB_STEP_DELAY),
            retry_delay: pacing.delay(REGISTER_RETRY_DELAY),
            max_tries: pacing.tries(REGISTER_RETRY_DELAY, REGISTER_MAX_TRIES),
            need_states: true,
            settled: &move |channel| !reliable.channel_pending(channel),
        }
    } else {
        AbilityPace::PYTHON
    };
    let jobs = reply.conn.authority.abilities.as_mut().expect("planned above");
    let step = jobs.step_paced(now, &mut AbilityReads { env }, &pace);
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
