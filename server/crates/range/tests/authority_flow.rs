//! per-packet work after ClientRestart, driven with captured client packets
//! from the golden decode fixtures: the input-ack trial and the range
//! settings apply (infinite ammo).

use std::net::SocketAddr;
use std::path::PathBuf;

use aries_bits::Bits;
use aries_net::client::open_server_datagram;
use aries_net::control::inspect_first_bunch;
use aries_range::bringup::Stage;
use aries_range::{LiveClient, Outbox, RangeConfig, RangeServer, SavedMoves};
use serde_json::Value;

mod common;
use common::*;

struct ScriptedLive {
    number: u32,
    bound: Option<u32>,
}

impl LiveClient for ScriptedLive {
    fn saved_moves(&mut self) -> Result<Option<SavedMoves>, String> {
        self.number += 15;
        // walking +x at 120 u per move-ack read
        let x = self.number as f64 * 8.0;
        Ok(Some(SavedMoves { newest: self.number, count: 20, newest_location: Some([x, -12019.322, 1294.153]), ..Default::default() }))
    }

    fn latest_generated_input_event(&mut self) -> Result<Option<u32>, String> {
        Ok(self.bound)
    }
}

/// one captured decoded packet from fixtures/decode_packets.json.
fn fixture(name: &str) -> (String, usize) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/decode_packets.json");
    let all: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let entry = all.as_array().unwrap().iter().find(|e| e["name"] == name).unwrap_or_else(|| panic!("fixture {name}"));
    (entry["hex"].as_str().unwrap().to_string(), entry["bit_count"].as_u64().unwrap() as usize)
}

struct Harness {
    server: RangeServer,
    client: Client,
    source: SocketAddr,
    out: Outbox,
    now: f64,
    events: PathBuf,
}

impl Harness {
    fn new(port: u16, bound: Option<u32>) -> Self {
        let source: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        let events = std::env::temp_dir().join(format!("aries_authority_flow_{}_{port}.jsonl", std::process::id()));
        let mut server = RangeServer::new(RangeConfig::default(), Box::new(FixedKeys), Some(&events));
        server.live = Box::new(ScriptedLive { number: 186, bound });
        let mut client = Client::new(500, 2000, 9);
        let mut out = Outbox::default();
        client.connect(&mut server, 1.0, source, &mut out);
        Self { server, client, source, out, now: 1.0, events }
    }

    /// session ticks every 125 ms until `done`.
    fn run_until(&mut self, done: impl Fn(&RangeServer, SocketAddr) -> bool) {
        for _ in 0..2000 {
            self.now += 0.125;
            self.client.tick(&mut self.server, self.now, self.source, &mut self.out);
            self.out.datagrams.clear();
            if done(&self.server, self.source) {
                return;
            }
        }
        panic!("never reached");
    }

    /// sends a captured packet inside the same session-ack window (no stage or
    /// move ack), returns every (channel, reliable, body) sent and the event.
    fn send_captured(&mut self, name: &str) -> (Vec<(u32, bool, Bits)>, Value) {
        let (hex, bits) = fixture(name);
        let packet = self.client.captured(&hex, bits);
        let datagram = self.client.seal(&packet);
        self.server.handle_datagram(self.now, self.source, &datagram, &mut self.out);
        let sent = self
            .out
            .datagrams
            .drain(..)
            .filter_map(|(_, d)| {
                let (packet, _) = open_server_datagram(&d, &self.client.server_gcm).unwrap();
                inspect_first_bunch(&packet).map(|b| (b.channel_index, b.reliable, slice(&packet, b.body_start_bit, b.end_bit)))
            })
            .collect();
        self.server.flush();
        let text = std::fs::read_to_string(&self.events).unwrap();
        let event = serde_json::from_str(text.lines().last().unwrap()).unwrap();
        (sent, event)
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.events);
    }
}

#[test]
fn input_ack_acks_wire_event_ids_once() {
    let mut h = Harness::new(50610, Some(600));
    h.run_until(|s, source| s.connections[&source].bringup.sent(Stage::ClientRestart));

    // first bunch on ch8 with events 501 (activation) and 502
    let (sent, event) = h.send_captured("trailer_overrun_1");
    let acks: Vec<_> = sent.iter().filter(|(channel, reliable, _)| *channel == 8 && !reliable).collect();
    assert_eq!(acks.len(), 1, "{sent:?}");
    assert_eq!(acks[0].2, aries_rep::setup::input_ack_body(502).unwrap());
    assert_eq!(event["input_ack_received_event_id"], 502);
    assert_eq!(event["received_activation_events"], serde_json::json!([{"event_id": 501, "slot": 3, "state": 4, "action": 0}]));
    assert_eq!(h.server.connections[&h.source].moves.last_input_event, Some(502));

    // the same ids again: logged, not re-acked
    let (sent, event) = h.send_captured("trailer_overrun_1");
    assert!(sent.iter().all(|(channel, reliable, _)| *channel != 8 || *reliable));
    assert!(event.get("input_ack_received_event_id").is_none());
    assert!(event.get("received_activation_events").is_some());

    // past the client's generated bound: rejected
    h.server.live = Box::new(ScriptedLive { number: 99_999, bound: Some(502) });
    let (sent, event) = h.send_captured("trailer_overrun_2");
    assert!(sent.iter().all(|(channel, reliable, _)| *channel != 8 || *reliable));
    assert_eq!(event["input_ack_rejected_event_id"], 503);
    assert!(event.get("received_activation_events").is_none());
}

#[test]
fn input_ack_needs_a_generated_bound() {
    let mut h = Harness::new(50611, None);
    h.run_until(|s, source| s.connections[&source].bringup.sent(Stage::ClientRestart));
    let (sent, event) = h.send_captured("trailer_overrun_1");
    assert!(sent.iter().all(|(channel, reliable, _)| *channel != 8 || *reliable));
    assert_eq!(event["input_ack_rejected_event_id"], 502);
}

#[test]
fn range_settings_apply_infinite_ammo() {
    let mut h = Harness::new(50612, Some(0));
    // before streaming the settings rpcs are not read at all
    h.run_until(|s, source| s.connections[&source].bringup.sent(Stage::ClientRestart));
    let (sent, event) = h.send_captured("settings");
    assert!(event.get("received_range_settings").is_none());
    assert!(sent.iter().all(|(channel, _, _)| *channel != 4));

    h.run_until(|s, source| s.connections[&source].authority.settings_requested);
    // 176 (infinite ammo on, bot amount 1) + 177; dedup is by channel sequence
    // and the pre-streaming copy was never recorded
    let (sent, event) = h.send_captured("settings");
    assert_eq!(
        event["received_range_settings"],
        serde_json::json!([
            {"field": 176, "channel_sequence": 3534, "infinite_ammo": true, "bot_amount_index": 1},
            {"field": 177, "channel_sequence": 3535, "is_init_settings_flow": true},
        ])
    );
    assert_eq!(event["sent_infinite_ammo"], true);
    let ammo: Vec<_> = sent.iter().filter(|(channel, reliable, _)| *channel == 4 && *reliable).collect();
    assert_eq!(ammo.len(), 1);
    assert_eq!(ammo[0].2, aries_rep::setup::infinite_ammo_body(true).unwrap());
    let state = &h.server.connections[&h.source].authority;
    assert_eq!(state.infinite_ammo_sent, Some(true));
    assert_eq!(state.settings.as_ref().map(|s| s.bot_amount_index), Some(1));

    // a retransmitted bunch (same channel sequences) is dropped
    let (sent, event) = h.send_captured("settings");
    assert!(event.get("received_range_settings").is_none());
    assert!(sent.iter().all(|(channel, _, _)| *channel != 4));

    // an absent struct: every member default, infinite ammo goes back off
    let (sent, event) = h.send_captured("settings_176_absent");
    assert_eq!(event["received_range_settings"], serde_json::json!([{"field": 176, "channel_sequence": 41}]));
    assert_eq!(event["sent_infinite_ammo"], false);
    assert_eq!(sent.iter().filter(|(channel, _, body)| *channel == 4 && *body == aries_rep::setup::infinite_ammo_body(false).unwrap()).count(), 1);

    let (_, event) = h.send_captured("settings_176_full");
    assert_eq!(event["sent_infinite_ammo"], true);
    // the same value again is recorded but sends nothing
    let (sent, event) = h.send_captured("capture_0");
    assert_eq!(event["received_range_settings"].as_array().map(|a| a.len()), Some(2));
    assert!(event.get("sent_infinite_ammo").is_none());
    assert!(sent.iter().all(|(channel, _, _)| *channel != 4));
}

/// the player position track: reset at the placement correction, then
/// smoothed 27 ticks (0.2109375 s) behind the move-ack reads.
#[test]
fn player_track_follows_the_saved_moves() {
    let mut h = Harness::new(50613, Some(0));
    h.run_until(|s, source| s.connections[&source].moves.correction_sent);
    for _ in 0..8 {
        h.now += 0.125;
        h.client.tick(&mut h.server, h.now, h.source, &mut h.out);
    }
    // python logs the correction rpc's own 932 bits (production log)
    h.server.flush();
    let log = std::fs::read_to_string(&h.events).unwrap();
    assert!(log.contains("\"move_correction_rpc_bits\":932"));
    let conn = &h.server.connections[&h.source];
    assert_eq!(conn.player.delay, 0.2109375);
    let value = conn.player.value_now(h.now).expect("samples");
    assert!(!value.stale);
    assert!(matches!(value.value.kind, aries_interp::var::Kind::Hermite | aries_interp::var::Kind::Linear), "{:?}", value.value.kind);
    // between the reads that bracket now - delay
    let newest = conn.player.var.newest().unwrap().v.x;
    assert!(value.value.v.x < newest && value.value.v.x > newest - 2.0 * 120.0, "{} vs {newest}", value.value.v.x);
}
/// the debug command queue, python semantics: case-insensitive glob and sort,
/// parse before unlink, int()/float()/bool() coercion, fromhex whitespace.
#[test]
fn command_queue_matches_python() {
    let queue = std::env::temp_dir().join(format!("aries_queue_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&queue);
    std::fs::create_dir_all(&queue).unwrap();
    let mut h = Harness::new(50614, Some(0));
    h.server.config.command_queue = Some(queue.clone());
    h.run_until(|s, source| s.connections[&source].bringup.sent(Stage::ClientRestart));
    // ch6 is open: a 16-bit body, channel and bits as strings, hex with spaces
    std::fs::write(queue.join("a.json"), r#"{"channel": "6", "body_hex": "ab cd", "bits": "16", "exports": 0}"#).unwrap();
    std::fs::write(queue.join("B.JSON"), r#"{"spawn_bot": ["5000", 1300, -14000.5]}"#).unwrap();
    std::fs::write(queue.join("c.json"), "{not json").unwrap();
    std::fs::write(queue.join("d.json"), r#"{"channel": 6, "body_hex": "abc", "bits": 12}"#).unwrap();
    h.now += 0.3;
    let (sent, event) = h.send_captured("trailer_overrun_1");
    assert_eq!(event["sent_queued_commands"], serde_json::json!([["a.json", "6", "16"]]));
    let errors = event["queued_command_errors"].as_array().unwrap();
    assert_eq!(errors.iter().map(|e| e[0].as_str().unwrap()).collect::<Vec<_>>(), ["c.json", "d.json"]);
    assert_eq!(event["bot_spawned"][0][2], serde_json::json!([5000, 1300, -14000]));
    assert!(sent.iter().any(|(channel, reliable, body)| *channel == 6 && *reliable && body.bytes == [0xAB, 0xCD]));
    // a file that does not parse stays; the others are gone
    let left: Vec<_> = std::fs::read_dir(&queue).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(left, ["c.json"]);
    let _ = std::fs::remove_dir_all(&queue);
}
