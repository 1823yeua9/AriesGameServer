//! lag compensation, modeled on source's player lag compensation (reimplemented).
//!
//! every tick each tracked entity gets a record if its simulation time moved.
//! a shot is resolved against where targets were when the shooter saw them:
//! target time = now - (latency + client interpolation), clamped to max_unlag.
//! backtrack walks newest to oldest and gives up (uses the current state)
//! when it crosses a death or a teleport, since lerping across either is wrong.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

use crate::Lerp;
use crate::lerp::Rotator;

/// at most this far back (source sv_maxunlag).
pub const DEFAULT_MAX_UNLAG: f64 = 1.0;
/// a jump bigger than this between two records is a teleport, in uu (cm).
/// source uses 64 in; scaled to valorant's run speed per tick this is ~70 cm.
pub const DEFAULT_TELEPORT_DISTANCE: f64 = 100.0;
/// when the client's own fire time disagrees with our latency estimate by more
/// than this, trust the estimate (source: 200 ms).
pub const MAX_CLIENT_TIME_ERROR: f64 = 0.2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LagRecord {
    pub time: f64,
    pub alive: bool,
    pub origin: [f64; 3],
    pub rotation: Rotator,
    /// collision extents (capsule radius, half height)
    pub extent: [f64; 2],
}

impl LagRecord {
    fn blend(older: &Self, newer: &Self, frac: f64, time: f64) -> Self {
        LagRecord {
            time,
            alive: older.alive && newer.alive,
            origin: <[f64; 3]>::lerp(&older.origin, &newer.origin, frac),
            rotation: Rotator::lerp(&older.rotation, &newer.rotation, frac),
            extent: [f64::lerp(&older.extent[0], &newer.extent[0], frac), f64::lerp(&older.extent[1], &newer.extent[1], frac)],
        }
    }
}

#[derive(Debug, Clone)]
pub struct LagCompensation<K: Eq + Hash + Copy> {
    tracks: HashMap<K, VecDeque<LagRecord>>,
    pub max_unlag: f64,
    pub teleport_distance: f64,
}

impl<K: Eq + Hash + Copy> Default for LagCompensation<K> {
    fn default() -> Self {
        Self { tracks: HashMap::new(), max_unlag: DEFAULT_MAX_UNLAG, teleport_distance: DEFAULT_TELEPORT_DISTANCE }
    }
}

impl<K: Eq + Hash + Copy> LagCompensation<K> {
    /// records an entity's state for this tick; skipped when its time did not advance.
    pub fn record(&mut self, key: K, record: LagRecord) {
        let track = self.tracks.entry(key).or_default();
        if track.front().is_some_and(|head| head.time >= record.time) {
            return;
        }
        track.push_front(record);
    }

    /// drops records older than now - max_unlag; call once per tick.
    pub fn prune(&mut self, now: f64) {
        let dead_time = now - self.max_unlag;
        for track in self.tracks.values_mut() {
            while track.back().is_some_and(|tail| tail.time < dead_time) {
                track.pop_back();
            }
        }
    }

    /// forget an entity (destroyed).
    pub fn remove(&mut self, key: K) {
        self.tracks.remove(&key);
    }

    pub fn track(&self, key: K) -> Option<&VecDeque<LagRecord>> {
        self.tracks.get(&key)
    }

    /// the time to rewind targets to for a shot. latency is the one-way time
    /// from us to the shooter; interp is their render delay. fire_time is the
    /// shooter's own claim of when they fired (our clock), if we have one.
    pub fn target_time(&self, now: f64, latency: f64, interp: f64, fire_time: Option<f64>) -> f64 {
        let correct = (latency + interp).clamp(0.0, self.max_unlag);
        let estimate = now - correct;
        match fire_time {
            Some(claimed) => {
                let target = claimed - interp;
                if (target - estimate).abs() > MAX_CLIENT_TIME_ERROR { estimate } else { target }
            }
            None => estimate,
        }
    }

    /// where `key` was at `target_time`. none when there is no usable history
    /// (no records, or a death/teleport between now and then): use the
    /// current state in that case, like source does.
    pub fn backtrack(&self, key: K, target_time: f64) -> Option<LagRecord> {
        let track = self.tracks.get(&key)?;
        let teleport_squared = self.teleport_distance * self.teleport_distance;
        let mut newer: Option<&LagRecord> = None;
        for record in track {
            if !record.alive {
                return None;
            }
            if let Some(next) = newer {
                let dx = record.origin[0] - next.origin[0];
                let dy = record.origin[1] - next.origin[1];
                if dx * dx + dy * dy > teleport_squared {
                    return None;
                }
            }
            if record.time <= target_time {
                return Some(match newer {
                    Some(next) if record.time < target_time && record.time < next.time => {
                        let frac = (target_time - record.time) / (next.time - record.time);
                        LagRecord::blend(record, next, frac, target_time)
                    }
                    _ => *record,
                });
            }
            newer = Some(record);
        }
        // older than all history: the oldest record is the best we have
        track.back().copied()
    }
}
