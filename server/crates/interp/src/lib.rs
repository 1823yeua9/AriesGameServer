//! interpolation, extrapolation and lag compensation for the 128 hz server.
//! the design follows valve's source sdk 2013 (interpolated var histories and
//! player lag compensation); this is our own implementation of those ideas.

mod history;
mod lag;
mod lerp;

pub use history::{DEFAULT_MAX_EXTRAPOLATION, History, Mode, Sample};
pub use lag::{DEFAULT_MAX_UNLAG, DEFAULT_TELEPORT_DISTANCE, LagCompensation, LagRecord, MAX_CLIENT_TIME_ERROR};
pub use lerp::{Lerp, Rotator, View, angle_delta};

/// server tick interval at 128 hz.
pub const TICK_INTERVAL: f64 = 1.0 / 128.0;

/// seconds -> whole ticks, rounded (source TIME_TO_TICKS).
pub fn time_to_ticks(seconds: f64) -> i64 {
    (0.5 + seconds / TICK_INTERVAL).floor() as i64
}

/// ticks -> seconds (source TICKS_TO_TIME).
pub fn ticks_to_time(ticks: i64) -> f64 {
    ticks as f64 * TICK_INTERVAL
}
