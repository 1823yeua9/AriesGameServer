//! OracleStream: smooths values read from the client at irregular times
//! (the oracle polls on its own thread) by sampling a short, fixed delay in
//! the past, like a source client renders remote entities.
//! docs/port/10_interp_source_sdk.md 4.5-4.6.

use crate::time::{ticks_to_time, time_to_ticks};
use crate::value::Interp;
use crate::var::{InterpolatedVar, MAX_EXTRAPOLATE, Value};

/// source cl_interp_ratio default: two update intervals of delay.
pub const ORACLE_INTERP_RATIO: f64 = 2.0;
/// source EXTRA_INTERPOLATION_HISTORY_STORED.
pub const HISTORY_EXTRA: f64 = 0.05;
/// after this long without a snapshot the stream is reported held.
pub const STALE_AFTER: f64 = 0.5;

#[derive(Clone, Debug)]
pub struct OracleStream<T: Interp, const N: usize> {
    pub var: InterpolatedVar<T, N>,
    /// the period the stream is actually fed at
    pub nominal_period: f64,
    /// ticks_to_time(time_to_ticks(ratio * nominal_period) + 1)
    pub delay: f64,
    /// oldest age value_at must still answer (0 for display, max_unlag for shots)
    pub lookback: f64,
    pub newest_snapshot_t: f64,
}

/// a stream value plus whether the source has gone quiet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamValue<T> {
    pub value: Value<T>,
    pub stale: bool,
}

impl<T: Interp, const N: usize> OracleStream<T, N> {
    pub fn new(nominal_period: f64, lookback: f64) -> Self {
        let delay = ticks_to_time(time_to_ticks(ORACLE_INTERP_RATIO * nominal_period) + 1);
        Self { var: InterpolatedVar::new(), nominal_period, delay, lookback, newest_snapshot_t: f64::NEG_INFINITY }
    }

    /// pushes a snapshot taken at sample_t (every snapshot, even unchanged ones).
    pub fn ingest(&mut self, now: f64, sample_t: f64, v: T) {
        self.var.push(sample_t, v);
        self.newest_snapshot_t = sample_t;
        self.var.prune_before(now - self.delay.max(self.lookback) - HISTORY_EXTRA);
    }

    /// the smoothed value for this tick: `delay` behind, extrapolating up to
    /// 0.25 s only when no snapshot covers that time yet.
    pub fn value_now(&self, now: f64) -> Option<StreamValue<T>> {
        let value = self.var.value_at(now - self.delay, Some(MAX_EXTRAPOLATE))?;
        Some(StreamValue { value, stale: now - self.newest_snapshot_t > STALE_AFTER })
    }

    /// exact-time lookup (shots); no delay and no extrapolation.
    pub fn value_at(&self, t: f64) -> Option<Value<T>> {
        self.var.value_at(t, None)
    }
}
