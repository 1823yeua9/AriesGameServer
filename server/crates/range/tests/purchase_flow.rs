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
use aries_range::{LiveClient, LiveItemMachine, Outbox, RangeConfig, RangeServer, ResourceAmounts, SavedMoves, SlotMachine};
use serde_json::Value;

mod common;
use common::*;

/// what the scripted client shows the oracle.
#[derive(Default)]
struct World {
    machines: HashMap<u32, SlotMachine>,
    names: HashMap<u64, Vec<String>>,
    items: HashMap<u64, Vec<LiveItemMachine>>,
    amounts: HashMap<u64, BTreeMap<String, ResourceAmounts>>,
}

struct Scripted(Rc<RefCell<World>>);

impl LiveClient for Scripted {
    fn saved_moves(&mut self) -> Result<Option<SavedMoves>, String> {
        Ok(None)
    }
    fn latest_generated_input_event(&mut self) -> Result<Option<u32>, String> {
        Ok(None)
    }
    fn slot_machines(&mut self, slots: &[u32]) -> Result<Vec<SlotMachine>, String> {
        let world = self.0.borrow();
        Ok(slots.iter().filter_map(|slot| world.machines.get(slot).cloned()).collect())
    }
    fn state_names(&mut self, state_machine: u64) -> Result<Vec<String>, String> {
        Ok(self.0.borrow().names.get(&state_machine).cloned().unwrap_or_default())
    }
    fn item_machines(&mut self, item: u64) -> Result<Vec<LiveItemMachine>, String> {
        Ok(self.0.borrow().items.get(&item).cloned().unwrap_or_default())
    }
    fn resource_amounts(&mut self, item: u64, _resources: &[aries_world::equippables::ResourceSubobject]) -> Result<BTreeMap<String, ResourceAmounts>, String> {
        Ok(self.0.borrow().amounts.get(&item).cloned().unwrap_or_default())
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Sent {
    channel: u32,
    reliable: bool,
    open: bool,
    close: bool,
    exports: bool,
    body: Bits,
}

fn fixture(name: &str) -> (String, usize) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/decode_packets.json");
    let all: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let entry = all.as_array().unwrap().iter().find(|e| e["name"] == name).unwrap_or_else(|| panic!("fixture {name}"));
    (entry["hex"].as_str().unwrap().to_string(), entry["bit_count"].as_u64().unwrap() as usize)
}

fn vector(hex: &str, bits: usize) -> Bits {
    Bits::from_hex(hex, bits).unwrap()
}

struct Harness {
    server: RangeServer,
    client: Client,
    source: SocketAddr,
    out: Outbox,
    now: f64,
    sent: Vec<Sent>,
    events: PathBuf,
}

impl Harness {
    fn new(port: u16, world: Rc<RefCell<World>>) -> Self {
        let source: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let events = std::env::temp_dir().join(format!("aries_purchase_flow_{}_{port}.jsonl", std::process::id()));
        let mut config = RangeConfig::default();
        // no skin files: every gun opens unskinned, like the spec vectors
        config.skin_catalog = PathBuf::from("no_such_skin_catalog.json");
        config.skin_state = PathBuf::from("no_such_skin_state.json");
        let mut server = RangeServer::new(config, Box::new(FixedKeys), Some(&events));
        server.live = Box::new(Scripted(world));
        let mut client = Client::new(1000, 3000, 40);
        let mut out = Outbox::default();
        client.connect(&mut server, 1.0, source, &mut out);
        Self { server, client, source, out, now: 1.0, sent: Vec::new(), events }
    }

    fn collect(&mut self) {
        for (_, datagram) in self.out.datagrams.drain(..) {
            let (packet, _) = open_server_datagram(&datagram, &self.client.server_gcm).unwrap();
            if let Some(b) = inspect_first_bunch(&packet) {
                if b.is_close {
                    // the inspector stops at bClose: [reason SerializeInt(15)][paused][reliable][packed channel]
                    let mut r = aries_bits::BitReader::from_bits(&packet);
                    r.set_position(b.bunch_start_bit + 3);
                    r.read_serialize_int(15);
                    r.read_bool();
                    let reliable = r.read_bool();
                    let channel = r.read_packed_uint() as u32;
                    self.sent.push(Sent { channel, reliable, open: false, close: true, exports: false, body: Bits::empty() });
                    continue;
                }
                self.sent.push(Sent { channel: b.channel_index, reliable: b.reliable, open: b.is_open, close: b.is_close, exports: b.has_exports, body: slice(&packet, b.body_start_bit, b.end_bit) });
            }
        }
    }

    fn tick(&mut self) {
        self.now += 0.125;
        self.client.tick(&mut self.server, self.now, self.source, &mut self.out);
        self.collect();
    }

    fn run_until(&mut self, done: impl Fn(&RangeServer, SocketAddr) -> bool) {
        for _ in 0..2000 {
            self.tick();
            if done(&self.server, self.source) {
                return;
            }
        }
        panic!("never reached");
    }

    /// a captured packet inside the current session-ack window; returns its event.
    fn send_captured(&mut self, name: &str) -> Value {
        let (hex, bits) = fixture(name);
        let packet = self.client.captured(&hex, bits);
        let datagram = self.client.seal(&packet);
        self.server.handle_datagram(self.now, self.source, &datagram, &mut self.out);
        self.collect();
        self.last_event()
    }

    fn last_event(&mut self) -> Value {
        self.server.flush();
        let text = std::fs::read_to_string(&self.events).unwrap();
        serde_json::from_str(text.lines().last().unwrap()).unwrap()
    }

    /// every event so far with this key.
    fn events_with(&mut self, key: &str) -> Vec<Value> {
        self.server.flush();
        let text = std::fs::read_to_string(&self.events).unwrap();
        text.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()).filter_map(|e| e.get(key).cloned()).collect()
    }

    fn on(&self, channel: u32) -> Vec<&Sent> {
        self.sent.iter().filter(|s| s.channel == channel && s.reliable).collect()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.events);
    }
}

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

impl Harness {
    /// a ServerPurchase(path, slot 0, type 0) on ch5, built like the client does.
    fn send_purchase(&mut self, path: &str) -> Value {
        use aries_bits::BitWriter;
        // rpc params: [1][object ref: packed 1, flags 1, outer none, name][1][u8 slot][1][3 bits type]
        let (package, name) = path.rsplit_once('.').unwrap();
        let mut params = BitWriter::new();
        params.append_bits(1, 1);
        params.append_packed_uint(1);
        params.append_bits(1, 8);
        // outer: the package as its own named reference with no outer
        params.append_packed_uint(1);
        params.append_bits(1, 8);
        params.append_packed_uint(0);
        params.append_fstring_ascii(package);
        params.append_fstring_ascii(name);
        params.append_bits(1, 1);
        params.append_bits(0, 8);
        params.append_bits(1, 1);
        params.append_bits(0, 3);
        let params = params.finish().unwrap();
        let mut rpc = BitWriter::new();
        rpc.append_serialize_int(156, 179);
        rpc.append_packed_uint(params.bit_count as u64);
        rpc.append_bits_value(&params);
        let rpc = rpc.finish().unwrap();
        let body = aries_rep::rpc::controller_rpc_body(&rpc).unwrap();
        // one reliable ch5 bunch from the client
        let sequence = self.client.sequence;
        let mut w = BitWriter::new();
        w.append_u32((sequence << 18) | (self.client.acked << 4));
        w.append_u32(0);
        w.append_bits(1, 1);
        w.append_bits(0, 11);
        self.client.sequence = (sequence + 1) & 0x3FFF;
        w.append_bits(0, 1); // not control
        w.append_bits(0, 1); // paused
        w.append_bits(1, 1); // reliable
        w.append_packed_uint(5);
        w.append_bits(0, 4);
        w.append_bits(100, 12);
        w.append_bits(1, 1);
        w.append_packed_uint(102);
        w.append_bits(body.bit_count as u64, 13);
        w.append_bits_value(&body);
        w.append_bits(1, 1);
        let packet = w.finish().unwrap();
        let datagram = self.client.seal(&packet);
        self.server.handle_datagram(self.now, self.source, &datagram, &mut self.out);
        self.collect();
        self.last_event()
    }
}
