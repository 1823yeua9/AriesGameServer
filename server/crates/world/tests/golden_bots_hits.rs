//! the bot hit model (spec 08 section 4.4) and the cpython float helpers it
//! depends on, against server/fixtures/world_bots.json.

mod golden_ab_common;

use aries_world::bots::{BotField, Region, dummy_hit, first_hit, hit_region, py_hypot, py_sum, shot_damage, view_ray};
use golden_ab_common::{bits_field, f64_of, fields, fixture, float_field, optional, same, same3, u32_of, vec3};

fn region(name: &str) -> Region {
    match name {
        "head" => Region::Head,
        "body" => Region::Body,
        "legs" => Region::Legs,
        other => panic!("region {other}"),
    }
}

fn expect_hit(actual: Option<(Region, f64)>, region_text: &str, distance_text: &str, label: &str) {
    match optional(region_text) {
        None => assert!(actual.is_none(), "{label}: python missed, rust hit {actual:?}"),
        Some(name) => {
            let (got, distance) = actual.unwrap_or_else(|| panic!("{label}: python hit {name}, rust missed"));
            assert_eq!(got, region(name), "{label} region");
            same(distance, float_field(distance_text), &format!("{label} distance"));
        }
    }
}

#[test]
fn view_rays_match_python() {
    let f = fixture("world_bots");
    for case in f["view_ray"].as_array().unwrap() {
        let (pitch, yaw) = (f64_of(&case["pitch"]), f64_of(&case["yaw"]));
        same3(view_ray(pitch, yaw), vec3(&case["dir"]), &format!("view_ray({pitch}, {yaw})"));
    }
}

#[test]
fn hit_regions_match_python() {
    let f = fixture("world_bots");
    let rows = f["hit_region"].as_array().unwrap();
    assert!(rows.len() > 2000);
    let mut hits = 0;
    for (k, row) in rows.iter().enumerate() {
        let parts = fields(row);
        assert_eq!(parts.len(), 11);
        let v: Vec<f64> = parts[..9].iter().map(|p| float_field(p)).collect();
        let result = hit_region([v[0], v[1], v[2]], [v[3], v[4], v[5]], [v[6], v[7], v[8]]);
        hits += result.is_some() as usize;
        expect_hit(result, parts[9], parts[10], &format!("hit row {k}"));
    }
    assert!(hits > rows.len() / 4, "only {hits} of {} rows hit", rows.len());
}

#[test]
fn first_hit_and_dummy_hit_match_python() {
    let f = fixture("world_bots");
    let case = &f["first_hit"];
    // the same field python built: dummy then 5 bots (allocate, not appended)
    let mut field = BotField::default();
    let mut dummy = field.allocate_dummy().unwrap().clone();
    let mut bots: Vec<_> = (0..5).map(|i| field.allocate(i).unwrap()).collect();
    same3(dummy.location, vec3(&case["dummy"]["location"]), "dummy");
    for (bot, want) in bots.iter().zip(case["bots"].as_array().unwrap()) {
        same3(bot.location, vec3(&want["location"]), "bot location");
    }
    for (k, row) in case["cases"].as_array().unwrap().iter().enumerate() {
        let parts = fields(row);
        assert_eq!(parts.len(), 16, "row {k}");
        for (bot, flags) in bots.iter_mut().zip(&parts[..5]) {
            bot.alive = flags.as_bytes()[0] == b'1';
            bot.opened = flags.as_bytes()[1] == b'1';
        }
        dummy.opened = parts[5] == "1";
        let origin = [float_field(parts[6]), float_field(parts[7]), float_field(parts[8])];
        let direction = view_ray(float_field(parts[9]), float_field(parts[10]));
        let best = first_hit(&bots, origin, direction);
        match optional(parts[11]) {
            None => assert!(best.is_none(), "row {k}: python none, rust {best:?}"),
            Some(index) => {
                let (got, got_region, distance) = best.unwrap_or_else(|| panic!("row {k}: python hit bot {index}"));
                assert_eq!(got, index.parse::<usize>().unwrap(), "row {k} bot");
                assert_eq!(got_region, region(parts[12]), "row {k} region");
                same(distance, float_field(parts[13]), &format!("row {k} distance"));
            }
        }
        expect_hit(dummy_hit(Some(&dummy), origin, direction), parts[14], parts[15], &format!("row {k} dummy"));
    }
    assert!(f["dummy_hit_none"].is_null());
    assert!(dummy_hit(None, [0.0; 3], [1.0, 0.0, 0.0]).is_none());
}

#[test]
fn shot_damage_matches_python() {
    let f = fixture("world_bots");
    for case in f["shot_damage"].as_array().unwrap() {
        let class = case[0].as_str();
        assert_eq!(shot_damage(class, region(case[1].as_str().unwrap())), u32_of(&case[2]), "{class:?} {}", case[1]);
    }
}

#[test]
fn cpython_hypot_matches() {
    let f = fixture("world_bots");
    for row in f["hypot"].as_array().unwrap() {
        let parts = fields(row);
        let (x, y, want) = (bits_field(parts[0]), bits_field(parts[1]), bits_field(parts[2]));
        same(py_hypot(x, y), want, &format!("hypot({x:?}, {y:?})"));
    }
}

#[test]
fn cpython_sum_matches() {
    let f = fixture("world_bots");
    for row in f["py_sum"].as_array().unwrap() {
        let values: Vec<f64> = fields(row).iter().map(|p| bits_field(p)).collect();
        let (want, values) = values.split_last().unwrap();
        same(py_sum(values), *want, &format!("sum({values:?})"));
    }
}
