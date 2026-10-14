//! the debug queue's spawn_bot (spec 08 section 4.7, spec 05 6.11) against
//! server/fixtures/world_bot_queue.json: ruh's own queue statement between
//! bot_shots and the bot tick, per datagram in production order.

mod golden_ab_common;

use aries_world::bots::{BotField, Camera, ShotOutcome, py_round};
use golden_ab_common::{Sequences, check_sends, f64_of, fixture, same, same3, u32_of, vec3};
use serde_json::{Value, json};

#[test]
fn queued_bots_match_python() {
    let f = fixture("world_bot_queue");
    assert_eq!(f["lines"]["spawn_bot"], 1958);
    assert_eq!(f["lines"]["bot_tick"], 2034);
    let mut sequences = Sequences::new(&f);
    let mut field: Option<BotField> = None;
    let (mut queued_total, mut hits) = (0, 0);
    for (k, datagram) in f["datagrams"].as_array().unwrap().iter().enumerate() {
        let now = f64_of(&datagram["now"]);
        let event = &datagram["event"];
        let label = format!("datagram {k}");
        let list = |key: &str| event.get(key).cloned().unwrap_or(json!([]));

        // the mirror poll's shot (only once a field exists)
        let mut bot_hits = Vec::new();
        if let Some(shot) = datagram["shot"].as_object() {
            let field = field.as_mut().expect("python shot before any field");
            let camera = Camera { location: vec3(&shot["origin"]), pitch: f64_of(&shot["pitch"]), yaw: f64_of(&shot["yaw"]), roll: 0.0 };
            let result = field.shots(shot["class_name"].as_str(), 1, &camera, now);
            assert!(result.error.is_none(), "{label}: {:?}", result.error);
            check_sends(&result.sends, &datagram["shot_sends"], &mut sequences, &format!("{label} shot"));
            for outcome in result.outcomes {
                if let ShotOutcome::Bot { actor_guid, region, damage, life, distance, .. } = outcome {
                    bot_hits.push(json!([actor_guid, region.name(), damage, py_round(life), py_round(distance)]));
                }
            }
        }
        assert_eq!(json!(bot_hits), list("bot_hits"), "{label} bot_hits");
        hits += bot_hits.len();

        // the command queue (ruh:1958-1971): the field is made here when the tick has not run yet
        let mut spawned = Vec::new();
        let mut queue_sends = Vec::new();
        for location in datagram["queued"].as_array().unwrap() {
            let field = field.get_or_insert_with(BotField::default);
            let tick = field.spawn_at(vec3(location));
            assert!(tick.error.is_none(), "{label}: {:?}", tick.error);
            assert!(tick.dummy_spawned.is_none() && tick.retired.is_empty());
            spawned.extend(tick.spawned.iter().map(|(channel, actor, location)| json!([channel, actor, location])));
            queue_sends.extend(tick.sends);
            queued_total += 1;
        }
        check_sends(&queue_sends, &datagram["queue_sends"], &mut sequences, &format!("{label} queue"));

        // the bot tick (ruh:2034-2062)
        let field = field.get_or_insert_with(BotField::default);
        if field.tick_due(now) {
            let tick = field.tick(now);
            assert!(tick.error.is_none(), "{label}: {:?}", tick.error);
            check_sends(&tick.sends, &datagram["tick_sends"], &mut sequences, &format!("{label} tick"));
            let dummy = tick.dummy_spawned.map(|(channel, actor)| json!([channel, actor]));
            assert_eq!(dummy.unwrap_or(Value::Null), event.get("dummy_spawned").cloned().unwrap_or(Value::Null), "{label} dummy_spawned");
            assert_eq!(json!(tick.retired), list("bot_retired"), "{label} bot_retired");
            spawned.extend(tick.spawned.iter().map(|(channel, actor, location)| json!([channel, actor, location])));
        } else {
            assert!(datagram["tick_sends"].as_array().unwrap().is_empty(), "{label}: python ticked, rust gate closed");
        }
        // queue and tick append to one bot_spawned list
        assert_eq!(json!(spawned), list("bot_spawned"), "{label} bot_spawned");
    }
    assert!(queued_total > 30 && hits > 100, "{queued_total} queued, {hits} hits");

    let field = field.unwrap();
    let last = &f["final"];
    let bots = last["bots"].as_array().unwrap();
    assert_eq!(field.bots.len(), bots.len());
    for (bot, want) in field.bots.iter().zip(bots) {
        assert_eq!(bot.channel, u32_of(&want["channel"]));
        assert_eq!(bot.actor_guid, u32_of(&want["actor_guid"]));
        same3(bot.location, vec3(&want["location"]), "final location");
        same(bot.yaw, f64_of(&want["yaw"]), "final yaw");
        same(bot.life, f64_of(&want["life"]), "final life");
        assert_eq!(bot.alive, want["alive"].as_bool().unwrap());
    }
    assert_eq!(field.next_channel, u32_of(&last["next_channel"]));
    assert_eq!(field.next_guid, u32_of(&last["next_guid"]));
    let mut field = field;
    same(field.rng.random(), f64_of(&last["rng_next"]), "rng after the run");
}
