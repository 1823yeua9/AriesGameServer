//! bots, the spray-wall dummy, shots through lag compensation, cloudburst and
//! the ability jobs, driven through RangeServer against the spec 05 section 11
//! vectors.

use std::cell::RefCell;
use std::rc::Rc;

use aries_range::{Camera, LiveItemMachine, SlotMachine};
use aries_world::projectiles::ProjectileSnapshot;
use serde_json::json;

mod common;
use common::*;

const BOT0: [f64; 3] = [4463.170774427629, -14407.674895086804, 1351.8];
const DUMMY: [f64; 3] = [6190.0, -11640.0, 1270.0];

fn streaming_done(h: &mut Harness) {
    h.run_until(|s, source| s.connections[&source].authority.bots.as_ref().is_some_and(|b| b.field.bots.len() == 5));
}

/// the classic in slot 1, current, so its firing seed is polled.
fn hold_classic(world: &Rc<RefCell<World>>) {
    let mut w = world.borrow_mut();
    w.machines.insert(1, SlotMachine { slot: 1, item: 0x1000, class_name: "BasePistol_C".into(), state_machine: 0x2000, predicted: Some("IdleState".into()), is_current: true });
    w.seed = Some(0);
}

/// one firing-seed change, then enough ticks for the mirror poll to see it.
fn fire(h: &mut Harness, world: &Rc<RefCell<World>>) {
    {
        let mut w = world.borrow_mut();
        w.seed = Some(w.seed.unwrap_or(0) + 1);
    }
    h.tick();
}

fn shots_flow(port: u16, lagcomp: bool) -> Vec<serde_json::Value> {
    let world = Rc::new(RefCell::new(World::default()));
    let mut h = Harness::new(port, world.clone());
    h.server.config.lagcomp.enabled = lagcomp;
    streaming_done(&mut h);

    // opens and health bodies match the python's
    let dummy_open = vector("02000000969f480a9e9f480a005a0000005e8ec2daca5e90cae4decae65e88cac6def25ea8c2e4cecae8a0e4c2c6e8d2c6caa8e4c2d2dcd2dcce84dee800000000004a00000088caccc2ead8e8bebea8c2e4cecae8a0e4c2c6e8d2c6caa8e4c2d2dcd2dcce84dee8be86000000000012a348969f48149632c7039523670c050004", 1030);
    let dummy_health = vector("020000001aa3480a12a348002800000090cac2d8e8d088c2dac2cecaa6cac6e8d2dedc00000000006a8c2249acac1dc6d880ab43a6b03cb832", 454);
    let bot0_open = vector("04000000869f480a8e9f480a00580000005e8ec2daca5e86d0c2e4c2c6e8cae4e65ea8e4c2d2dcd2dcce84dee85ea8e4c2d2dcd2dcce84dee8bea08600000000003400000088caccc2ead8e8bebea8e4c2d2dcd2dcce84dee8bea086be8600000000004aa3480a32a348001200000084dedac4a8cac2da000000000032a348869f48149e62b962a6b9ce34e8b50b2a8d22297311b70f", 1197);
    let ch400: Vec<_> = h.on(400).into_iter().map(|s| (s.open, s.body.clone())).collect();
    assert_eq!(ch400, vec![(true, dummy_open), (false, dummy_health)]);
    assert_eq!(h.on(401)[0].body, bot0_open);
    assert!(h.on(401)[0].open);

    // a body shot from 200 units north of bot0, looking south
    hold_classic(&world);
    world.borrow_mut().camera = Some(Camera { location: [BOT0[0], BOT0[1] + 200.0, BOT0[2] + 15.0], rotation: [0.0, -90.0, 0.0] });
    h.tick(); // first seed read: no shot
    let before = h.on(401).len();
    fire(&mut h, &world);
    let ch401 = h.on(401);
    assert_eq!(ch401.len(), before + 1);
    assert_eq!(ch401[before].body, vector("75469124e6340c636d28d321bb071be803", 133), "health 74");
    // three more body shots kill it: health 0, then bAlive false (exported)
    for _ in 0..3 {
        fire(&mut h, &world);
    }
    let ch401 = h.on(401);
    let tail: Vec<_> = ch401[ch401.len() - 2..].iter().map(|s| (s.exports, s.body.clone())).collect();
    assert_eq!(
        tail,
        vec![
            (false, vector("75469124ae3b0b631e29d321bb071be803", 133)),
            (true, vector("0200000042a3480a32a348002e00000088c2dac2ceca90c2dcc8d8cae486dedae0dedccadce800000000000a8d2249c21205", 398)),
        ]
    );

    // the dummy shows damage numbers instead
    world.borrow_mut().camera = Some(Camera { location: [DUMMY[0], DUMMY[1] + 300.0, DUMMY[2] + 20.0], rotation: [0.0, -90.0, 0.0] });
    fire(&mut h, &world);
    assert_eq!(h.on(400).last().unwrap().body, vector("7ad69d85217523cf93e51900", 89), "damage text 26");

    // a new bot takes channel 406 at once (four alive); 1.5 s after death bot0 is closed
    h.run_until(|s, source| s.connections[&source].authority.bots.as_ref().is_some_and(|b| b.field.bots.iter().any(|bot| bot.channel == 406) && b.field.bots.iter().all(|bot| bot.channel != 401)));
    assert!(h.on(401).last().unwrap().close);
    assert_eq!(h.on(406)[0].body, vector("04000000869f480a8e9f480a00580000005e8ec2daca5e86d0c2e4c2c6e8cae4e65ea8e4c2d2dcd2dcce84dee85ea8e4c2d2dcd2dcce84dee8bea08600000000003400000088caccc2ead8e8bebea8e4c2d2dcd2dcce84dee8bea086be860000000000eaa3480ad2a348001200000084dedac4a8cac2da0000000000d2a348869f48149eea08e3fab4ce3428d910aa8f222983542c19", 1197));
    let mut outcomes = h.events_with("bot_hits");
    outcomes.extend(h.events_with("dummy_hits"));
    outcomes.extend(h.events_with("bot_retired"));
    outcomes
}

#[test]
fn bots_shots_and_respawn() {
    let outcomes = shots_flow(50630, true);
    assert_eq!(
        outcomes,
        vec![
            json!([[300108, "body", 26, 74, 172]]),
            json!([[300108, "body", 26, 48, 172]]),
            json!([[300108, "body", 26, 22, 172]]),
            json!([[300108, "body", 26, 0, 172]]),
            json!([["body", 26, 272]]),
            json!([300108]),
        ]
    );
}

/// static bots: lag compensation changes nothing (docs/port/10 section 4.7).
#[test]
fn lag_compensation_matches_python_hits() {
    assert_eq!(shots_flow(50631, true), shots_flow(50632, false));
}

#[test]
fn cloudburst_throw_stops_on_hit_and_opens_a_zone() {
    let world = Rc::new(RefCell::new(World::default()));
    let mut h = Harness::new(50633, world.clone());
    h.run_until(|s, source| s.connections[&source].bringup.starting_equippable_index >= aries_range::bringup::STARTING_EQUIPPABLES);
    let smoke = |predicted: &str| SlotMachine { slot: 3, item: 0x3000, class_name: "Ability_Wushu_4_Smoke_C".into(), state_machine: 0x4000, predicted: Some(predicted.into()), is_current: true };
    world.borrow_mut().machines.insert(3, smoke("InactiveState"));
    world.borrow_mut().camera = Some(Camera { location: [4825.0, -14600.0, 1415.8], rotation: [5.0, -90.0, 0.0] });
    h.tick();
    assert!(h.on(10000).is_empty());
    world.borrow_mut().machines.insert(3, smoke("ProjectileThrowState"));
    h.tick();
    let open = h.on(10000);
    assert_eq!(open.len(), 1);
    assert!(open[0].open && open[0].exports);
    assert_eq!(open[0].body, vector("040000004677940a4e77940a007a0000005e8ec2daca5e86d0c2e4c2c6e8cae4e65eaeeae6d0ea5ea6605e82c4d2d8d2e8f2be685ea0e4ded4cac6e8d2d8cabeaeeae6d0eabe68bea6daded6ca00000000004800000088caccc2ead8e8bebea0e4ded4cac6e8d2d8cabeaeeae6d0eabe68bea6daded6cabe860000000000da7a940ad27a9400280000008cd2dcd2e8caa6e0cacac89adeeccadacadce80000000000d27a94467794149eeaf1e26cb885375825210030ae1278bdf672847d8768726f462f623c17270106230aa2a9b61ea5125601", 1689));
    assert_eq!(h.events_with("cloudburst_open")[0], json!([10000, 610100, [4825, -14658, 1421], [0, -2827, 647], "A"]));

    // the client's projectile hits the floor: the zone opens 65 units up the normal
    world.borrow_mut().projectiles.insert(10000, ProjectileSnapshot { class_name: "Projectile_Wushu_4_Smoke_C".into(), location: Some([5000.0, -15000.0, 1260.0]), collisions: 1, hit_location: [5000.0, -15000.0, 1253.8], hit_normal: [0.0, 0.0, 1.0] });
    h.tick();
    assert_eq!(h.events_with("cloudburst_zone")[0], json!([10001, 610104, [5000, -15000, 1319], "hit"]));
    assert!(h.on(10001)[0].open);
    assert!(h.on(10000).last().unwrap().close, "the projectile closes with the stop");
    // the zone goes after 3.2 s
    h.run_until(|s, source| s.connections[&source].authority.projectiles.as_ref().is_some_and(|p| p.flights.is_empty()));
    assert!(h.on(10001).last().unwrap().close);
    assert_eq!(h.events_with("cloudburst_zone_closed"), vec![json!([10001])]);
}

#[test]
fn ability_jobs_register_with_the_mirror() {
    let world = Rc::new(RefCell::new(World::default()));
    let mut h = Harness::new(50634, world.clone());
    // the client's ability slots, with tailwind priming as an extra machine
    let abilities = [(3u32, "Ability_Wushu_4_Smoke_C"), (5, "Ability_Wushu_E_Dash_C"), (4, "Ability_Wushu_Q_CycloneBoost_C"), (9, "Ability_Wushu_X_Dagger_Production_C")];
    {
        let mut w = world.borrow_mut();
        for (i, (slot, class)) in abilities.iter().enumerate() {
            let (item, machine) = (0x5000 + i as u64 * 0x100, 0x9000 + i as u64 * 0x100);
            w.machines.insert(*slot, SlotMachine { slot: *slot, item, class_name: (*class).into(), state_machine: machine, predicted: Some("InactiveState".into()), is_current: false });
            w.names.insert(machine, ["InactiveState", "EquipState", "IdleState"].map(String::from).to_vec());
        }
        w.items.insert(
            0x5100,
            vec![LiveItemMachine { name: "StateMachine_Priming".into(), address: 0xA000, predicted: None, starting: Some("NotPrimed".into()), state_names: ["NotPrimed", "Primed"].map(String::from).to_vec() }],
        );
    }
    h.run_until(|s, source| s.connections[&source].authority.abilities.as_ref().is_some_and(|jobs| jobs.done()));
    // smoke's three setup bunches on ch14 are the python's
    let ch14: Vec<_> = h.on(14).into_iter().skip(2).map(|s| (s.exports, s.body.clone())).collect();
    assert_eq!(
        ch14[..3],
        [
            (false, vector("a3807b08", 30)),
            (true, vector("020000008236180a4205002e0000008ae2ead2e0e0c2c4d8caa6e8c2e8ca9ac2c6d0d2dcca00000000000ada60281101", 381)),
            (true, vector("020000008a36180a4205001c00000092dcc2c6e8d2eccaa6e8c2e8ca00000000000ada6028e539d0f80113", 341)),
        ]
    );
    let ready: Vec<_> = h.events_with("ability_ready").into_iter().flat_map(|v| v.as_array().unwrap().clone()).collect();
    assert_eq!(ready.len(), 4);
    assert!(ready.contains(&json!(["Ability_Wushu_E_Dash_C", 3, ["StateMachine_Priming"]])));
    let mirror = &h.server.connections[&h.source].authority.mirror;
    assert_eq!(mirror.abilities.len(), 4);
    let dash = mirror.abilities.iter().find(|e| e.key == "Ability_Wushu_E_Dash_C").unwrap();
    assert_eq!(dash.resources.len(), 1, "the charge resource carries over");
    assert!(mirror.states.get("Ability_Wushu_E_Dash_C/StateMachine_Priming").is_some_and(|s| s.active && s.auth == "NotPrimed"));

    // equipping smoke now mirrors its state
    {
        let mut w = world.borrow_mut();
        let smoke = w.machines.get_mut(&3).unwrap();
        smoke.predicted = Some("EquipState".into());
    }
    let before = h.on(14).len();
    h.tick();
    assert_eq!(h.on(14).len(), before + 1);
    let sent = h.events_with("sent_equippable_auth_states");
    assert_eq!(sent.last().unwrap()["Ability_Wushu_4_Smoke_C"], json!(["EquipState", "EquipState"]));
}
