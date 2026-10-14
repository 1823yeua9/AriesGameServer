//! InterpolatedVar: a time-stamped history (newest first) sampled the way
//! source's CInterpolatedVar does (GetInterpolationInfo + Interpolate),
//! reimplemented. docs/port/10_interp_source_sdk.md 3.2-3.7 and 4.4.

use std::collections::VecDeque;

use crate::value::{Interp, Velocity};

/// insert drops head samples with t0 + this > t (clock went back, duplicates).
pub const FLUSH_EPS: f64 = 1e-4;
/// segments shorter than this are not interpolated across.
pub const SEGMENT_EPS: f64 = 1e-4;
/// extrapolation needs a last segment at least this long.
pub const EXTRAP_EPS: f64 = 1e-3;
/// source cl_extrapolate_amount.
pub const MAX_EXTRAPOLATE: f64 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample<T> {
    pub t: f64,
    pub v: T,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Locate {
    Empty,
    /// target >= the newest sample time
    AtOrAfterNewest,
    /// target before every sample
    BeforeOldest,
    /// older <= target < newer; oldest is the hermite third point
    Between { newer: usize, older: usize, oldest: Option<usize>, frac: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Hermite,
    Linear,
    HeldNewest,
    HeldOldest,
    Extrapolated { amount: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Value<T> {
    pub v: T,
    pub kind: Kind,
}

#[derive(Clone, Debug)]
pub struct InterpolatedVar<T: Interp, const N: usize> {
    samples: VecDeque<Sample<T>>,
    /// source INTERPOLATE_LINEAR_ONLY
    pub linear_only: bool,
}

impl<T: Interp, const N: usize> Default for InterpolatedVar<T, N> {
    fn default() -> Self {
        Self { samples: VecDeque::with_capacity(N), linear_only: false }
    }
}

impl<T: Interp, const N: usize> InterpolatedVar<T, N> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// 0 = newest.
    pub fn sample(&self, i: usize) -> &Sample<T> {
        &self.samples[i]
    }

    pub fn newest(&self) -> Option<&Sample<T>> {
        self.samples.front()
    }

    /// adds a sample, first dropping head samples it does not follow. at capacity the oldest goes.
    pub fn push(&mut self, t: f64, v: T) {
        while self.samples.front().is_some_and(|s| s.t + FLUSH_EPS > t) {
            self.samples.pop_front();
        }
        self.samples.push_front(Sample { t, v });
        if self.samples.len() > N {
            self.samples.pop_back();
        }
    }

    /// discontinuity: three copies so hermite has its points.
    pub fn reset(&mut self, t: f64, v: T) {
        self.samples.clear();
        for _ in 0..3.min(N) {
            self.samples.push_front(Sample { t, v });
        }
    }

    /// keeps the newest sample older than cutoff plus two more (hermite needs them).
    pub fn prune_before(&mut self, cutoff: f64) {
        if let Some(i) = self.samples.iter().position(|s| s.t < cutoff) {
            self.samples.truncate((i + 3).min(self.samples.len()));
        }
    }

    pub fn locate(&self, target: f64) -> Locate {
        let Some(first) = self.samples.front() else {
            return Locate::Empty;
        };
        if target >= first.t {
            return Locate::AtOrAfterNewest;
        }
        let mut newer = 0usize;
        for (i, sample) in self.samples.iter().enumerate() {
            if target < sample.t {
                newer = i;
                continue;
            }
            let dt = self.samples[newer].t - sample.t;
            if dt <= SEGMENT_EPS {
                return Locate::Between { newer, older: i, oldest: None, frac: 0.0 };
            }
            let frac = (target - sample.t) / dt;
            let oldest = (!self.linear_only && T::HERMITE)
                .then_some(i + 1)
                .filter(|&o| o < self.samples.len() && sample.t - self.samples[o].t > SEGMENT_EPS);
            return Locate::Between { newer, older: i, oldest, frac };
        }
        Locate::BeforeOldest
    }

    /// p0 moved along p0->p1 so both segments have the newer segment's length (source time fixup).
    fn fixed_p0(&self, oldest: usize, older: usize, newer: usize) -> T {
        let (p0, p1, p2) = (&self.samples[oldest], &self.samples[older], &self.samples[newer]);
        let dn = p2.t - p1.t;
        let d_o = p1.t - p0.t;
        if (dn - d_o).abs() > SEGMENT_EPS && d_o > SEGMENT_EPS {
            let s = dn / d_o;
            T::lerp(p0.v, p1.v, 1.0 - s)
        } else {
            p0.v
        }
    }

    /// value at `target`. `extrapolate`: Some(max seconds) only when the caller
    /// decided the source is stale; otherwise past the newest sample it holds.
    pub fn value_at(&self, target: f64, extrapolate: Option<f64>) -> Option<Value<T>> {
        match self.locate(target) {
            Locate::Empty => None,
            Locate::BeforeOldest => self.samples.back().map(|s| Value { v: s.v, kind: Kind::HeldOldest }),
            Locate::Between { newer, older, oldest: Some(oldest), frac } => {
                let p0 = self.fixed_p0(oldest, older, newer);
                Some(Value { v: T::hermite(p0, self.samples[older].v, self.samples[newer].v, frac), kind: Kind::Hermite })
            }
            Locate::Between { newer, older, oldest: None, frac } => {
                Some(Value { v: T::lerp(self.samples[older].v, self.samples[newer].v, frac), kind: Kind::Linear })
            }
            Locate::AtOrAfterNewest => {
                let new = self.samples[0];
                let held = Some(Value { v: new.v, kind: Kind::HeldNewest });
                let (Some(max), Some(old)) = (extrapolate, self.samples.get(1)) else {
                    return held;
                };
                if (new.t - old.t).abs() < EXTRAP_EPS || target <= new.t {
                    return held;
                }
                let amount = (target - new.t).min(max);
                let f = 1.0 + amount * (1.0 / (new.t - old.t));
                Some(Value { v: T::extrapolate(old.v, new.v, f), kind: Kind::Extrapolated { amount } })
            }
        }
    }
}

impl<T: Velocity, const N: usize> InterpolatedVar<T, N> {
    /// rate of change at `target` (source GetDerivative). past the newest
    /// sample the last segment's velocity ramps to zero between E and 2E.
    pub fn velocity_at(&self, target: f64) -> Option<T> {
        let zero = |v: T| T::rate(v, v, 0.0);
        match self.locate(target) {
            Locate::Empty => None,
            Locate::BeforeOldest => self.samples.back().map(|s| zero(s.v)),
            Locate::Between { newer, older, oldest: Some(oldest), frac } => {
                let p0 = self.fixed_p0(oldest, older, newer);
                let span = self.samples[newer].t - self.samples[older].t;
                Some(T::hermite_rate(p0, self.samples[older].v, self.samples[newer].v, frac, span))
            }
            Locate::Between { newer, older, oldest: None, .. } => {
                let dt = self.samples[newer].t - self.samples[older].t;
                if dt.abs() < SEGMENT_EPS {
                    return Some(zero(self.samples[older].v));
                }
                Some(T::rate(self.samples[older].v, self.samples[newer].v, 1.0 / dt))
            }
            Locate::AtOrAfterNewest => {
                let new = self.samples[0];
                let Some(old) = self.samples.get(1) else {
                    return Some(zero(new.v));
                };
                let dt = new.t - old.t;
                if dt.abs() < SEGMENT_EPS {
                    return Some(zero(new.v));
                }
                let velocity = T::rate(old.v, new.v, 1.0 / dt);
                let diff = (target - new.t).clamp(0.0, MAX_EXTRAPOLATE * 2.0);
                let scale = if diff > MAX_EXTRAPOLATE { 1.0 - (diff - MAX_EXTRAPOLATE) / MAX_EXTRAPOLATE } else { 1.0 };
                Some(velocity.scaled(scale))
            }
        }
    }
}
