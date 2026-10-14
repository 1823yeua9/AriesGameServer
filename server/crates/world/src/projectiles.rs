//! server-spawned cloudburst, stage A (range_projectiles.py, spec 08 section 5):
//! the per-connection allocator, the throw transform, the projectile and smoke
//! zone opens, and where a flight stops (client snapshot, timeout or a
//! ballistic fallback), plus the throw trigger and the flight tick
//! (ruh:1016-1044, 1046-1100) as state changes and the bunches to send.
//!
//! stage B (bending, teleport, stop rpc, ignore-pawn open) is not used by any
//! launcher and is not ported (spec 08 0.1).
//!
//! rp = range_projectiles.py, ruh = range_udp_handshake.py.

use aries_bits::{BitWriter, Bits, append_quantized_vector, append_quantized_vector10, append_rotator_bytes, compress_short};
use aries_rep::block::{Spawn, Target, append_export_header, append_rep_block, append_stable_child_entry, append_static_export_entry, dynamic_actor_reference};
use aries_rep::props::empty_rep;
use aries_rep::{RepError, RepResult};

use crate::bots::{ChannelSend, py_radians, py_round};

/// rp:33-34
pub const PAWN_GUID: u32 = 10;
pub const LEVEL_GUID: u32 = 5;
/// rp:36-39: the smoke ability, its projectile, and the predicted states that mean "thrown".
pub const SMOKE_ABILITY_CLASS: &str = "Ability_Wushu_4_Smoke_C";
pub const SMOKE_PROJECTILE_CLASS: &str = "Projectile_Wushu_4_Smoke_C";
pub const SMOKE_THROW_STATES: [&str; 3] = ["ProjectileThrowState", "CommitTime", "AirBendingState"];
/// rp:41-47
pub const SMOKE_PROJECTILE: (&str, &str) = ("Default__Projectile_Wushu_4_Smoke_C", "/Game/Characters/Wushu/S0/Ability_4/Projectile_Wushu_4_Smoke");
pub const SMOKE_ZONE: (&str, &str) = ("Default__GameObject_Wushu_4_SmokeZone_C", "/Game/Characters/Wushu/S0/Ability_4/GameObject_Wushu_4_SmokeZone");
pub const SMOKE_PROJECTILE_ARCHETYPE_GUID: u32 = 610001;
pub const SMOKE_PROJECTILE_PACKAGE_GUID: u32 = 610003;
pub const SMOKE_ZONE_ARCHETYPE_GUID: u32 = 610005;
pub const SMOKE_ZONE_PACKAGE_GUID: u32 = 610007;
pub const MOVEMENT_COMPONENT_NAME: &str = "FiniteSpeedMovement";
/// rp:50: AActor handles 11 ReplicatedMovement, 12 Owner, 14 Instigator.
pub const REPLICATED_MOVEMENT_HANDLE: u32 = 11;
pub const OWNER_HANDLE: u32 = 12;
pub const INSTIGATOR_HANDLE: u32 = 14;
/// rp:60-67
pub const FIRST_PROJECTILE_CHANNEL: u32 = 10000;
pub const LAST_PROJECTILE_CHANNEL: u32 = 32766;
pub const FIRST_PROJECTILE_GUID: u32 = 610100;
pub const STAGE_A_SPAWN_FORWARD: f64 = 60.0;
pub const SMOKE_NORMAL_OFFSET: f64 = 65.0;
pub const STOP_AFTER_TIMEOUT: f64 = 2.0;
pub const UNSEEN_FALLBACK_AFTER: f64 = 0.3;
pub const SMOKE_ZONE_LIFETIME: f64 = 3.2;
/// rp:268: throw_transform defaults (ThrowOffset 0 for cloudburst).
pub const THROW_UPWARD_ARC: f64 = 8.0;
pub const THROW_UPWARD_SHIFT: f64 = 8.0;
pub const THROW_SPEED: f64 = 2900.0;
pub const THROW_MAX_PITCH: f64 = 89.9;
/// rp:285-286: ballistic_fallback defaults.
pub const FALLBACK_FLOOR_Z: f64 = 1253.8;
pub const FALLBACK_GRAVITY_Z: f64 = -2500.0 * 0.45;
pub const FALLBACK_STEP: f64 = 0.005;
/// ruh:2027: the flight tick runs at most this often.
pub const PROJECTILE_TICK_INTERVAL: f64 = 0.05;

fn invalid(message: &str) -> RepError {
    RepError::Invalid(message.into())
}

// ---------------------------------------------------------------- allocation and flights

/// rp:70-86 (stage A fields).
#[derive(Debug, Clone, PartialEq)]
pub struct Flight {
    pub channel: u32,
    pub actor_guid: u32,
    pub movement_guid: u32,
    pub opened: f64,
    pub location: [f64; 3],
    pub velocity: [f64; 3],
    pub pitch: f64,
    pub yaw: f64,
    pub state: FlightState,
    pub seen: bool,
    pub last_location: Option<[f64; 3]>,
    pub close_at: Option<f64>,
    pub zone_channel: Option<u32>,
    pub zone_close_at: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlightState {
    Flying,
    Stopped,
    Closed,
}

impl Flight {
    pub fn new(channel: u32, actor_guid: u32, movement_guid: u32, opened: f64, transform: &ThrowTransform) -> Self {
        Self {
            channel,
            actor_guid,
            movement_guid,
            opened,
            location: transform.location,
            velocity: transform.velocity,
            pitch: transform.pitch,
            yaw: transform.yaw,
            state: FlightState::Flying,
            seen: false,
            last_location: None,
            close_at: None,
            zone_channel: None,
            zone_close_at: None,
        }
    }
}

/// rp:89-107: one channel per actor, even guids, never reused (per connection),
/// plus the connection's last flight tick (ruh projectile_ticks).
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectileField {
    pub next_channel: u32,
    pub next_guid: u32,
    pub flights: Vec<Flight>,
    pub previous_smoke_state: Option<String>,
    /// ruh projectile_ticks.get(source, 0)
    pub ticked_at: f64,
}

impl Default for ProjectileField {
    fn default() -> Self {
        Self {
            next_channel: FIRST_PROJECTILE_CHANNEL,
            next_guid: FIRST_PROJECTILE_GUID,
            flights: Vec::new(),
            previous_smoke_state: None,
            ticked_at: 0.0,
        }
    }
}

impl ProjectileField {
    /// rp:97-102
    pub fn channel(&mut self) -> RepResult<u32> {
        let channel = self.next_channel;
        if channel > LAST_PROJECTILE_CHANNEL {
            return Err(invalid("out of channel indices"));
        }
        self.next_channel += 1;
        Ok(channel)
    }

    /// rp:104-107
    pub fn guid(&mut self) -> u32 {
        let guid = self.next_guid;
        self.next_guid += 2;
        guid
    }
}

/// rp:110-112: true on the transition into the throw states.
pub fn smoke_thrown(previous: Option<&str>, current: Option<&str>) -> bool {
    let thrown = |state: Option<&str>| state.is_some_and(|state| SMOKE_THROW_STATES.contains(&state));
    thrown(current) && !thrown(previous)
}

// ---------------------------------------------------------------- transforms

/// rp:267-278 output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThrowTransform {
    pub location: [f64; 3],
    pub pitch: f64,
    pub yaw: f64,
    pub velocity: [f64; 3],
}

/// rp:267-278 with stage A's forward 60 (ruh:1036): pitch wrapped to [-180, 180)
/// with python's floored modulo, lifted by the arc, spawn shifted down and forward.
pub fn throw_transform(view_location: [f64; 3], view_pitch: f64, view_yaw: f64) -> ThrowTransform {
    let view_pitch = py_mod(view_pitch + 180.0, 360.0) - 180.0;
    let scale = 1.0 - 0.25 * view_pitch.abs().min(90.0) / 90.0;
    let pitch = (view_pitch + scale * THROW_UPWARD_ARC).min(THROW_MAX_PITCH);
    let (p, y) = (py_radians(pitch), py_radians(view_yaw));
    let direction = [p.cos() * y.cos(), p.cos() * y.sin(), p.sin()];
    let forward = STAGE_A_SPAWN_FORWARD;
    let location = [
        view_location[0] + direction[0] * forward,
        view_location[1] + direction[1] * forward,
        view_location[2] - scale * THROW_UPWARD_SHIFT + direction[2] * forward,
    ];
    ThrowTransform { location, pitch, yaw: view_yaw, velocity: direction.map(|c| c * THROW_SPEED) }
}

/// python float % for a positive divisor (cpython float_rem: fmod, then shift
/// into the divisor's sign; a zero result takes the divisor's sign).
fn py_mod(value: f64, divisor: f64) -> f64 {
    let mut rem = value % divisor;
    if rem != 0.0 {
        if (divisor < 0.0) != (rem < 0.0) {
            rem += divisor;
        }
    } else {
        rem = 0.0f64.copysign(divisor);
    }
    rem
}

/// rp:281-282
pub fn smoke_location_from_hit(hit_location: [f64; 3], hit_normal: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| hit_location[i] + hit_normal[i] * SMOKE_NORMAL_OFFSET)
}

/// rp:285-295: step a dropped smoke at 5 ms (accumulated, not i * step) until
/// it reaches the firing-line floor, at most 2 s.
pub fn ballistic_fallback(location: [f64; 3], velocity: [f64; 3]) -> [f64; 3] {
    let mut elapsed = 0.0f64;
    let mut point = location;
    while elapsed < STOP_AFTER_TIMEOUT {
        elapsed += FALLBACK_STEP;
        point = [
            location[0] + velocity[0] * elapsed,
            location[1] + velocity[1] * elapsed,
            location[2] + velocity[2] * elapsed + 0.5 * FALLBACK_GRAVITY_Z * elapsed * elapsed,
        ];
        if point[2] <= FALLBACK_FLOOR_Z {
            return [point[0], point[1], FALLBACK_FLOOR_Z + SMOKE_NORMAL_OFFSET];
        }
    }
    point
}

/// rp:298-305: the client's simulated projectile (read-only, from the oracle).
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectileSnapshot {
    pub class_name: String,
    /// root component location; none when the actor has no root
    pub location: Option<[f64; 3]>,
    pub collisions: u32,
    pub hit_location: [f64; 3],
    pub hit_normal: [f64; 3],
}

/// why a flight stopped (the cloudburst_zone event's reason).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Hit,
    Timeout,
    Lost,
    Fallback,
}

impl StopReason {
    pub fn name(self) -> &'static str {
        match self {
            StopReason::Hit => "hit",
            StopReason::Timeout => "timeout",
            StopReason::Lost => "lost",
            StopReason::Fallback => "fallback",
        }
    }
}

/// rp:346-360: where the flight stops now, or none to keep flying.
pub fn flight_stop_point(flight: &mut Flight, snapshot: Option<&ProjectileSnapshot>, now: f64) -> Option<([f64; 3], StopReason)> {
    if let Some(snapshot) = snapshot
        && snapshot.class_name == SMOKE_PROJECTILE_CLASS
        && let Some(location) = snapshot.location
    {
        flight.seen = true;
        flight.last_location = Some(location);
        if snapshot.collisions >= 1 {
            return Some((smoke_location_from_hit(snapshot.hit_location, snapshot.hit_normal), StopReason::Hit));
        }
        if now - flight.opened >= STOP_AFTER_TIMEOUT {
            return Some((location, StopReason::Timeout));
        }
        return None;
    }
    if flight.seen {
        return flight.last_location.map(|location| (location, StopReason::Lost));
    }
    if now - flight.opened >= UNSEEN_FALLBACK_AFTER {
        return Some((ballistic_fallback(flight.location, flight.velocity), StopReason::Fallback));
    }
    None
}

// ---------------------------------------------------------------- bodies

/// rp:137-142: FRepMovement::NetSerialize (sub_7FF739016C70), quantization 0/0/0.
pub fn append_rep_movement(w: &mut BitWriter, location: [f64; 3], pitch: f64, yaw: f64, velocity: [f64; 3]) {
    w.append_bits(0, 4); // bSimulatedPhysicSleep, bRepPhysics, bRepServerFrame, bRepServerHandle
    append_quantized_vector(w, location, 1.0);
    append_rotator_bytes(w, pitch, yaw, 0.0);
    append_quantized_vector(w, velocity, 1.0);
}

/// rp:150-161: [ReplicatedMovement] Owner = Instigator = the pawn.
fn owner_instigator_rep(movement: Option<&ThrowTransform>) -> RepResult<Bits> {
    let mut rep = BitWriter::new();
    rep.append_bits(0, 1);
    if let Some(movement) = movement {
        rep.append_packed_uint(REPLICATED_MOVEMENT_HANDLE as u64);
        append_rep_movement(&mut rep, movement.location, movement.pitch, movement.yaw, movement.velocity);
    }
    rep.append_packed_uint(OWNER_HANDLE as u64);
    rep.append_packed_uint(PAWN_GUID as u64);
    rep.append_packed_uint(INSTIGATOR_HANDLE as u64);
    rep.append_packed_uint(PAWN_GUID as u64);
    rep.append_packed_uint(0);
    Ok(rep.finish()?)
}

/// rp:164-179: the dynamic actor header with pitch and yaw (roll 0).
fn append_spawn_header(w: &mut BitWriter, actor_guid: u32, location: [f64; 3], pitch: f64, yaw: f64) {
    w.append_packed_uint(actor_guid as u64);
    w.append_packed_uint(SMOKE_PROJECTILE_ARCHETYPE_GUID as u64);
    w.append_packed_uint(LEVEL_GUID as u64);
    w.append_bits(1, 1); // location present
    w.append_bits(1, 1); // quantized format
    append_quantized_vector10(w, location);
    let shorts = [compress_short(pitch), compress_short(yaw), 0];
    let any = shorts.iter().any(|&s| s != 0);
    w.append_bool(any);
    if any {
        // SerializeCompressedShort: pitch, yaw, roll
        for value in shorts {
            w.append_bool(value != 0);
            if value != 0 {
                w.append_bits(value as u64, 16);
            }
        }
    }
    w.append_bits(0, 2); // default scale, no velocity
}

/// rp:182-222, the stage A production call (ruh:1039-1041: movement guid set,
/// ignore_pawn off): archetype + FiniteSpeedMovement exports, spawn header,
/// actor block (movement, owner, instigator) and the empty movement block.
pub fn smoke_projectile_open_body(actor_guid: u32, movement_guid: u32, throw: &ThrowTransform) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    append_export_header(&mut w, 2);
    append_static_export_entry(&mut w, SMOKE_PROJECTILE.0, SMOKE_PROJECTILE.1, SMOKE_PROJECTILE_ARCHETYPE_GUID, SMOKE_PROJECTILE_PACKAGE_GUID);
    // outer: guid only, the actor is spawned by this bunch
    append_stable_child_entry(&mut w, movement_guid, actor_guid, MOVEMENT_COMPONENT_NAME);
    append_spawn_header(&mut w, actor_guid, throw.location, throw.pitch, throw.yaw);
    append_rep_block(&mut w, Target::Actor, &owner_instigator_rep(Some(throw))?, actor_guid)?;
    append_rep_block(&mut w, Target::Subobject(movement_guid), &empty_rep(), actor_guid)?;
    let body = w.finish()?;
    if body.bit_count >= 1 << 13 {
        return Err(invalid("projectile open exceeds the bunch size limit"));
    }
    Ok(body)
}

/// rp:253-264: the zone actor at the stop point (no ReplicatedMovement: it has
/// bReplicateMovement 0) with Owner/Instigator. yaw is any angle (production
/// draws it from the unseeded global random, ruh:1087).
pub fn smoke_zone_open_body(zone_guid: u32, location: [f64; 3], yaw: f64) -> RepResult<Bits> {
    let reference = dynamic_actor_reference(SMOKE_ZONE, zone_guid, SMOKE_ZONE_ARCHETYPE_GUID, SMOKE_ZONE_PACKAGE_GUID, LEVEL_GUID, Some(Spawn { location, yaw }))?;
    let mut w = BitWriter::new();
    w.append_bits_value(&reference);
    append_rep_block(&mut w, Target::Actor, &owner_instigator_rep(None)?, zone_guid)?;
    Ok(w.finish()?)
}

// ---------------------------------------------------------------- the per-packet systems

/// a throw's open (ruh:1037-1044); the event cloudburst_open is
/// [channel, actor, location, velocity (both rounded half-even), "A"].
#[derive(Debug, Clone, PartialEq)]
pub struct CloudburstOpen {
    pub send: ChannelSend,
    pub channel: u32,
    pub actor_guid: u32,
    pub location: [i64; 3],
    pub velocity: [i64; 3],
}

/// a smoke zone opened at a stop (event cloudburst_zone: [channel, zone, location, reason]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneOpened {
    pub channel: u32,
    pub zone_guid: u32,
    pub location: [i64; 3],
    pub reason: StopReason,
}

/// what one flight tick did (ruh:1059-1100). python keeps only the last zone
/// in its cloudburst_zone event; every one is listed here.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProjectileTick {
    pub sends: Vec<ChannelSend>,
    pub zones: Vec<ZoneOpened>,
    /// event cloudburst_zone_closed: zone channels
    pub zones_closed: Vec<u32>,
}

impl ProjectileField {
    /// ruh:1022-1025: remember the smoke ability's predicted state; true on the
    /// poll where it enters a throw state (then read the camera and throw).
    pub fn smoke_state(&mut self, predicted: Option<&str>) -> bool {
        let previous = std::mem::replace(&mut self.previous_smoke_state, predicted.map(str::to_string));
        smoke_thrown(previous.as_deref(), predicted)
    }

    /// ruh:1033-1044: spawn the projectile from the camera view on a fresh
    /// channel (its open is sequence +0).
    pub fn throw(&mut self, view_location: [f64; 3], view_pitch: f64, view_yaw: f64, now: f64) -> RepResult<CloudburstOpen> {
        let throw = throw_transform(view_location, view_pitch, view_yaw);
        let channel = self.channel()?;
        let actor_guid = self.guid();
        let movement_guid = self.guid();
        let body = smoke_projectile_open_body(actor_guid, movement_guid, &throw)?;
        self.flights.push(Flight::new(channel, actor_guid, movement_guid, now, &throw));
        Ok(CloudburstOpen {
            send: ChannelSend::open(channel, body),
            channel,
            actor_guid,
            location: throw.location.map(py_round),
            velocity: throw.velocity.map(py_round),
        })
    }

    /// ruh:2026-2027: the 0.05 s gate (the caller adds ability_projectiles).
    pub fn tick_due(&self, now: f64) -> bool {
        now - self.ticked_at >= PROJECTILE_TICK_INTERVAL
    }

    /// ruh:1059-1100, stage A: a flying flight reads its client actor
    /// (`read(channel)`, none on a failed read) and, once it stops, opens the
    /// zone there (yaw from `zone_yaw`) and closes the projectile channel in the
    /// same tick; the zone closes after 3.2 s; a done flight is dropped.
    pub fn tick(&mut self, now: f64, mut read: impl FnMut(u32) -> Option<ProjectileSnapshot>, mut zone_yaw: impl FnMut() -> f64) -> RepResult<ProjectileTick> {
        self.ticked_at = now;
        let mut tick = ProjectileTick::default();
        let mut index = 0;
        while index < self.flights.len() {
            if self.flights[index].state == FlightState::Flying {
                let snapshot = read(self.flights[index].channel);
                let Some((location, reason)) = flight_stop_point(&mut self.flights[index], snapshot.as_ref(), now) else {
                    index += 1;
                    continue;
                };
                self.flights[index].state = FlightState::Stopped;
                self.flights[index].close_at = Some(now);
                let zone_channel = self.channel()?;
                let zone_guid = self.guid();
                let flight = &mut self.flights[index];
                flight.zone_channel = Some(zone_channel);
                tick.sends.push(ChannelSend::open(zone_channel, smoke_zone_open_body(zone_guid, location, zone_yaw())?));
                flight.zone_close_at = Some(now + SMOKE_ZONE_LIFETIME);
                tick.zones.push(ZoneOpened { channel: zone_channel, zone_guid, location: location.map(py_round), reason });
            }
            let flight = &mut self.flights[index];
            if flight.close_at.is_some_and(|close_at| now >= close_at) {
                tick.sends.push(ChannelSend::close(flight.channel));
                flight.close_at = None;
                flight.state = FlightState::Closed;
            }
            if let (Some(close_at), Some(zone_channel)) = (flight.zone_close_at, flight.zone_channel)
                && now >= close_at
            {
                tick.sends.push(ChannelSend::close(zone_channel));
                flight.zone_close_at = None;
                tick.zones_closed.push(zone_channel);
            }
            if flight.state == FlightState::Closed && flight.zone_close_at.is_none() {
                self.flights.remove(index);
            } else {
                index += 1;
            }
        }
        Ok(tick)
    }
}
