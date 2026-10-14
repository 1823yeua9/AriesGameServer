//! ticks and time at 128 hz (source TIME_TO_TICKS / TICKS_TO_TIME).

pub const TICK_HZ: u32 = 128;
/// 0.0078125 s, exact in binary.
pub const TICK_DT: f64 = 1.0 / TICK_HZ as f64;

/// source TIME_TO_TICKS: (int)(0.5 + dt / interval). `as` truncates toward
/// zero like the c cast; callers pass dt >= 0. dt * 128 == dt / TICK_DT exactly.
pub fn time_to_ticks(dt: f64) -> i64 {
    (0.5 + dt * TICK_HZ as f64) as i64
}

pub fn ticks_to_time(ticks: i64) -> f64 {
    ticks as f64 * TICK_DT
}

/// source entity interpolation amount: one tick past the rounded client interp.
pub fn entity_interp(client_interp: f64) -> f64 {
    ticks_to_time(time_to_ticks(client_interp) + 1)
}
