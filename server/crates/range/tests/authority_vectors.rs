//! the per-packet authority features end to end, against the spec 05 section
//! 11 vectors: range hud, level streaming, the settings request, ultimate
//! points, hud refresh and the input-correction responder.

use std::cell::RefCell;
use std::rc::Rc;

use aries_range::{HudWidget, InputReplication};
use serde_json::json;

mod common;
use common::*;

/// bodies sent on a channel, in order.
fn bodies(h: &Harness, channel: u32) -> Vec<aries_bits::Bits> {
    h.on(channel).into_iter().map(|s| s.body.clone()).collect()
}

/// `wanted` appear in `have` in this order (other bunches may sit between).
fn in_order(have: &[aries_bits::Bits], wanted: &[aries_bits::Bits]) -> bool {
    let mut from = 0;
    for body in wanted {
        match have[from..].iter().position(|b| b == body) {
            Some(i) => from += i + 1,
            None => return false,
        }
    }
    true
}

#[test]
fn authority_bodies_match_the_python() {
    let world = Rc::new(RefCell::new(World::default()));
    {
        let mut w = world.borrow_mut();
        w.ult = Some(8);
        w.hud = Some(HudWidget { collapsed: true, cinematic: false });
        w.replication = Some(InputReplication { stopped: true, requested_correction: 0, inventory_correction: 0, current_equippable_class: Some("BasePistol_C".into()) });
    }
    let mut h = Harness::new(50650, world.clone());
    // the client resumes after the first correction (it would be resent every 1 s otherwise)
    h.run_until(|s, source| s.connections[&source].authority.input_correction_sent.is_some());
    world.borrow_mut().replication.as_mut().unwrap().stopped = false;
    h.run_until(|s, source| s.connections[&source].authority.settings_requested);

    // ch4: GameModeClass and the HUD config (both exported) after the shop settings
    assert!(in_order(
        &bodies(&h, 4),
        &[
            vector("0200000016080a1e080a00680000005e8ec2daca5e8ec2daca9adec8cae65ea6d0dedee8d2dccea4c2dcceca5ea6d0dedee8d2dccea4c2dcceca8ec2daca9adec8ca000000000030000000a6d0dedee8d2dccea4c2dcceca8ec2daca9adec8cabe86000000000016b22e94a902", 868),
            vector("0200000026080a2e080a007e0000005e8ec2daca5eaa925e92dc8ec2daca5e90aa885e8ec2dacaa6e8c2e8ca90aa8886dedcccd2cee65ea6d0dedee8d2dccea4c2dcceca90aa8886dedcccd2ce000000000032000000a6d0dedee8d2dccea4c2dcceca90aa8886dedcccd2cebe86000000000016b22e640e05", 964),
        ]
    ));
    // ch5: ClientSetHUD again, streaming, flush, then the settings request
    assert!(in_order(
        &bodies(&h, 5),
        &[
            vector("c2a88cad03", 34),
            vector("1641108685b58ff5d07e4e5537a9c973e9e0f1254a5fb1042396696a0dfa39d0c9b9c52f9383c0d60f306b8c3572513bbaf64ab54f9e5f4b8500c526b6b901a643bb14adcb90313b618654e1cc4d226b915cd5569d6d2be4b1d3a58355bb9ef88a15af973bf5c4679f46700b98bc70930913db5c6ce927d8fc8c99af6c93b5e6f0738f2090a20a", 1076),
            vector("82c4c202", 26),
            vector("8290c002", 26),
        ]
    ));
    // ch6: NumUltimatePoints 8, then SpawnedCharacter null and back to the pawn
    assert!(in_order(&bodies(&h, 6), &[vector("cb598fb04dcb7a4c07", 67), vector("0b59d7a28e01", 43), vector("0b5953a08e01", 43)]));
    assert_eq!(h.events_with("sent_ultimate_points"), vec![json!(8)]);
    // ch8: the input-resume correction names the classic (94) at index 6
    assert!(in_order(&bodies(&h, 8), &[vector("e1150c230b43b8cb7aa60f", 84)]));
    assert_eq!(h.events_with("sent_input_resume_correction")[0], json!(6));

    // a second stop with no current equippable, at least 1 s later: index 7, no guid
    world.borrow_mut().replication = Some(InputReplication { stopped: true, requested_correction: 0, inventory_correction: 0, current_equippable_class: None });
    h.run_until(|s, source| s.connections[&source].correction_index == 7);
    assert!(in_order(&bodies(&h, 8), &[vector("e1150c230b43b8cb7aa60f", 84), vector("e115b3c6c523c59000", 68)]));

    // the hud refresh stops after six sends
    for _ in 0..80 {
        h.tick();
    }
    assert_eq!(h.events_with("sent_hud_refresh").last(), Some(&json!(6)));
}
