//! the bot systems (spec 08 sections 4.3, 4.4) against server/fixtures/world_bot_systems.json:
//! ruh's own bot_shots and bot tick block, run per datagram on a fake camera.

mod golden_ab_common;

use aries_world::bots::{BotField, DEFAULT_BOT_COUNT, ShotOutcome, py_round};
use golden_ab_common::{Sequences, check_sends, f64_of, fixture, same, same3, u32_of, vec3};
use serde_json::{Value, json};

#[test]
fn shots_and_ticks_match_python() {
    let f = fixture("world_bot_systems");
    assert_eq!(f["lines"]["bot_tick"], 2034);
    assert_eq!(f["lines"]["bot_shots"], 969);
    assert_eq!(f["range_bot_count"].as_u64().unwrap() as usize, DEFAULT_BOT_COUNT);
    let mut sequences = Sequences::new(&f);
    let mut field: Option<BotField> = None;
    let mut kills = 0;
    for (k, datagram) in f["datagrams"].as_array().unwrap().iter().enumerate() {
        let now = f64_of(&datagram["now"]);
        let event = &datagram["event"];
        let label = format!("datagram {k}");

        // the mirror poll's shot (ruh:1631), only once the field exists
        let mut dummy_hits = Vec::new();
        let mut bot_hits = Vec::new();
        let mut missed = 0;
        if let Some(shot) = datagram["shot"].as_object() {
            let field = field.as_mut().expect("python shot before the first bot tick");
            let result = field
                .shots(shot["class_name"].as_str(), 1, vec3(&shot["origin"]), f64_of(&shot["pitch"]), f64_of(&shot["yaw"]), now)
                .unwrap();
            check_sends(&result.sends, &datagram["shot_sends"], &mut sequences, &format!("{label} shot"));
            for outcome in result.outcomes {
                match outcome {
                    ShotOutcome::Dummy { region, damage, distance } => dummy_hits.push(json!([region.name(), damage, py_round(distance)])),
                    ShotOutcome::Missed => missed += 1,
                    ShotOutcome::Bot { actor_guid, region, damage, life, distance, killed } => {
                        kills += killed as usize;
                        bot_hits.push(json!([actor_guid, region.name(), damage, py_round(life), py_round(distance)]));
                    }
                }
            }
        } else {
            assert!(datagram["shot_sends"].as_array().unwrap().is_empty());
        }
        let list = |key: &str| event.get(key).cloned().unwrap_or(json!([]));
        assert_eq!(json!(dummy_hits), list("dummy_hits"), "{label} dummy_hits");
        assert_eq!(json!(bot_hits), list("bot_hits"), "{label} bot_hits");
        assert_eq!(missed, event.get("bot_shot_missed").map_or(0, |v| v.as_u64().unwrap()), "{label} missed");

        // the bot tick (ruh:2034-2062)
        let field = field.get_or_insert_with(BotField::default);
        if field.tick_due(now) {
            let tick = field.tick(now).unwrap();
            check_sends(&tick.sends, &datagram["tick_sends"], &mut sequences, &format!("{label} tick"));
            let dummy = tick.dummy_spawned.map(|(channel, actor)| json!([channel, actor]));
            assert_eq!(dummy.unwrap_or(Value::Null), event.get("dummy_spawned").cloned().unwrap_or(Value::Null), "{label} dummy_spawned");
            assert_eq!(json!(tick.retired), list("bot_retired"), "{label} bot_retired");
            let spawned: Vec<Value> = tick.spawned.iter().map(|(channel, actor, location)| json!([channel, actor, location])).collect();
            assert_eq!(json!(spawned), list("bot_spawned"), "{label} bot_spawned");
        } else {
            assert!(datagram["tick_sends"].as_array().unwrap().is_empty(), "{label}: python ticked, rust gate closed");
        }
    }
    assert!(kills > 50, "only {kills} kills exercised");

    let field = field.unwrap();
    let last = &f["final"];
    let bots = last["bots"].as_array().unwrap();
    assert_eq!(field.bots.len(), bots.len());
    for (bot, want) in field.bots.iter().zip(bots) {
        assert_eq!(bot.actor_guid, u32_of(&want["actor_guid"]));
        same3(bot.location, vec3(&want["location"]), "final location");
        same(bot.life, f64_of(&want["life"]), "final life");
        assert_eq!(bot.alive, want["alive"].as_bool().unwrap());
        assert_eq!(bot.opened, want["opened"].as_bool().unwrap());
    }
    assert_eq!(field.next_channel, u32_of(&last["next_channel"]));
    assert_eq!(field.next_guid, u32_of(&last["next_guid"]));
    let mut field = field;
    same(field.rng.random(), f64_of(&last["rng_next"]), "rng after the run");
}
