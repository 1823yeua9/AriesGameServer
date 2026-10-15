//! fast pacing: bring-up, the range setup and purchases wait only for the
//! client's acks (plus the stage gap), and send exactly the python's bunches.

use std::cell::RefCell;
use std::rc::Rc;

use aries_bits::Bits;
use aries_range::bringup::{ITEM_SLOT_COUNT, STARTING_EQUIPPABLES, Stage, TEAM_STATES};
use aries_range::config::PacingSettings;
use aries_range::{LiveItemMachine, RangeServer, SlotMachine};

mod common;
use common::*;

fn fast(port: u16, world: Rc<RefCell<World>>) -> Harness {
    let mut config = Harness::config();
    config.pacing = PacingSettings::fast();
    let mut h = Harness::with_config(port, world, config);
    // a 64 fps client that acks everything it gets
    h.step = 1.0 / 64.0;
    h.auto_ack = true;
    h
}

fn bringup_done(server: &RangeServer, source: std::net::SocketAddr) -> bool {
    let bringup = &server.connections[&source].bringup;
    bringup.sent(Stage::MeleeInactiveState) && bringup.sent(Stage::RangeShop) && bringup.sent(Stage::ClientSetHud) && bringup.sent(Stage::RangeMoney)
}

#[test]
fn bringup_and_range_setup_take_seconds() {
    let world = Rc::new(RefCell::new(World::default()));
    let mut h = fast(50640, world);
    let start = h.now;
    h.run_until(bringup_done);
    let took = h.now - start;
    eprintln!("fast bring-up: {took:.3} s");
    // python: ~75 s to the knife
    assert!(took < 6.0, "bring-up took {took:.2} s");
    let conn = &h.server.connections[&h.source];
    let bringup = &conn.bringup;
    assert_eq!(bringup.item_slot_scale_index, ITEM_SLOT_COUNT);
    assert_eq!((bringup.team_state_open_index, bringup.team_state_link_index), (TEAM_STATES, TEAM_STATES));
    assert_eq!(bringup.starting_equippable_index, STARTING_EQUIPPABLES);
    assert!(conn.correction_index >= 5);
    for stage in [Stage::ViewTarget, Stage::PawnOwner, Stage::MatchState, Stage::GameplayStart, Stage::PlayerArray, Stage::PistolInactiveState] {
        assert!(bringup.sent(stage), "{stage:?}");
    }
    // the range hud, its sublevels and the settings request follow within a second
    h.run_until(|s, source| s.connections[&source].authority.settings_requested);
    let conn = &h.server.connections[&h.source];
    assert!(conn.authority.streaming_sent && conn.authority.range_hud_sent.is_some());
    eprintln!("fast range setup: {:.3} s", h.now - start);
    assert!(h.now - start < 7.0, "range setup done at {:.2} s", h.now - start);
}

#[test]
fn a_stage_waits_for_the_ack_of_the_last() {
    let world = Rc::new(RefCell::new(World::default()));
    let mut h = fast(50641, world);
    h.run_until(|s, source| s.connections[&source].bringup.sent(Stage::PawnOpen));
    // the client stops acking after it acks the pawn: GameStateBool goes out, then nothing
    h.auto_ack = false;
    h.run_until(|s, source| s.connections[&source].bringup.sent(Stage::GameStateBool));
    let frozen = h.client.acked;
    let sent = h.sent.len();
    for _ in 0..64 {
        h.tick();
    }
    assert_eq!(h.client.acked, frozen);
    let stages = h.sent[sent..].iter().filter(|s| s.reliable).count();
    assert_eq!(stages, 0, "a stage went out unacked");
    assert!(!h.server.connections[&h.source].bringup.sent(Stage::ControllerPlayerState));
    // acks resume: so does the chain
    h.auto_ack = true;
    h.run_until(|s, source| s.connections[&source].bringup.sent(Stage::ControllerPawn));
}

/// the purchase_flow vandal, fast: the same bunches on ch20 in the same order, done in a blink.
#[test]
fn vandal_purchase_is_quick_and_unchanged() {
    let world = Rc::new(RefCell::new(World::default()));
    let mut h = fast(50642, world.clone());
    h.run_until(|s, source| s.connections[&source].bringup.starting_equippable_index >= STARTING_EQUIPPABLES);
    {
        // the client's slot shows the vandal as soon as it is slotted
        let mut w = world.borrow_mut();
        w.machines.insert(0, SlotMachine { slot: 0, item: 0xA000, class_name: "AssaultRifle_AK_C".into(), state_machine: 0xB000, predicted: Some("InactiveState".into()), is_current: false });
        w.names.insert(0xB000, ["InactiveState", "EquipState", "IdleState", "FiringState", "ReloadState"].map(String::from).to_vec());
        let zoom = ["ZoomStateInactive", "ZoomStateUnzoomed", "ZoomStateLevel1"].map(String::from).to_vec();
        w.items.insert(
            0xA000,
            vec![
                LiveItemMachine { name: "EquippableStateMachine".into(), address: 0xB000, predicted: Some("InactiveState".into()), starting: Some("InactiveState".into()), state_names: vec![] },
                LiveItemMachine { name: "ZoomStateMachine".into(), address: 0xC000, predicted: Some("ZoomStateInactive".into()), starting: Some("ZoomStateInactive".into()), state_names: zoom },
            ],
        );
    }
    let event = h.send_captured("capture_1");
    assert_eq!(event["purchase_planned"], serde_json::json!(["Vandal", 20, 520]));
    let bought = h.now;
    h.run_until(|s, source| s.connections[&source].authority.purchases.jobs[0].stage == aries_range::purchases::STAGE_DONE);
    let took = h.now - bought;
    eprintln!("fast vandal: {took:.3} s");
    // python: 0.2 + 5 x 0.5 = 2.7 s
    assert!(took < 0.6, "purchase took {took:.3} s");
    assert_eq!(h.events_with("purchase_ready").last().unwrap(), &serde_json::json!(["Vandal", 5]));
    let bodies: Vec<&Bits> = h.on(20).iter().map(|s| &s.body).collect();
    let expected = [
        vector("cb94985a05", 35),
        vector("a37c832f", 30),
        vector("0200000042100a2210002e0000008ae2ead2e0e0c2c4d8caa6e8c2e8ca9ac2c6d0d2dcca00000000000a4128f109", 365),
        vector("02000000a2100a2210001c00000092dcc2c6e8d2eccaa6e8c2e8ca00000000000a4128644d421506", 317),
        vector("02000000aa100a22100022000000b4dededaa6e8c2e8ca9ac2c6d0d2dcca0000000000aa4228f109", 317),
        vector("02000000b2100a22100024000000b4dededaa6e8c2e8ca92dcc2c6e8d2ecca0000000000aa4228644d121806", 349),
    ];
    // [open, owner, role, sm open, inactive, zoom open, zoom state]
    assert_eq!(bodies.len(), 7);
    for (have, want) in bodies[1..].iter().zip(&expected) {
        assert_eq!(*have, want);
    }
    assert_eq!(h.events_with("purchase_slotted").last().unwrap(), &serde_json::json!(["Vandal", 0, 6]));
}

/// an empty state map is the client not done yet: fast registration waits for it.
#[test]
fn fast_registration_waits_for_the_state_map() {
    let world = Rc::new(RefCell::new(World::default()));
    let mut h = fast(50643, world.clone());
    h.run_until(|s, source| s.connections[&source].bringup.starting_equippable_index >= STARTING_EQUIPPABLES);
    world.borrow_mut().machines.insert(0, SlotMachine { slot: 0, item: 0xA000, class_name: "AssaultRifle_AK_C".into(), state_machine: 0xB000, predicted: None, is_current: false });
    h.send_captured("capture_1");
    h.run_until(|s, source| s.connections[&source].authority.purchases.jobs[0].registered_tries >= 3);
    assert_eq!(h.server.connections[&h.source].authority.purchases.jobs[0].stage, 5);
    world.borrow_mut().names.insert(0xB000, ["InactiveState", "EquipState"].map(String::from).to_vec());
    h.run_until(|s, source| s.connections[&source].authority.purchases.jobs[0].stage == aries_range::purchases::STAGE_DONE);
    assert_eq!(h.events_with("purchase_ready").last().unwrap(), &serde_json::json!(["Vandal", 2]));
}
