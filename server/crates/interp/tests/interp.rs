use aries_interp::*;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn linear_interpolates_between_bracketing_samples() {
    let mut h = History::new(1.0, Mode::Linear);
    h.push(0.0, 0.0);
    h.push(1.0, 10.0);
    h.push(2.0, 30.0);
    assert!(close(h.sample(0.5).unwrap(), 5.0));
    assert!(close(h.sample(1.5).unwrap(), 20.0));
    assert!(close(h.sample(2.0).unwrap(), 30.0));
    // before all history: hold the oldest
    assert!(close(h.sample(-1.0).unwrap(), 0.0));
}

#[test]
fn hermite_passes_through_samples_and_is_smooth_on_a_line() {
    let mut h = History::new(1.0, Mode::Hermite);
    for i in 0..5 {
        h.push(i as f64 * 0.1, i as f64 * 7.0);
    }
    // a straight line stays a straight line
    for k in 0..40 {
        let t = 0.1 + k as f64 * 0.0075;
        assert!(close(h.sample(t).unwrap(), t * 70.0), "t {t}");
    }
    assert!(close(h.sample(0.2).unwrap(), 14.0));
}

#[test]
fn hermite_renormalizes_uneven_spacing() {
    // constant velocity, uneven spacing: still exact
    let mut h = History::new(2.0, Mode::Hermite);
    for &t in &[0.0, 0.3, 0.35, 0.5] {
        h.push(t, 4.0 * t);
    }
    for k in 0..10 {
        let t = 0.35 + k as f64 * 0.015;
        assert!(close(h.sample(t).unwrap(), 4.0 * t), "t {t}");
    }
}

#[test]
fn extrapolation_continues_velocity_and_clamps() {
    let mut h = History::new(1.0, Mode::Hermite);
    h.push(0.0, [0.0, 0.0, 0.0]);
    h.push(0.1, [10.0, 0.0, -1.0]);
    let a = h.sample(0.2).unwrap();
    assert!(close(a[0], 20.0) && close(a[2], -2.0));
    // capped at 0.25 s past the newest sample
    let b = h.sample(5.0).unwrap();
    assert!(close(b[0], 10.0 + 100.0 * DEFAULT_MAX_EXTRAPOLATION));
}

#[test]
fn rotators_take_the_short_way_round() {
    let mut h = History::new(1.0, Mode::Linear);
    h.push(0.0, Rotator([0.0, 350.0, 0.0]));
    h.push(1.0, Rotator([0.0, 10.0, 0.0]));
    let mid = h.sample(0.5).unwrap();
    assert!(close(mid.0[1].rem_euclid(360.0), 0.0));
    assert!(close(angle_delta(359.0, 1.0), 2.0) && close(angle_delta(1.0, 359.0), -2.0));
}

#[test]
fn time_going_back_resets_newer_samples() {
    let mut h = History::new(1.0, Mode::Linear);
    h.push(1.0, 1.0);
    h.push(2.0, 2.0);
    h.push(1.5, 9.0);
    assert_eq!(h.len(), 2);
    assert!(close(h.newest().unwrap().value, 9.0));
}

#[test]
fn history_prunes_but_keeps_an_edge_sample() {
    let mut h = History::new(0.5, Mode::Linear);
    for i in 0..100 {
        h.push(i as f64 * 0.1, i as f64);
    }
    assert!(h.len() <= 8);
    assert!(close(h.sample(9.9 - 0.5).unwrap(), 94.0));
}

fn rec(time: f64, x: f64, alive: bool) -> LagRecord {
    LagRecord { time, alive, origin: [x, 0.0, 0.0], rotation: Rotator::default(), extent: [34.0, 98.0] }
}

#[test]
fn lag_compensation_backtracks_and_interpolates() {
    let mut lag = LagCompensation::default();
    for i in 0..=128 {
        lag.record(1u32, rec(i as f64 * TICK_INTERVAL, i as f64 * 5.0, true));
    }
    // exact record
    assert!(close(lag.backtrack(1, 64.0 * TICK_INTERVAL).unwrap().origin[0], 320.0));
    // between records
    let r = lag.backtrack(1, 64.5 * TICK_INTERVAL).unwrap();
    assert!(close(r.origin[0], 322.5));
    // same or older time is not recorded twice
    lag.record(1, rec(0.5, 999.0, true));
    assert_eq!(lag.track(1).unwrap().len(), 129);
    lag.prune(1.0 + 0.5);
    assert!(lag.track(1).unwrap().back().unwrap().time >= 0.5);
}

#[test]
fn lag_compensation_refuses_deaths_and_teleports() {
    let mut lag = LagCompensation::default();
    lag.record(7u32, rec(0.0, 0.0, true));
    lag.record(7, rec(0.1, 10.0, false));
    lag.record(7, rec(0.2, 20.0, true));
    assert!(lag.backtrack(7, 0.05).is_none());
    let mut lag = LagCompensation::default();
    lag.record(8u32, rec(0.0, 0.0, true));
    lag.record(8, rec(0.1, 5000.0, true));
    assert!(lag.backtrack(8, 0.05).is_none());
    assert!(close(lag.backtrack(8, 0.15).unwrap().origin[0], 5000.0));
}

#[test]
fn target_time_clamps_and_sanity_checks_client_claims() {
    let lag: LagCompensation<u32> = LagCompensation::default();
    assert!(close(lag.target_time(10.0, 0.05, 0.03, None), 9.92));
    assert!(close(lag.target_time(10.0, 5.0, 0.0, None), 9.0));
    // a believable client claim wins, a wild one does not
    assert!(close(lag.target_time(10.0, 0.05, 0.03, Some(9.96)), 9.93));
    assert!(close(lag.target_time(10.0, 0.05, 0.03, Some(5.0)), 9.92));
    assert_eq!(time_to_ticks(1.0), 128);
    assert!(close(ticks_to_time(64), 0.5));
}
