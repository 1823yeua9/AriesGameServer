//! docs/port/10_interp_source_sdk.md section 5 vectors (from an offline python
//! reference of the same operation order). bit equality where the doc gives hex.

use aries_interp::*;

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

fn hexf(text: &str) -> f64 {
    // parses c99 hex floats like 0x1.4000000000000p+4 / -0x1.dfffffffffffap+5 / 0x0.0p+0
    let negative = text.starts_with('-');
    let body = text.trim_start_matches('-').trim_start_matches("0x");
    let (mantissa, exponent) = body.split_once('p').unwrap();
    let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let mut value = i64::from_str_radix(int_part, 16).unwrap() as f64;
    let mut scale = 1.0 / 16.0;
    for c in frac_part.chars() {
        value += c.to_digit(16).unwrap() as f64 * scale;
        scale /= 16.0;
    }
    let value = value * 2f64.powi(exponent.parse::<i32>().unwrap());
    if negative { -value } else { value }
}

#[test]
fn ticks() {
    for (dt, ticks) in [(0.0078125, 1), (0.015625, 2), (0.05, 6), (0.1, 13), (0.109375, 14), (0.2, 26), (0.25, 32), (0.3, 38), (1.0, 128), (0.0039, 0), (0.00390625, 1)] {
        assert_eq!(time_to_ticks(dt), ticks, "{dt}");
    }
    for (ticks, t) in [(1, 0.0078125), (7, 0.0546875), (13, 0.1015625), (14, 0.109375), (26, 0.203125), (32, 0.25), (128, 1.0)] {
        assert_eq!(ticks_to_time(ticks), t);
    }
    assert_eq!(entity_interp(0.1), 0.109375);
    assert_eq!(entity_interp(0.015625), 0.0234375);
    assert_eq!(entity_interp(0.031), 0.0390625);
}

fn track() -> InterpolatedVar<Vec3, 32> {
    let mut var = InterpolatedVar::new();
    for (t, p) in [(10.0, v(0.0, 0.0, 0.0)), (10.0078125, v(5.0, 0.0, 1.0)), (10.015625, v(10.0, 0.0, 4.0)), (10.0234375, v(15.0, 0.0, 9.0)), (10.0390625, v(25.0, 2.0, 16.0))] {
        var.push(t, p);
    }
    var
}

#[test]
fn linear_and_hermite() {
    let var = track();
    let at = |t: f64| var.value_at(t, None).unwrap();

    let r = at(10.03125);
    assert_eq!(r.kind, Kind::Hermite);
    assert_eq!(var.locate(10.03125), Locate::Between { newer: 0, older: 1, oldest: Some(2), frac: 0.5 });
    assert_eq!((r.v.x.to_bits(), r.v.y.to_bits(), r.v.z.to_bits()), (hexf("0x1.4000000000000p+4").to_bits(), hexf("0x1.8000000000000p-1").to_bits(), hexf("0x1.9c00000000000p+3").to_bits()));

    let r = at(10.02734375);
    assert_eq!(r.kind, Kind::Hermite);
    assert_eq!((r.v.x, r.v.y, r.v.z), (hexf("0x1.1800000000000p+4"), hexf("0x1.c000000000000p-3"), hexf("0x1.6580000000000p+3")));

    let r = at(10.01953125);
    assert_eq!(var.locate(10.01953125), Locate::Between { newer: 1, older: 2, oldest: Some(3), frac: 0.5 });
    assert_eq!(r.v, v(12.5, 0.0, 6.25));

    assert_eq!(at(10.0234375).v, v(15.0, 0.0, 9.0));
    let r = at(10.0);
    assert_eq!((r.kind, r.v), (Kind::Linear, v(0.0, 0.0, 0.0)));
    assert_eq!(var.locate(10.0), Locate::Between { newer: 3, older: 4, oldest: None, frac: 0.0 });
    assert_eq!((at(9.99).kind, at(9.99).v), (Kind::HeldOldest, v(0.0, 0.0, 0.0)));
    assert_eq!((at(10.05).kind, at(10.05).v), (Kind::HeldNewest, v(25.0, 2.0, 16.0)));

    let mut linear = track();
    linear.linear_only = true;
    let r = linear.value_at(10.03125, None).unwrap();
    assert_eq!((r.kind, r.v), (Kind::Linear, v(20.0, 1.0, 12.5)));
}

#[test]
fn push_flush_and_prune() {
    let mut var: InterpolatedVar<f64, 32> = InterpolatedVar::new();
    for (t, x) in [(1.0, 1.0), (2.0, 2.0), (3.0, 3.0), (2.5, 25.0)] {
        var.push(t, x);
    }
    let times: Vec<f64> = (0..var.len()).map(|i| var.sample(i).t).collect();
    assert_eq!(times, vec![2.5, 2.0, 1.0]);
    var.push(2.50005, 26.0);
    let times: Vec<f64> = (0..var.len()).map(|i| var.sample(i).t).collect();
    assert_eq!(times, vec![2.50005, 2.0, 1.0]);

    let mut var: InterpolatedVar<f64, 32> = InterpolatedVar::new();
    for t in 0..10 {
        var.push(t as f64, t as f64);
    }
    var.prune_before(6.5);
    let times: Vec<f64> = (0..var.len()).map(|i| var.sample(i).t).collect();
    assert_eq!(times, vec![9.0, 8.0, 7.0, 6.0, 5.0, 4.0]);
}

#[test]
fn extrapolation() {
    let mut var: InterpolatedVar<Vec3, 32> = InterpolatedVar::new();
    var.push(1.0, v(0.0, 0.0, 0.0));
    var.push(1.05, v(10.0, 0.0, -5.0));
    let at = |t: f64| var.value_at(t, Some(0.25)).unwrap();
    assert_eq!((at(1.04).kind, at(1.04).v), (Kind::Linear, v(8.0, 0.0, -4.0)));
    assert_eq!(at(1.05).kind, Kind::HeldNewest);
    let r = at(1.1);
    assert_eq!(r.kind, Kind::Extrapolated { amount: 0.050000000000000044 });
    assert_eq!(r.v, v(20.0, 0.0, -10.0));
    let r = at(1.3);
    assert_eq!(r.kind, Kind::Extrapolated { amount: 0.25 });
    assert_eq!((r.v.x, r.v.z), (hexf("0x1.dfffffffffffap+5"), hexf("-0x1.dfffffffffffap+4")));
    assert_eq!(at(1.5).v, r.v);

    let mut short: InterpolatedVar<f64, 8> = InterpolatedVar::new();
    short.push(1.0, 0.0);
    short.push(1.0005, 10.0);
    let r = short.value_at(1.2, Some(0.25)).unwrap();
    assert_eq!((r.kind, r.v), (Kind::HeldNewest, 10.0));
}

#[test]
fn velocity_and_rotation() {
    let mut var: InterpolatedVar<Vec3, 32> = InterpolatedVar::new();
    var.push(1.0, v(0.0, 0.0, 0.0));
    var.push(1.05, v(10.0, 0.0, -5.0));
    let velocity = var.velocity_at(1.02).unwrap();
    assert_eq!((velocity.x, velocity.z), (hexf("0x1.8fffffffffffap+7"), hexf("-0x1.8fffffffffffap+6")));
    // past the newest sample the velocity ramps from E to 2E
    assert_eq!(var.velocity_at(1.2).unwrap(), velocity);
    assert!((var.velocity_at(1.35).unwrap().x - velocity.x * 0.7999999999999998).abs() < 1e-9);
    assert!((var.velocity_at(1.45).unwrap().x - 80.0).abs() < 1e-9);
    assert_eq!(var.velocity_at(1.6).unwrap().x, 0.0);

    let hermite_rate = Vec3::hermite_rate(v(0.0, 0.0, 0.0), v(5.0, 0.0, 1.0), v(10.0, 0.0, 4.0), 0.5, 0.0078125);
    assert_eq!(hermite_rate, v(640.0, 0.0, 448.0));

    assert_eq!(Rot::lerp(Rot::new(0.0, 170.0, 0.0), Rot::new(0.0, -170.0, 0.0), 0.5), Rot::new(0.0, -180.0, 0.0));
    assert_eq!(Rot::lerp(Rot::new(0.0, 170.0, 0.0), Rot::new(0.0, -170.0, 0.0), 0.25), Rot::new(0.0, 175.0, 0.0));
    assert_eq!(Rot::lerp(Rot::new(350.0, 10.0, 0.0), Rot::new(10.0, 30.0, 0.0), 0.5), Rot::new(0.0, 20.0, 0.0));
    assert_eq!(Rot::lerp(Rot::new(-10.0, 0.0, 0.0), Rot::new(20.0, 90.0, 0.0), 0.75), Rot::new(12.5, 67.5, 0.0));

    let mut rot: InterpolatedVar<Rot, 8> = InterpolatedVar::new();
    rot.push(2.0, Rot::new(0.0, 179.0, 0.0));
    rot.push(2.0078125, Rot::new(0.0, -179.0, 0.0));
    rot.push(2.015625, Rot::new(0.0, -177.0, 0.0));
    let r = rot.value_at(2.01171875, None).unwrap();
    assert_eq!((r.kind, r.v), (Kind::Linear, Rot::new(0.0, -178.0, 0.0)));
}

fn lag() -> LagCompensation {
    LagCompensation::new(LagCompConfig::default())
}

#[test]
fn target_time() {
    let now = ticks_to_time(1000);
    let l = lag();
    let t = l.target_time(now, 0.0, 0.0, None);
    assert_eq!((t.t, t.source), (7.8125, TargetSource::Latency));
    let t = l.target_time(now, 0.0123, 0.05, None);
    assert_eq!((t.lerp, t.correct, t.t, t.source), (0.046875, 0.059175, hexf("0x1.f0367a0f9096cp+2"), TargetSource::Latency));
    let t = l.target_time(now, 0.0123, 0.05, Some(7.8025));
    assert_eq!((t.t, t.source), (hexf("0x1.f05c28f5c28f6p+2"), TargetSource::Claimed));
    let t = l.target_time(now, 0.0123, 0.05, Some(7.4125));
    assert_eq!((t.t, t.source), (7.753325, TargetSource::Latency));
    let t = l.target_time(now, 1.5, 0.1, None);
    assert_eq!((t.lerp, t.correct, t.t), (0.1015625, 1.0, 6.8125));
    let t = l.target_time(now, 0.03, 0.1, Some(7.8625));
    assert_eq!((t.correct, t.t, t.source), (0.1315625, hexf("0x1.f0b3333333333p+2"), TargetSource::Claimed));
}

fn pose(x: f64, alive: bool, serial: u16) -> Pose {
    Pose { origin: v(x, 0.0, 0.0), yaw: 0.0, shape: HitShape::TrainingBot { lift: 0.0 }, alive, serial }
}

fn record_track(l: &mut LagCompensation, track: &[(f64, bool, u16)], first_tick: u64) {
    for (i, &(x, alive, serial)) in track.iter().enumerate().rev() {
        let tick = first_tick - i as u64;
        l.record(tick, ticks_to_time(tick as i64), [(1, pose(x, alive, serial))]);
    }
}

#[test]
fn rewind_edge_cases() {
    let now = ticks_to_time(10);
    let current = pose(100.0, true, 0);
    let mut l = lag();
    record_track(&mut l, &[(90.0, true, 0), (80.0, true, 0), (70.0, true, 0), (60.0, true, 0), (50.0, true, 0)], 9);
    let at = |l: &LagCompensation, ticks: f64| l.rewind(1, &current, now, ticks / 128.0);
    assert!(matches!(at(&l, 10.0), Rewind::Exact { pose, .. } if pose.origin.x == 100.0));
    assert!(matches!(at(&l, 9.5), Rewind::Interpolated { pose, frac, .. } if pose.origin.x == 95.0 && frac == 0.5));
    assert!(matches!(at(&l, 8.0), Rewind::Exact { pose, .. } if pose.origin.x == 80.0));
    assert!(matches!(at(&l, 7.25), Rewind::Interpolated { pose, frac, .. } if pose.origin.x == 72.5 && frac == 0.25));
    assert!(matches!(at(&l, 4.0), Rewind::Clamped { pose, .. } if pose.origin.x == 50.0));

    let mut jump = lag();
    record_track(&mut jump, &[(90.0, true, 0), (10.0, true, 0), (9.0, true, 0)], 9);
    assert_eq!(jump.rewind(1, &current, now, 7.5 / 128.0), Rewind::Unavailable(Lost::TeleportDistance));
    assert_eq!(jump.rewind(1, &current, now, 8.5 / 128.0), Rewind::Unavailable(Lost::TeleportDistance));

    let mut dead = lag();
    record_track(&mut dead, &[(90.0, false, 0), (80.0, true, 0), (70.0, true, 0)], 9);
    assert_eq!(dead.rewind(1, &current, now, 7.5 / 128.0), Rewind::Unavailable(Lost::DeadOnPath));

    let mut serial = lag();
    let current1 = pose(100.0, true, 1);
    record_track(&mut serial, &[(90.0, true, 1), (80.0, true, 0)], 9);
    assert_eq!(serial.rewind(1, &current1, now, 8.5 / 128.0), Rewind::Unavailable(Lost::SerialChanged));

    let empty = lag();
    assert!(matches!(empty.rewind(1, &current, now, 9.0 / 128.0), Rewind::Clamped { pose, .. } if pose.origin.x == 100.0));
    // unusable history falls back to the live pose
    assert_eq!(Rewind::Unavailable(Lost::DeadOnPath).effective(&current), Some(current));
    assert_eq!(Rewind::Unavailable(Lost::DeadOnPath).effective(&pose(1.0, false, 0)), None);
}

#[test]
fn records_prune_by_max_unlag() {
    let mut l = lag();
    for tick in 0..300u64 {
        l.record(tick, ticks_to_time(tick as i64), [(7, pose(tick as f64, true, 0))]);
        l.prune(ticks_to_time(tick as i64));
    }
    let track = l.track(7).unwrap();
    assert_eq!(track.len(), 129);
    assert!(track.back().unwrap().t >= ticks_to_time(299) - 1.0);
    l.forget(7);
    assert!(l.track(7).is_none());
}

#[test]
fn oracle_stream_delay_and_staleness() {
    let mut stream: OracleStream<Vec3, 32> = OracleStream::new(0.05, 0.0);
    assert_eq!(stream.delay, 0.109375);
    let mut now = 0.0;
    for i in 0..20 {
        now = i as f64 * 0.05;
        stream.ingest(now, now, v(i as f64 * 10.0, 0.0, 0.0));
    }
    // smooth value 0.109375 behind now, on the 200 u/s line
    let value = stream.value_now(now).unwrap();
    assert!(!value.stale);
    assert!((value.value.v.x - (now - 0.109375) * 200.0).abs() < 1e-6);
    // the oracle stalls: extrapolate up to 0.25 s, flag stale after 0.5 s
    let stalled = stream.value_now(now + 0.6).unwrap();
    assert!(stalled.stale);
    assert!(matches!(stalled.value.kind, Kind::Extrapolated { amount } if amount == 0.25));
    assert_eq!(OracleStream::<Vec3, 8>::new(TICK_DT, 1.0).delay, 0.0234375);
}
