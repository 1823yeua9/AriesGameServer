//! cloudburst stage A (spec 08 section 5) against server/fixtures/world_projectiles.json:
//! throw transforms for many camera poses, the projectile and zone opens,
//! FRepMovement, the ballistic fallback and the stop logic.

mod golden_ab_common;

use aries_bits::BitWriter;
use aries_world::projectiles::{
    Flight, ProjectileField, ProjectileSnapshot, append_rep_movement, ballistic_fallback, flight_stop_point, smoke_location_from_hit,
    smoke_projectile_open_body, smoke_thrown, smoke_zone_open_body, throw_transform,
};
use golden_ab_common::{check, f64_of, fixture, same, same3, u32_of, vec3};
use serde_json::Value;

#[test]
fn throws_and_projectile_opens_match_python() {
    let f = fixture("world_projectiles");
    let throws = f["throws"].as_array().unwrap();
    assert!(throws.len() > 150);
    let mut field = ProjectileField::default();
    for (k, case) in throws.iter().enumerate() {
        let label = format!("throw {k}");
        let throw = throw_transform(vec3(&case["view"]), f64_of(&case["view_pitch"]), f64_of(&case["view_yaw"]));
        same3(throw.location, vec3(&case["location"]), &format!("{label} location"));
        same(throw.pitch, f64_of(&case["pitch"]), &format!("{label} pitch"));
        same(throw.yaw, f64_of(&case["yaw"]), &format!("{label} yaw"));
        same3(throw.velocity, vec3(&case["velocity"]), &format!("{label} velocity"));
        let (channel, actor, movement) = (field.channel().unwrap(), field.guid(), field.guid());
        assert_eq!((channel, actor, movement), (u32_of(&case["channel"]), u32_of(&case["actor"]), u32_of(&case["movement"])), "{label} ids");
        let mut rep = BitWriter::new();
        append_rep_movement(&mut rep, throw.location, throw.pitch, throw.yaw, throw.velocity);
        check(rep.finish(), &case["rep_movement"], &format!("{label} rep movement"));
        check(smoke_projectile_open_body(actor, movement, &throw), &case["open"], &format!("{label} open"));
        same3(ballistic_fallback(throw.location, throw.velocity), vec3(&case["fallback"]), &format!("{label} fallback"));
    }
    assert_eq!(field.next_channel, u32_of(&f["alloc"]["next_channel"]));
    assert_eq!(field.next_guid, u32_of(&f["alloc"]["next_guid"]));

    // rp:99-100: channel 32766 is the last one handed out
    let mut edge = ProjectileField { next_channel: 32766, ..ProjectileField::default() };
    assert_eq!(edge.channel().unwrap(), u32_of(&f["alloc_edge"][0]));
    assert_eq!(f["alloc_edge"][1], "RuntimeError");
    assert!(edge.channel().is_err());
}

#[test]
fn zone_opens_match_python() {
    let f = fixture("world_projectiles");
    for (k, case) in f["zones"].as_array().unwrap().iter().enumerate() {
        let result = smoke_zone_open_body(u32_of(&case["zone"]), vec3(&case["location"]), f64_of(&case["yaw"]));
        check(result, case, &format!("zone {k}"));
    }
}

#[test]
fn ballistic_fallback_and_hit_offset_match_python() {
    let f = fixture("world_projectiles");
    for (k, case) in f["fallback"].as_array().unwrap().iter().enumerate() {
        same3(ballistic_fallback(vec3(&case["location"]), vec3(&case["velocity"])), vec3(&case["result"]), &format!("fallback {k}"));
    }
    for (k, case) in f["hit_offset"].as_array().unwrap().iter().enumerate() {
        same3(smoke_location_from_hit(vec3(&case["location"]), vec3(&case["normal"])), vec3(&case["result"]), &format!("hit offset {k}"));
    }
}

#[test]
fn throw_transition_matches_python() {
    let f = fixture("world_projectiles");
    for case in f["thrown"].as_array().unwrap() {
        let (previous, current) = (case[0].as_str(), case[1].as_str());
        assert_eq!(smoke_thrown(previous, current), case[2].as_bool().unwrap(), "{previous:?} -> {current:?}");
    }
}

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
fn flight_stops_match_python() {
    let f = fixture("world_projectiles");
    for (k, case) in f["stops"].as_array().unwrap().iter().enumerate() {
        let throw = throw_transform([4786.03, -13285.84, 1427.98], -2.86, -130.79);
        same3(throw.location, vec3(&case["location"]), &format!("stop {k} location"));
        let mut flight = Flight::new(10000, 610100, 610102, f64_of(&case["opened"]), &throw);
        for (step, want) in case["steps"].as_array().unwrap().iter().enumerate() {
            let label = format!("stop {k} step {step}");
            let snapshot = snapshot(&want["snapshot"]);
            let stop = flight_stop_point(&mut flight, snapshot.as_ref(), f64_of(&want["now"]));
            match want["stop"].as_array() {
                None => assert!(stop.is_none(), "{label}: python kept flying, rust stopped {stop:?}"),
                Some(expected) => {
                    let (location, reason) = stop.unwrap_or_else(|| panic!("{label}: python stopped"));
                    same3(location, vec3(&expected[0]), &label);
                    assert_eq!(reason.name(), expected[1].as_str().unwrap(), "{label} reason");
                }
            }
            assert_eq!(flight.seen, want["seen"].as_bool().unwrap(), "{label} seen");
            match want["last_location"].is_null() {
                true => assert!(flight.last_location.is_none(), "{label} last location"),
                false => same3(flight.last_location.unwrap(), vec3(&want["last_location"]), &format!("{label} last location")),
            }
        }
    }
}
