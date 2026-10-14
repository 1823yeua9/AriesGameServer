//! the scripted client shared by the harness tests.
#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::rc::Rc;

use aries_bits::{BitReader, BitWriter, Bits};
use aries_net::AresGcm;
use aries_net::client::{open_server_datagram, seal_client_packet};
use aries_net::control::inspect_first_bunch;
use aries_range::{Camera, KeySource, LiveClient, LiveItemMachine, Outbox, RangeConfig, RangeServer, ResourceAmounts, SavedMoves, SessionKeys, SlotMachine};
use aries_world::projectiles::ProjectileSnapshot;
use serde_json::Value;

pub const CLIENT_KEY: [u8; 32] = [0x20; 32];
pub const SERVER_KEY: [u8; 32] = [0x40; 32];
pub const STATELESS: u32 = 12;

pub struct FixedKeys;

impl KeySource for FixedKeys {
    fn read_keys(&mut self) -> Result<SessionKeys, String> {
        Ok(SessionKeys { client_send: CLIENT_KEY, server_send: SERVER_KEY })
    }
}

pub struct Client {
    pub gcm: AresGcm,
    pub server_gcm: AresGcm,
    pub sequence: u32,
    pub acked: u32,
    pub counter: u32,
}

fn packed(w: &mut BitWriter, value: u64) {
    w.append_packed_uint(value);
}

impl Client {
    pub fn header(&mut self, w: &mut BitWriter) {
        w.append_u32((self.sequence << 18) | (self.acked << 4));
        w.append_u32(0);
        w.append_bits(1, 1);
        w.append_bits(0, 11);
        self.sequence = (self.sequence + 1) & 0x3FFF;
    }

    /// an ack-only 77-bit packet.
    pub fn short(&mut self) -> Bits {
        let mut w = BitWriter::new();
        self.header(&mut w);
        w.append_bits(1, 1);
        w.finish().unwrap()
    }

    /// a reliable control bunch on channel 0 with an nmt message, padded to `total` bits.
    pub fn control(&mut self, message_type: u8, open: bool, total: usize) -> Bits {
        let mut w = BitWriter::new();
        self.header(&mut w);
        w.append_bits(1, 1); // control
        w.append_bool(open);
        w.append_bits(0, 1); // close
        w.append_bits(0, 1); // paused
        w.append_bits(1, 1); // reliable
        packed(&mut w, 0);
        w.append_bits(0, 4);
        w.append_bits(100, 12);
        w.append_bits(1, 1);
        packed(&mut w, 2);
        let body_bits = total - 1 - w.bit_count() - 13;
        w.append_bits(body_bits as u64, 13);
        w.append_bits(message_type as u64, 8);
        for _ in 8..body_bits {
            w.append_bits(0, 1);
        }
        w.append_bits(1, 1);
        let bits = w.finish().unwrap();
        assert_eq!(bits.bit_count, total);
        bits
    }

    pub fn seal(&mut self, packet: &Bits) -> Vec<u8> {
        self.counter += 1;
        seal_client_packet(packet, self.counter, STATELESS, &self.gcm)
    }
}

pub fn initial_datagram() -> Vec<u8> {
    let mut w = BitWriter::new();
    w.append_bits(0, 64);
    w.append_bits(1, 2);
    w.append_bits(3, 3);
    w.append_bits(1, 1);
    w.append_bits(0, 1);
    w.append_bits(4, 8);
    w.append_bits(4, 8);
    w.append_bits(0, 8);
    w.append_bits(1, 8);
    w.append_u32(0x1234);
    w.append_bits(0, 16);
    w.append_bits(0, 1);
    w.append_f64(0.0);
    for _ in 0..(160 + 128) / 32 {
        w.append_u32(0);
    }
    w.append_bits(1, 1);
    w.finish().unwrap().bytes
}

pub fn response_datagram(challenge: &[u8]) -> Vec<u8> {
    // echo the challenge's timestamp and cookie back as a type-2 response
    let mut r = BitReader::new(&challenge[4..], (challenge.len() - 4) * 8);
    r.set_position(2 + 3 + 2 + 8 + 8 + 8 + 8 + 32 + 16 + 1);
    let timestamp = r.read_f64();
    let cookie: Vec<u64> = (0..20).map(|_| r.read_bits(8)).collect();
    let mut w = BitWriter::new();
    w.append_bits(0, 64);
    w.append_bits(1, 2);
    w.append_bits(3, 3);
    w.append_bits(1, 1);
    w.append_bits(0, 1);
    w.append_bits(4, 8);
    w.append_bits(4, 8);
    w.append_bits(2, 8);
    w.append_bits(2, 8);
    w.append_u32(0x1234);
    w.append_bits(0, 16);
    w.append_bits(0, 1);
    w.append_f64(timestamp);
    for byte in cookie {
        w.append_bits(byte, 8);
    }
    for _ in 0..4 {
        w.append_u32(0);
    }
    w.append_bits(1, 1);
    w.finish().unwrap().bytes
}

impl Client {
    pub fn new(sequence: u32, acked: u32, counter: u32) -> Self {
        Self { gcm: AresGcm::new(&CLIENT_KEY), server_gcm: AresGcm::new(&SERVER_KEY), sequence, acked, counter }
    }

    /// a captured decoded packet with this client's header spliced over its first 64 bits.
    pub fn captured(&mut self, hex: &str, bit_count: usize) -> Bits {
        let source = Bits::from_hex(hex, bit_count).unwrap();
        let mut w = BitWriter::new();
        w.append_u32((self.sequence << 18) | (self.acked << 4));
        w.append_u32(0);
        self.sequence = (self.sequence + 1) & 0x3FFF;
        w.append_bits_value(&slice(&source, 64, bit_count));
        w.finish().unwrap()
    }

    /// stateless handshake, hello, nmt challenge, login and join.
    pub fn connect(&mut self, server: &mut RangeServer, now: f64, source: SocketAddr, out: &mut Outbox) {
        server.handle_datagram(now, source, &initial_datagram(), out);
        let (_, challenge) = out.datagrams.pop().unwrap();
        server.handle_datagram(now, source, &response_datagram(&challenge), out);
        let hello = self.control(0, true, 232);
        let short = self.short();
        let login = self.control(5, false, 600);
        let join = self.control(9, false, 142);
        for packet in [hello, short, login, join] {
            let d = self.seal(&packet);
            server.handle_datagram(now, source, &d, out);
        }
        out.datagrams.clear();
    }

    /// one ack-only packet at `now`.
    pub fn tick(&mut self, server: &mut RangeServer, now: f64, source: SocketAddr, out: &mut Outbox) {
        let short = self.short();
        let d = self.seal(&short);
        server.handle_datagram(now, source, &d, out);
    }
}

/// the bits of a packet between two positions.
pub fn slice(packet: &Bits, start: usize, end: usize) -> Bits {
    let mut r = BitReader::new(&packet.bytes, packet.bytes.len() * 8);
    r.set_position(start);
    r.read_payload(end - start)

}

/// what the scripted client shows the oracle.
#[derive(Default)]
pub struct World {
    pub machines: HashMap<u32, SlotMachine>,
    pub names: HashMap<u64, Vec<String>>,
    pub items: HashMap<u64, Vec<LiveItemMachine>>,
    pub amounts: HashMap<u64, BTreeMap<String, ResourceAmounts>>,
    pub camera: Option<Camera>,
    pub seed: Option<u32>,
    pub projectiles: HashMap<u32, ProjectileSnapshot>,
}

pub struct Scripted(pub Rc<RefCell<World>>);

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
    fn firing_seed(&mut self, _state_machine: u64, _key: &str) -> Option<u32> {
        self.0.borrow().seed
    }
    fn camera(&mut self) -> Result<Option<Camera>, String> {
        Ok(self.0.borrow().camera)
    }
    fn projectile(&mut self, channel: u32) -> Result<Option<ProjectileSnapshot>, String> {
        Ok(self.0.borrow().projectiles.get(&channel).cloned())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sent {
    pub channel: u32,
    pub reliable: bool,
    pub open: bool,
    pub close: bool,
    pub exports: bool,
    pub body: Bits,
}

pub fn fixture(name: &str) -> (String, usize) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/decode_packets.json");
    let all: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let entry = all.as_array().unwrap().iter().find(|e| e["name"] == name).unwrap_or_else(|| panic!("fixture {name}"));
    (entry["hex"].as_str().unwrap().to_string(), entry["bit_count"].as_u64().unwrap() as usize)
}

pub fn vector(hex: &str, bits: usize) -> Bits {
    Bits::from_hex(hex, bits).unwrap()
}

pub struct Harness {
    pub server: RangeServer,
    pub client: Client,
    pub source: SocketAddr,
    pub out: Outbox,
    pub now: f64,
    pub sent: Vec<Sent>,
    pub events: PathBuf,
    pub ticks: u64,
}

impl Harness {
    pub fn new(port: u16, world: Rc<RefCell<World>>) -> Self {
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
        Self { server, client, source, out, now: 1.0, sent: Vec::new(), events, ticks: 0 }
    }

    pub fn collect(&mut self) {
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

    pub fn tick(&mut self) {
        self.now += 0.125;
        self.client.tick(&mut self.server, self.now, self.source, &mut self.out);
        self.collect();
        // the 128 hz loop's per-tick work (lag compensation records)
        self.ticks += 16;
        self.server.tick(self.ticks, self.now);
    }

    pub fn run_until(&mut self, done: impl Fn(&RangeServer, SocketAddr) -> bool) {
        for _ in 0..2000 {
            self.tick();
            if done(&self.server, self.source) {
                return;
            }
        }
        panic!("never reached");
    }

    /// a captured packet inside the current session-ack window; returns its event.
    pub fn send_captured(&mut self, name: &str) -> Value {
        let (hex, bits) = fixture(name);
        let packet = self.client.captured(&hex, bits);
        let datagram = self.client.seal(&packet);
        self.server.handle_datagram(self.now, self.source, &datagram, &mut self.out);
        self.collect();
        self.last_event()
    }

    pub fn last_event(&mut self) -> Value {
        self.server.flush();
        let text = std::fs::read_to_string(&self.events).unwrap();
        serde_json::from_str(text.lines().last().unwrap()).unwrap()
    }

    /// every event so far with this key.
    pub fn events_with(&mut self, key: &str) -> Vec<Value> {
        self.server.flush();
        let text = std::fs::read_to_string(&self.events).unwrap();
        text.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()).filter_map(|e| e.get(key).cloned()).collect()
    }

    pub fn on(&self, channel: u32) -> Vec<&Sent> {
        self.sent.iter().filter(|s| s.channel == channel && s.reliable).collect()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.events);
    }
}


impl Harness {
    /// a ServerPurchase(path, slot 0, type 0) on ch5, built like the client does.
    pub fn send_purchase(&mut self, path: &str) -> Value {
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
