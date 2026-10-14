//! lag compensation: resolve a shot against where targets were when the
//! shooter saw them. source CLagCompensationManager, reimplemented; we never
//! mutate entities, a rewound pose is just a value for the analytic hit test.
//! docs/port/10_interp_source_sdk.md 3.10-3.14 and 4.7.

use std::collections::{HashMap, VecDeque};

use crate::time::{ticks_to_time, time_to_ticks};
use crate::value::{Interp, Vec3, wrap180};

/// actor net guid (bots are per connection).
pub type EntityKey = u32;

/// 1 s at 128 hz = 128 records, with 2x headroom.
pub const LAG_RING: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HitShape {
    /// range bot capsule regions; lift 0 for bots, 4 for the spray-wall dummy
    TrainingBot { lift: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub origin: Vec3,
    pub yaw: f64,
    pub shape: HitShape,
    pub alive: bool,
    /// bumped by server code on any jump (respawn, teleport); a rewind never crosses it
    pub serial: u16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LagRecord {
    pub tick: u64,
    pub t: f64,
    pub pose: Pose,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LagCompConfig {
    pub enabled: bool,
    /// source sv_maxunlag
    pub max_unlag: f64,
    /// source sv_lagcompensation_teleport_dist, 2d, per record
    pub teleport_dist: f64,
    /// source StartLagCompensation: distrust a client fire time this far off
    pub claim_tolerance: f64,
}

impl Default for LagCompConfig {
    fn default() -> Self {
        Self { enabled: true, max_unlag: 1.0, teleport_dist: 64.0, claim_tolerance: 0.2 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetSource {
    Claimed,
    Latency,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub t: f64,
    pub source: TargetSource,
    pub lerp: f64,
    pub correct: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lost {
    DeadOnPath,
    TeleportDistance,
    SerialChanged,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rewind {
    Exact { pose: Pose, t: f64 },
    Interpolated { pose: Pose, older_t: f64, newer_t: f64, frac: f64 },
    /// target older than all history: the oldest pose
    Clamped { pose: Pose, t: f64 },
    Unavailable(Lost),
}

impl Rewind {
    /// the pose to hit-test; when history is unusable, the current pose if alive.
    pub fn effective(&self, current: &Pose) -> Option<Pose> {
        match self {
            Rewind::Exact { pose, .. } | Rewind::Interpolated { pose, .. } | Rewind::Clamped { pose, .. } => Some(*pose),
            Rewind::Unavailable(_) => current.alive.then_some(*current),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct LagCompensation {
    tracks: HashMap<EntityKey, VecDeque<LagRecord>>,
    pub config: LagCompConfig,
}

impl LagCompensation {
    pub fn new(config: LagCompConfig) -> Self {
        Self { tracks: HashMap::new(), config }
    }

    /// once per tick after all systems: one record per compensated entity.
    pub fn record(&mut self, tick: u64, now: f64, poses: impl IntoIterator<Item = (EntityKey, Pose)>) {
        for (key, pose) in poses {
            let track = self.tracks.entry(key).or_default();
            if track.front().is_some_and(|head| head.t >= now) {
                continue;
            }
            track.push_front(LagRecord { tick, t: now, pose });
            if track.len() > LAG_RING {
                track.pop_back();
            }
        }
    }

    /// bot retired / channel closed.
    pub fn forget(&mut self, key: EntityKey) {
        self.tracks.remove(&key);
    }

    /// drops records older than now - max_unlag (f64, no truncation unlike source).
    pub fn prune(&mut self, now: f64) {
        let dead = now - self.config.max_unlag;
        for track in self.tracks.values_mut() {
            while track.back().is_some_and(|r| r.t < dead) {
                track.pop_back();
            }
        }
    }

    pub fn track(&self, key: EntityKey) -> Option<&VecDeque<LagRecord>> {
        self.tracks.get(&key)
    }

    /// source StartLagCompensation in f64: rewind by rtt + the shooter's view
    /// delay (rounded to ticks, no +1), unless a believable fire time is claimed.
    pub fn target_time(&self, now: f64, rtt: f64, view_interp: f64, claimed_fire: Option<f64>) -> Target {
        let lerp = ticks_to_time(time_to_ticks(view_interp));
        let correct = (rtt + lerp).clamp(0.0, self.config.max_unlag);
        let (mut t, mut source) = (now - correct, TargetSource::Latency);
        if let Some(fire) = claimed_fire {
            let claimed = fire - lerp;
            if (correct - (now - claimed)).abs() <= self.config.claim_tolerance {
                (t, source) = (claimed, TargetSource::Claimed);
            }
        }
        // explicit; source relied on pruning
        t = t.clamp(now - self.config.max_unlag, now);
        Target { t, source, lerp, correct }
    }

    /// where `key` was at `target`. the current pose at `now` is the virtual
    /// newest record, so a target between the last record and now interpolates.
    pub fn rewind(&self, key: EntityKey, current: &Pose, now: f64, target: f64) -> Rewind {
        let records = self.tracks.get(&key);
        let chain = std::iter::once(LagRecord { tick: u64::MAX, t: now, pose: *current }).chain(records.into_iter().flatten().copied());
        let teleport_squared = self.config.teleport_dist * self.config.teleport_dist;
        let mut prev_origin = current.origin;
        let mut newer: Option<LagRecord> = None;
        let mut last: Option<LagRecord> = None;
        for record in chain {
            if !record.pose.alive {
                return Rewind::Unavailable(Lost::DeadOnPath);
            }
            if record.pose.serial != current.serial {
                return Rewind::Unavailable(Lost::SerialChanged);
            }
            let dx = record.pose.origin.x - prev_origin.x;
            let dy = record.pose.origin.y - prev_origin.y;
            if dx * dx + dy * dy > teleport_squared {
                return Rewind::Unavailable(Lost::TeleportDistance);
            }
            if record.t <= target {
                return match newer {
                    Some(next) if record.t < target && record.t < next.t => {
                        let frac = (target - record.t) / (next.t - record.t);
                        let pose = Pose {
                            origin: Vec3::lerp(record.pose.origin, next.pose.origin, frac),
                            yaw: wrap180(record.pose.yaw + wrap180(next.pose.yaw - record.pose.yaw) * frac),
                            shape: record.pose.shape,
                            alive: true,
                            serial: current.serial,
                        };
                        Rewind::Interpolated { pose, older_t: record.t, newer_t: next.t, frac }
                    }
                    _ => Rewind::Exact { pose: record.pose, t: record.t },
                };
            }
            prev_origin = record.pose.origin;
            newer = Some(record);
            last = Some(record);
        }
        let oldest = last.expect("the chain always has the current pose");
        Rewind::Clamped { pose: oldest.pose, t: oldest.t }
    }
}
