//! a bought vandal through every job stage, its registration with the mirror
//! and the first mirror pass, against the spec 05 section 11 vectors; then a
//! repeat purchase by package guid that replaces it.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::rc::Rc;

use aries_bits::Bits;
use aries_net::client::open_server_datagram;
use aries_net::control::inspect_first_bunch;
use aries_range::{LiveItemMachine, Outbox, RangeConfig, RangeServer, ResourceAmounts, SlotMachine};

mod common;
use common::*;

fn job_stage(server: &RangeServer, source: SocketAddr, index: usize) -> Option<u8> {
    server.connections[&source].authority.purchases.jobs.get(index).map(|j| j.stage)
}

/// positions of `wanted` in `have`, in order (each after the previous).
fn in_order(have: &[&Sent], wanted: &[Bits]) -> bool {
    let mut from = 0;
    for body in wanted {
        match have[from..].iter().position(|s| &s.body == body) {
            Some(i) => from += i + 1,
            None => return false,
        }
    }
    true
}

#[test]
fn vandal_purchase_job_and_mirror() {
    let world = Rc::new(RefCell::new(World::default()));
    let mut h = Harness::new(50620, world.clone());
    h.run_until(|s, source| s.connections[&source].bringup.starting_equippable_index >= aries_range::bringup::STARTING_EQUIPPABLES);

    // before anything: an unsupported path is logged and dropped
    let event = h.send_captured("purchase_deep_chain");
    assert_eq!(event["purchase_unsupported"], "L0.L1.L2");

    // stage 0a: the forced attachment cdos go out on ch8 first
    let event = h.send_captured("capture_1");
    assert_eq!(event["purchase_planned"], serde_json::json!(["Vandal", 20, 520]));
    let pre_export = vector(
        "020000008e100a96100a00b80000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e82965e82e8e8c2c6d0dacadce8e65ea4caccd8caf05ea4cac888dee85e82e8e8c2c6d0dacadce8be8296bea4caccd8caf0a0e4d2dac2e4f282e6e6cae800000000005800000088caccc2ead8e8bebe82e8e8c2c6d0dacadce8be8296bea4caccd8caf0a0e4d2dac2e4f282e6e6cae8be86000000000000",
        1305,
    );
    let last = h.sent.last().unwrap();
    assert_eq!((last.channel, last.exports, &last.body), (8, true, &pre_export));

    // stages 0..4 every 0.5 s, then stage 5 waits for the client's slot
    h.run_until(|s, source| s.connections[&source].authority.purchases.jobs[0].registered_tries > 0);
    assert_eq!(job_stage(&h.server, h.source, 0), Some(5));
    let ch20 = h.on(20);
    let open = vector(
        "020000002e100a36100a00620000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e82965e82e6e6c2ead8e8a4d2ccd8cabe829600000000003600000088caccc2ead8e8bebe82e6e6c2ead8e8a4d2ccd8cabe8296be86000000000022102e1014e00182f8d46ea01f6c85b002061137cf2d7b889e351ee6012ddca232bca453034ef3c8",
        1144,
    );
    let expected = [
        (open, true, true),
        (vector("cb94985a05", 35), false, false),
        (vector("a37c832f", 30), false, false),
        (vector("0200000042100a2210002e0000008ae2ead2e0e0c2c4d8caa6e8c2e8ca9ac2c6d0d2dcca00000000000a4128f109", 365), true, false),
        (vector("02000000a2100a2210001c00000092dcc2c6e8d2eccaa6e8c2e8ca00000000000a4128644d421506", 317), true, false),
    ];
    assert_eq!(ch20.len(), expected.len());
    for (sent, (body, exports, open)) in ch20.iter().zip(&expected) {
        assert_eq!((&sent.body, sent.exports, sent.open), (body, *exports, *open));
    }
    // ch8: pre-export, slot contents, correction 6 naming the vandal
    assert!(in_order(&h.on(8), &[pre_export, vector("f1a1074fc8da042028", 70), vector("e1950c130b43c94419c20709", 92)]));
    assert_eq!(h.server.connections[&h.source].correction_index, 6);
    assert_eq!(h.events_with("purchase_slotted").last().unwrap(), &serde_json::json!(["Vandal", 0, 6]));
    // the classic stays (the vandal goes in the primary slot)
    assert!(!h.server.connections[&h.source].authority.mirror.starting_pistol_replaced);

    // the client's slot now holds the vandal: register it with a zoom machine
    {
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
    let before = h.on(20).len();
    h.run_until(|s, source| job_stage(s, source, 0) == Some(7));
    assert_eq!(h.events_with("purchase_ready").last().unwrap(), &serde_json::json!(["Vandal", 5]));
    assert_eq!(h.events_with("purchase_extra_machines").last().unwrap(), &serde_json::json!(["ZoomStateMachine"]));
    let ch20 = h.on(20);
    assert_eq!(ch20[before].body, vector("02000000aa100a22100022000000b4dededaa6e8c2e8ca9ac2c6d0d2dcca0000000000aa4228f109", 317));
    assert_eq!(ch20[before + 1].body, vector("02000000b2100a22100024000000b4dededaa6e8c2e8ca92dcc2c6e8d2ecca0000000000aa4228644d121806", 349));
    let allocator = h.server.connections[&h.source].authority.purchases.allocator.clone().unwrap();
    assert_eq!((allocator.next_channel, allocator.next_guid), (21, 570));

    // the player equips it: ammo seeds, zoom and EquipState are confirmed in that order
    {
        let mut w = world.borrow_mut();
        let machine = w.machines.get_mut(&0).unwrap();
        machine.predicted = Some("EquipState".into());
        machine.is_current = true;
        w.items.get_mut(&0xA000).unwrap()[1].predicted = Some("ZoomStateUnzoomed".into());
        w.amounts.insert(
            0xA000,
            BTreeMap::from([
                ("MagazineAmmo".to_string(), ResourceAmounts { predicted: 25, authoritative: 0, default_amount: 25 }),
                ("ReserveAmmo".to_string(), ResourceAmounts { predicted: 75, authoritative: 0, default_amount: 75 }),
            ]),
        );
    }
    let before = h.on(20).len();
    h.tick();
    let ch20 = h.on(20);
    assert_eq!(ch20.len(), before + 4, "magazine, reserve, zoom, equip");
    assert_eq!(ch20[before].body, vector("020000004a100a2210001a0000009ac2cec2f4d2dcca82dadade00000000002a412866ad2bfa418519", 325));
    assert_eq!(ch20[before + 2].body, vector("02000000ba100a22100024000000b4dededaa6e8c2e8caaadcf4dededacac80000000000aa422864dd441d06", 349));
    assert_eq!(ch20[before + 3].body, vector("02000000ca100a221000160000008ae2ead2e0a6e8c2e8ca00000000000a4128644df21d06", 293));
    assert!(ch20[before..].iter().all(|s| s.exports));

    // a repeat purchase names the package by its guid; it replaces the vandal in slot 0
    let event = h.send_captured("spec_repeat_purchase");
    assert_eq!(event["purchase_resolved_path"], serde_json::json!(["<guid:525>.AssaultRifle_AK_C", "/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK.AssaultRifle_AK_C"]));
    assert_eq!(event["purchase_planned"], serde_json::json!(["Vandal", 21, 570]));
    h.run_until(|s, source| job_stage(s, source, 1).is_some_and(|stage| stage >= 5));
    let close = h.on(20).into_iter().last().unwrap().clone();
    assert!(close.close && close.body.bit_count == 0, "{:?}", h.on(20).iter().rev().take(3).map(|s| (s.open, s.close, s.exports, s.body.bit_count)).collect::<Vec<_>>());
    assert_eq!(h.events_with("purchase_replaced").last().unwrap(), &serde_json::json!(["AssaultRifle_AK_C", 20, 520]));
    let authority = &h.server.connections[&h.source].authority;
    assert!(authority.mirror.purchased.is_empty(), "the old vandal is no longer mirrored");
    assert_eq!(authority.purchased_class_guids.get("AssaultRifle_AK_C"), Some(&570));
}

#[test]
fn heavy_shields_armor_job() {
    let world = Rc::new(RefCell::new(World::default()));
    let mut h = Harness::new(50621, world);
    h.run_until(|s, source| s.connections[&source].bringup.starting_equippable_index >= aries_range::bringup::STARTING_EQUIPPABLES);
    // the purchase rpc for heavy shields is built with the client's own field layout
    let path = "/Game/Gear/HeavyArmorItem.HeavyArmorItem_C";
    let event = h.send_purchase(path);
    assert_eq!(event["purchase_planned"], serde_json::json!(["Heavy Shields", 20, 520]));
    h.run_until(|s, source| job_stage(s, source, 0) == Some(7));
    let ch20 = h.on(20);
    assert_eq!(ch20.len(), 2);
    assert!(ch20[0].open && ch20[0].exports);
    assert_eq!(ch20[0].body, vector("020000002e100a36100a00340000005e8ec2daca5e8ecac2e45e90cac2ecf282e4dadee492e8cada00000000003400000088caccc2ead8e8bebe90cac2ecf282e4dadee492e8cadabe86000000000022102e101400", 677));
    assert_eq!(ch20[1].body, vector("cb94985a05", 35));
    let shield = vector("02000000f2080a280028000000a6d0d2cad8c888c2dac2cecaa6cac6e8d2dedc0000000000ca23481ca415069ec91716f687ca5229", 422);
    assert!(in_order(&h.on(8), &[vector("a1a2074fc8da042028", 70), shield]));
    assert_eq!(h.events_with("purchase_shield").last().unwrap(), &serde_json::json!(["Heavy Shields", 50.0]));
}

/// A21/B18 with the player's real skin files (read only): the classic and the
/// knife open exactly as the world crate's decision builds them.
#[test]
fn saved_skins_open_the_classic_and_knife() {
    let backend = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../local_backend");
    let (catalog, state) = (backend.join("skin_catalog.json"), backend.join("offline_skin_state.json"));
    if !catalog.is_file() || !state.is_file() {
        eprintln!("skipped: no local_backend skin files");
        return;
    }
    let source: SocketAddr = "127.0.0.1:50622".parse().unwrap();
    let mut config = RangeConfig::default();
    config.skin_catalog = catalog.clone();
    config.skin_state = state.clone();
    let player = config.player_id.clone();
    let mut server = RangeServer::new(config, Box::new(FixedKeys), None);
    let mut client = Client::new(2000, 5000, 77);
    let mut out = Outbox::default();
    client.connect(&mut server, 1.0, source, &mut out);
    let mut now = 1.0;
    let mut opens: HashMap<u32, Bits> = HashMap::new();
    for _ in 0..2000 {
        now += 0.125;
        client.tick(&mut server, now, source, &mut out);
        for (_, datagram) in out.datagrams.drain(..) {
            let (packet, _) = open_server_datagram(&datagram, &client.server_gcm).unwrap();
            if let Some(b) = inspect_first_bunch(&packet).filter(|b| b.is_open && (b.channel_index == 9 || b.channel_index == 13)) {
                opens.insert(b.channel_index, slice(&packet, b.body_start_bit, b.end_bit));
            }
        }
        if opens.len() == 2 {
            break;
        }
    }
    let inventory = aries_world::skins::SkinInventory::load(&player, &catalog, Some(&state)).unwrap();
    let selection = aries_world::skins::select_cosmetic_loadout(&inventory, aries_world::cosmetics::BASE_PISTOL_WEAPON_ID).unwrap();
    let (classic, _) = aries_world::cosmetics::base_pistol_open(selection.as_ref()).unwrap();
    let (knife, _) = aries_world::melee::melee_open(Ok(&inventory)).unwrap();
    assert_eq!(opens[&9], classic);
    assert_eq!(opens[&13], knife);
}
