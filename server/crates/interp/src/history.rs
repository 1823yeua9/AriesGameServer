//! time-stamped value history with interpolation and clamped extrapolation,
//! modeled on the source engine's interpolated vars (reimplemented, not copied).
//!
//! samples are kept newest first. sample(t) picks the two samples around t
//! (older <= t < newer); with a third, older sample it uses a cubic hermite
//! whose incoming tangent comes from that sample, renormalized when the
//! sample spacing is uneven. past the newest sample it extrapolates from the
//! last velocity, at most `max_extrapolation` seconds.

use std::collections::VecDeque;

use crate::Lerp;

/// how far past the newest sample we are willing to guess (source: cl_extrapolate_amount).
pub const DEFAULT_MAX_EXTRAPOLATION: f64 = 0.25;
/// samples closer than this are treated as the same instant.
const TIME_EPSILON: f64 = 0.0001;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample<T> {
    pub time: f64,
    pub value: T,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Linear,
    Hermite,
}

#[derive(Debug, Clone)]
pub struct History<T> {
    samples: VecDeque<Sample<T>>,
    /// how much history to keep, in seconds behind the newest sample
    pub keep: f64,
    pub mode: Mode,
    pub max_extrapolation: f64,
}

impl<T: Lerp + Copy> History<T> {
    pub fn new(keep: f64, mode: Mode) -> Self {
        Self { samples: VecDeque::new(), keep, mode, max_extrapolation: DEFAULT_MAX_EXTRAPOLATION }
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    pub fn newest(&self) -> Option<&Sample<T>> {
        self.samples.front()
    }

    pub fn samples(&self) -> impl Iterator<Item = &Sample<T>> {
        self.samples.iter()
    }

    /// adds a sample. anything newer than it is dropped (a time step back means
    /// a reset), then old samples past `keep` are pruned.
    pub fn push(&mut self, time: f64, value: T) {
        while self.samples.front().is_some_and(|s| s.time >= time) {
            self.samples.pop_front();
        }
        self.samples.push_front(Sample { time, value });
        let oldest_kept = time - self.keep;
        // keep one sample older than the window so the window edge can interpolate
        while self.samples.len() > 2 && self.samples[self.samples.len() - 2].time < oldest_kept {
            self.samples.pop_back();
        }
    }

    /// the value at `time`, interpolated, held, or extrapolated.
    pub fn sample(&self, time: f64) -> Option<T> {
        let newest = *self.samples.front()?;
        if time >= newest.time {
            return Some(self.extrapolate(time));
        }
        // newest first: find the first sample at or before `time`
        let older_index = self.samples.iter().position(|s| s.time <= time);
        let Some(older_index) = older_index else {
            // before all history: hold the oldest value
            return self.samples.back().map(|s| s.value);
        };
        let older = self.samples[older_index];
        let newer = self.samples[older_index - 1];
        let span = newer.time - older.time;
        if span <= TIME_EPSILON {
            return Some(newer.value);
        }
        let frac = ((time - older.time) / span).min(2.0);
        if self.mode == Mode::Hermite {
            if let Some(&oldest) = self.samples.get(older_index + 1) {
                if older.time - oldest.time > TIME_EPSILON {
                    return Some(hermite(oldest, older, newer, frac));
                }
            }
        }
        Some(T::lerp(&older.value, &newer.value, frac))
    }

    fn extrapolate(&self, time: f64) -> T {
        let newest = self.samples[0];
        let Some(previous) = self.samples.get(1) else {
            return newest.value;
        };
        let span = newest.time - previous.time;
        if span < 0.001 || time <= newest.time {
            return newest.value;
        }
        let ahead = (time - newest.time).min(self.max_extrapolation);
        // lerp past 1.0 continues the last velocity
        T::lerp(&previous.value, &newest.value, 1.0 + ahead / span)
    }
}

/// cubic hermite over [older, newer] with tangents from oldest->older and
/// older->newer. an unevenly spaced oldest sample is first moved along its
/// segment so both intervals match, which keeps the tangents comparable.
fn hermite<T: Lerp + Copy>(oldest: Sample<T>, older: Sample<T>, newer: Sample<T>, t: f64) -> T {
    let span = newer.time - older.time;
    let previous_span = older.time - oldest.time;
    let previous = if (span - previous_span).abs() > TIME_EPSILON && previous_span > TIME_EPSILON {
        T::lerp(&oldest.value, &older.value, 1.0 - span / previous_span)
    } else {
        oldest.value
    };
    let t2 = t * t;
    let t3 = t2 * t;
    // basis for p1, p2, tangent at p1, tangent at p2
    let h_p1 = 2.0 * t3 - 3.0 * t2 + 1.0;
    let h_p2 = -2.0 * t3 + 3.0 * t2;
    let h_d1 = t3 - 2.0 * t2 + t;
    let h_d2 = t3 - t2;
    T::hermite(&previous, &older.value, &newer.value, [h_p1, h_p2, h_d1, h_d2])
}
