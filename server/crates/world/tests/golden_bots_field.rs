//! bot allocation, Random(1306) placement and every bot/dummy body (spec 08
//! sections 4.1, 4.2, 4.5) against server/fixtures/world_bots.json.

mod golden_ab_common;

use aries_world::bots::{
    BOT_CENTER_Z, Bot, BotField, FIRST_BOT_CHANNEL, FIRST_BOT_GUID, BOT_GUID_STRIDE, DUMMY_LOCATION, MANAGER_ORIGIN, PLAYER_TEAM, RED_TEAM,
    build_bot_alive_body, build_bot_health_body, build_bot_open, build_damage_section_body, build_dummy_health_body, build_dummy_open,
    build_update_damage_text_body, build_update_damage_text_payload,
};
use golden_ab_common::{check, f64_of, fixture, same, same3, u32_of, vec3};
use serde_json::Value;

fn bot_matches(bot: &Bot, case: &Value, label: &str) {
    assert_eq!(bot.index, case["index"].as_i64().unwrap(), "{label} index");
    assert_eq!(bot.channel, u32_of(&case["channel"]), "{label} channel");
    assert_eq!(bot.actor_guid, u32_of(&case["actor_guid"]), "{label} actor");
    assert_eq!(bot.health_guid, u32_of(&case["health_guid"]), "{label} health");
    assert_eq!(bot.handler_guid, u32_of(&case["handler_guid"]), "{label} handler");
    assert_eq!(bot.team_guid, u32_of(&case["team_guid"]), "{label} team guid");
    assert_eq!(bot.team, u32_of(&case["team"]), "{label} team");
    same3(bot.location, vec3(&case["location"]), &format!("{label} location"));
    same(bot.yaw, f64_of(&case["yaw"]), &format!("{label} yaw"));
    same(bot.life, f64_of(&case["life"]), &format!("{label} life"));
    assert_eq!(bot.alive, case["alive"].as_bool().unwrap(), "{label} alive");
}

fn bodies_match(bot: &Bot, bodies: &Value, label: &str) {
    check(build_bot_open(bot), &bodies["open"], &format!("{label} open"));
    check(build_bot_health_body(bot, true), &bodies["health_export"], &format!("{label} health export"));
    check(build_bot_health_body(bot, false), &bodies["health_update"], &format!("{label} health update"));
    check(build_bot_alive_body(bot), &bodies["alive_export"], &format!("{label} alive"));
}

#[test]
fn constants_match_python() {
    let c = fixture("world_bots")["constants"].clone();
    same(BOT_CENTER_Z, f64_of(&c["bot_center_z"]), "bot_center_z");
    let origin = c["manager_origin"].as_array().unwrap();
    same(MANAGER_ORIGIN.0, f64_of(&origin[0]), "origin x");
    same(MANAGER_ORIGIN.1, f64_of(&origin[1]), "origin y");
    same3(DUMMY_LOCATION, vec3(&c["dummy_location"]), "dummy location");
    assert_eq!(FIRST_BOT_CHANNEL, u32_of(&c["first_bot_channel"]));
    assert_eq!(FIRST_BOT_GUID, u32_of(&c["first_bot_guid"]));
    assert_eq!(BOT_GUID_STRIDE, u32_of(&c["bot_guid_stride"]));
    assert_eq!(PLAYER_TEAM, u32_of(&c["player_team"]));
    assert_eq!(RED_TEAM, u32_of(&c["red_team"]));
}

/// the production order: the dummy (its placement draws discarded), then 20 bots.
#[test]
fn dummy_then_twenty_bots_match_python() {
    let f = fixture("world_bots");
    let case = &f["placement_all_alive"];
    let entities = case["entities"].as_array().unwrap();
    assert_eq!(entities.len(), 21);
    let mut field = BotField::default();
    let dummy = field.allocate_dummy().unwrap().clone();
    bot_matches(&dummy, &entities[0], "dummy");
    check(build_dummy_open(&dummy), &entities[0]["bodies"]["open"], "dummy open");
    check(build_dummy_health_body(&dummy), &entities[0]["bodies"]["health"], "dummy health");
    for (k, want) in entities[1..].iter().enumerate() {
        let bot = field.allocate(field.bots.len() as i64).unwrap();
        field.bots.push(bot.clone());
        bot_matches(&bot, want, &format!("bot {k}"));
        bodies_match(&bot, &want["bodies"], &format!("bot {k}"));
    }
    assert_eq!(field.next_channel, u32_of(&case["next_channel"]));
    assert_eq!(field.next_guid, u32_of(&case["next_guid"]));
    same(field.rng.random(), f64_of(&case["rng_next"]), "rng after 20 bots");
}

/// kills, retirements and respawns: placement only avoids live bots.
#[test]
fn respawn_placement_matches_python() {
    let f = fixture("world_bots");
    let case = &f["placement_respawns"];
    let mut field = BotField::default();
    field.allocate_dummy().unwrap();
    let mut spawns = 0;
    for op in case["ops"].as_array().unwrap() {
        match op["op"].as_str().unwrap() {
            "spawn" => {
                let mut bot = field.allocate(field.bots.len() as i64).unwrap();
                bot.opened = true;
                bot_matches(&bot, &op["bot"], &format!("spawn {spawns}"));
                field.bots.push(bot);
                spawns += 1;
            }
            "kill" => field.bots[op["at"].as_u64().unwrap() as usize].alive = false,
            "retire" => {
                field.bots.remove(op["at"].as_u64().unwrap() as usize);
            }
            other => panic!("unknown op {other}"),
        }
    }
    assert!(spawns > 20, "only {spawns} spawns exercised");
    assert_eq!(field.next_channel, u32_of(&case["next_channel"]));
    assert_eq!(field.next_guid, u32_of(&case["next_guid"]));
    same(field.rng.random(), f64_of(&case["rng_next"]), "rng after respawns");
}

/// a crowded cone: rejected tries draw again, and 64 misses use the fallback spot.
#[test]
fn crowded_placement_matches_python() {
    let f = fixture("world_bots");
    let case = &f["placement_crowded"];
    let mut field = BotField::default();
    for (k, want) in case["bots"].as_array().unwrap().iter().enumerate() {
        let bot = field.allocate(field.bots.len() as i64).unwrap();
        same3(bot.location, vec3(&want["location"]), &format!("crowded {k}"));
        same(bot.yaw, f64_of(&want["yaw"]), &format!("crowded {k} yaw"));
        field.bots.push(bot);
    }
    same(field.rng.random(), f64_of(&case["rng_next"]), "rng after crowding");
}

fn base_bot() -> Bot {
    let mut field = BotField::default();
    let mut bot = field.allocate(0).unwrap();
    // rb.Bot(0, 401, 300108, 300110, <V.3 bot 0>, 300112, <its yaw>, team_guid=300114)
    bot.channel = 401;
    bot.actor_guid = 300108;
    bot.health_guid = 300110;
    bot.location = [4463.170774427629, -14407.674895086804, 1351.8];
    bot.handler_guid = 300112;
    bot.yaw = 65.86815966797721;
    bot.team_guid = 300114;
    bot
}

#[test]
fn health_and_alive_bodies_match_python() {
    let f = fixture("world_bots");
    for (k, case) in f["health"].as_array().unwrap().iter().enumerate() {
        let mut bot = base_bot();
        bot.life = f64_of(&case["life"]);
        bot.alive = case["alive"].as_bool().unwrap();
        check(build_bot_health_body(&bot, true), &case["export"], &format!("health {k} export"));
        check(build_bot_health_body(&bot, false), &case["update"], &format!("health {k} update"));
        check(build_bot_alive_body(&bot), &case["alive_export"], &format!("health {k} alive"));
    }
    for (k, case) in f["damage_sections"].as_array().unwrap().iter().enumerate() {
        let result = build_damage_section_body(
            u32_of(&case["actor"]),
            u32_of(&case["section"]),
            case["name"].as_str().unwrap(),
            f64::from_bits(case["life_bits"].as_u64().unwrap()),
            f64::from_bits(case["maximum_bits"].as_u64().unwrap()),
            case["alive"].as_bool().unwrap(),
            case["export"].as_bool().unwrap(),
        );
        check(result, case, &format!("damage section {k}"));
    }
}

#[test]
fn bot_open_guards_and_yaw_shorts_match_python() {
    let f = fixture("world_bots");
    for (k, case) in f["bot_open_cases"].as_array().unwrap().iter().enumerate() {
        let mut bot = base_bot();
        bot.channel = 400;
        bot.actor_guid = 300100;
        bot.health_guid = 300102;
        bot.handler_guid = 300104;
        bot.location = [4825.0, -14600.0, 1351.8];
        bot.yaw = f64_of(&case["yaw"]);
        bot.team_guid = u32_of(&case["team_guid"]);
        bot.team = u32_of(&case["team"]);
        check(build_bot_open(&bot), case, &format!("bot open case {k}"));
    }
}

#[test]
fn damage_text_matches_python() {
    let f = fixture("world_bots");
    let mut field = BotField::default();
    let dummy = field.allocate_dummy().unwrap().clone();
    for (k, case) in f["damage_text"].as_array().unwrap().iter().enumerate() {
        let damage = f64_of(&case["damage"]);
        let mut target = dummy.clone();
        if let Some(actor) = case.get("actor") {
            target.actor_guid = u32_of(actor);
        }
        check(build_update_damage_text_payload(damage), &case["payload"], &format!("damage text {k} payload"));
        check(build_update_damage_text_body(&target, damage), &case["body"], &format!("damage text {k} body"));
    }
}
