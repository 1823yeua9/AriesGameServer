//! read_firing_seed's pick of the firing state (spec 08 section 4.4) against
//! server/fixtures/world_firing_seed.json: ruh's own function run on fake reads
//! that turn the picked state's address into the seed.

mod golden_ab_common;

use aries_world::bots::firing_state_address;
use golden_ab_common::fixture;

#[test]
fn firing_state_pick_matches_python() {
    let f = fixture("world_firing_seed");
    assert_eq!(f["line"], 946);
    let cases = f["cases"].as_array().unwrap();
    assert!(cases.len() > 300);
    let mut picked = 0;
    for (k, case) in cases.iter().enumerate() {
        let states: Vec<(String, u64)> = case["states"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pair| (pair[0].as_str().unwrap().to_string(), pair[1].as_u64().unwrap()))
            .collect();
        let want = case["address"].as_u64();
        assert_eq!(firing_state_address(&states), want, "case {k}: {states:?}");
        picked += want.is_some() as usize;
    }
    assert!(picked > 100 && picked < cases.len() - 50, "{picked} picks: both outcomes must be exercised");
}
