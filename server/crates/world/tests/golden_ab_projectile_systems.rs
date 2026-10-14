//! the cloudburst systems (spec 08 sections 5.1, 5.2) against
//! server/fixtures/world_projectile_systems.json: ruh's own cloudburst_state and
//! flight tick, run per datagram on a fake camera and scripted projectile reads.

mod golden_ab_common;

use aries_world::bots::Camera;
use aries_world::projectiles::{FlightState, LAST_PROJECTILE_CHANNEL, ProjectileField, ProjectileSnapshot};
use golden_ab_common::{Sequences, check_sends, f64_of, fields, fixture, float_field, u32_of, vec3};
use serde_json::{Value, json};

fn snapshot(value: &Value) -> Option<ProjectileSnapshot> {
    if value.is_null() {
        return None;
    }
    Some(ProjectileSnapshot {
        class_name: value["class_name"].as_str().unwrap().to_string(),
        location: (!value["location"].is_null()).then(|| vec3(&value["location"])),
        collisions: u32_of(&value["collisions"]),
        hit_location: vec3(&value["hit_location"]),
        hit_normal: vec3(&value["hit_normal"]),
    })
}

#[test]
fn throws_and_flight_ticks_match_python() {
    replay(0.0);
}

/// python's first flight-tick gate (now - 0) always passes on its uptime
/// clock; ours may start near zero. 7000.1875 puts the first datagram at
/// 0.0125 s, under the 0.05 s gate, and keeps every `now - shift` and every
/// gate difference exact (sterbenz). the zone deadlines (now + 3.2) round at
/// another magnitude, which could only matter for a datagram within an ulp of
/// one; none is in this fixture.
#[test]
fn a_clock_from_zero_changes_nothing() {
    replay(7000.1875);
}

fn replay(shift: f64) {
    let f = fixture("world_projectile_systems");
    if shift > 0.0 {
        assert!(f["datagrams"].as_array().unwrap().iter().all(|d| (shift..=2.0 * shift).contains(&f64_of(&d["now"]))));
    }
    assert_eq!(f["lines"]["cloudburst_state"], 1016);
    assert_eq!(f["lines"]["flight_tick"], 2026);
    let mut sequences = Sequences::new(&f);
    let mut field = ProjectileField::default();
    let (mut opens, mut zones, mut closes) = (0, 0, 0);
    for (k, datagram) in f["datagrams"].as_array().unwrap().iter().enumerate() {
        let now = f64_of(&datagram["now"]) - shift;
        let event = &datagram["event"];
        let label = format!("datagram {k}");

        // the mirror poll (ruh:1619-1621)
        let thrown = field.smoke_state(datagram["predicted"].as_str());
        let camera: Option<Vec<f64>> = datagram["camera"].as_str().map(|_| fields(&datagram["camera"]).iter().map(|p| float_field(p)).collect());
        match (thrown, camera) {
            (true, Some(camera)) => {
                let camera = Camera { location: [camera[0], camera[1], camera[2]], pitch: camera[3], yaw: camera[4], roll: 0.0 };
                let open = field.throw(&camera, now).unwrap();
                check_sends(std::slice::from_ref(&open.send), &datagram["state_sends"], &mut sequences, &format!("{label} throw"));
                assert_eq!(json!([open.channel, open.actor_guid, open.location, open.velocity, "A"]), event["cloudburst_open"], "{label} open event");
                opens += 1;
            }
            _ => {
                assert!(datagram["state_sends"].as_array().unwrap().is_empty(), "{label}: python threw");
                assert!(event.get("cloudburst_open").is_none(), "{label}: python threw");
            }
        }

        // the flight tick (ruh:2026-2030)
        let reads = datagram["reads"].as_array().unwrap();
        let yaws = datagram["yaws"].as_array().unwrap();
        if field.tick_due(now) {
            let mut read_calls = reads.iter();
            let mut yaw_calls = yaws.iter();
            let tick = field
                .tick(
                    now,
                    |channel| {
                        let call = read_calls.next().unwrap_or_else(|| panic!("{label}: rust read channel {channel} python did not"));
                        assert_eq!(channel, u32_of(&call["channel"]), "{label} read order");
                        // a failed read is no snapshot (python logs cloudburst_read_error)
                        if call.get("error").is_some() { None } else { snapshot(&call["snapshot"]) }
                    },
                    || f64_of(yaw_calls.next().unwrap_or_else(|| panic!("{label}: extra zone yaw"))),
                );
            assert!(tick.error.is_none(), "{label}: {:?}", tick.error);
            assert!(read_calls.next().is_none(), "{label}: python read more flights");
            assert!(yaw_calls.next().is_none(), "{label}: python opened more zones");
            check_sends(&tick.sends, &datagram["tick_sends"], &mut sequences, &format!("{label} tick"));
            // python's event keeps the last zone of the tick
            let zone = tick.zones.last().map(|z| json!([z.channel, z.zone_guid, z.location, z.reason.name()]));
            assert_eq!(zone.unwrap_or(Value::Null), event.get("cloudburst_zone").cloned().unwrap_or(Value::Null), "{label} zone event");
            assert_eq!(json!(tick.zones_closed), event.get("cloudburst_zone_closed").cloned().unwrap_or(json!([])), "{label} zone closed");
            zones += tick.zones.len();
            closes += tick.zones_closed.len();
        } else {
            assert!(reads.is_empty() && yaws.is_empty(), "{label}: python ticked, rust gate closed");
            assert!(datagram["tick_sends"].as_array().unwrap().is_empty(), "{label}: python ticked");
        }
    }
    assert!(opens > 60 && zones > 60 && closes > 60, "{opens} opens, {zones} zones, {closes} closes");
    let last = &f["final"];
    assert_eq!(field.next_channel, u32_of(&last["next_channel"]));
    assert_eq!(field.next_guid, u32_of(&last["next_guid"]));
    assert_eq!(field.flights.len() as u64, last["flights"].as_u64().unwrap());
}

/// python raises out of the tick when a stop finds no channel left (rp:99-100);
/// we stop there too, but what the tick did before still goes out.
#[test]
fn running_out_of_channels_keeps_the_earlier_sends() {
    let camera = Camera { location: [4786.03, -13285.84, 1427.98], pitch: -2.86, yaw: -130.79, roll: 0.0 };
    let mut field = ProjectileField::default();
    let first = field.throw(&camera, 10.0).unwrap();
    let second = field.throw(&camera, 10.0).unwrap();
    field.next_channel = LAST_PROJECTILE_CHANNEL;
    // both are never seen and fall back at once: the first takes the last
    // channel for its zone and closes, the second finds none
    let tick = field.tick(10.3, |_| None, || 0.0);
    assert!(tick.error.is_some());
    assert_eq!(tick.zones.len(), 1);
    assert_eq!(tick.zones[0].channel, LAST_PROJECTILE_CHANNEL);
    let sent: Vec<(u32, bool, bool)> = tick.sends.iter().map(|s| (s.channel, s.is_open, s.is_close)).collect();
    assert_eq!(sent, vec![(LAST_PROJECTILE_CHANNEL, true, false), (first.channel, false, true)]);
    let stuck = field.flights.iter().find(|f| f.channel == second.channel).unwrap();
    assert!(stuck.state == FlightState::Stopped && stuck.zone_channel.is_none());
    // a later throw is refused without allocating anything
    let guid = field.next_guid;
    assert!(field.throw(&camera, 11.0).is_err());
    assert_eq!(field.next_guid, guid);
}
