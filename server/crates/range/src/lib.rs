//! the range game mode: per-datagram handling in the python responder's exact
//! order (stateless handshake, nmt replies, session-ack tick with one bring-up
//! stage and the move ack, plus the per-packet authority work).

pub mod bringup;
pub mod config;
pub mod conn;
pub mod event;
pub mod live;
pub mod moves;
pub mod send;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;

use aries_bits::Bits;
use aries_net::control::{control_reply, inspect_first_bunch, read_packet_header, session_ack_packet};
use aries_net::handshake::{self, PACKET_INITIAL, PACKET_RESPONSE, RAW_ENVELOPE_BYTES};
use aries_net::{AresGcm, decode_client_packet, seal_packet};

pub use config::RangeConfig;
pub use conn::Connection;
pub use event::{Event, EventLog};
pub use live::{LiveClient, NoLiveClient, SavedMoves};
pub use send::{Outbox, Reply, SendError};

/// both session keys, read from the live client's ares packet handler.
#[derive(Clone)]
pub struct SessionKeys {
    /// decrypts client -> server (handler +0x28)
    pub client_send: [u8; 32],
    /// encrypts server -> client (handler +0x48, python receive_key_override)
    pub server_send: [u8; 32],
}

/// where keys come from (the oracle's handler scan in production).
pub trait KeySource {
    fn read_keys(&mut self) -> Result<SessionKeys, String>;
}

struct Keys {
    client: AresGcm,
    server: AresGcm,
}

pub struct RangeServer {
    pub config: RangeConfig,
    keys: Option<Keys>,
    key_source: Box<dyn KeySource>,
    pub connections: HashMap<SocketAddr, Connection>,
    pub events: EventLog,
    /// skins, ability tuning and live rpc indices for bring-up
    pub hooks: Box<dyn bringup::StageHooks>,
    /// reads of the running client (moves, input bounds, game state)
    pub live: Box<dyn LiveClient>,
    start: f64,
}

/// the welcome map url (python welcome_map_url default).
pub const WELCOME_MAP_URL: &str = "/Game/Maps/Poveglia/Range";

impl RangeServer {
    pub fn new(config: RangeConfig, key_source: Box<dyn KeySource>, event_path: Option<&Path>) -> Self {
        Self { config, keys: None, key_source, connections: HashMap::new(), events: EventLog::new(event_path), hooks: Box::new(bringup::DefaultHooks), live: Box::new(NoLiveClient), start: 0.0 }
    }

    /// handles one datagram at monotonic time `now` (seconds).
    pub fn handle_datagram(&mut self, now: f64, source: SocketAddr, datagram: &[u8], out: &mut Outbox) {
        if !source.ip().is_loopback() {
            return;
        }
        if self.start == 0.0 {
            self.start = now;
        }
        let mut event = Event::default();
        event.set("elapsed_seconds", ((now - self.start) * 1000.0).round() / 1000.0);
        event.set("source_port", source.port());
        event.set("length", datagram.len());

        if self.keys.is_none() {
            match self.key_source.read_keys() {
                Ok(keys) => {
                    log::info!(target: "range", "session keys read from the live client");
                    self.keys = Some(Keys { client: AresGcm::new(&keys.client_send), server: AresGcm::new(&keys.server_send) });
                }
                Err(error) => {
                    event.set("handler_state", "not_ready");
                    event.set("handler_error", error);
                    self.events.record(event);
                    return;
                }
            }
        }

        match handshake::parse_handshake(datagram) {
            Ok(packet) => self.handle_handshake(now, source, &packet, &mut event, out),
            Err(error) => {
                if !self.handle_session_packet(now, source, datagram, &error.to_string(), &mut event, out) {
                    return;
                }
            }
        }
        self.events.record(event);
    }

    fn handle_handshake(&mut self, now: f64, source: SocketAddr, packet: &handshake::HandshakePacket, event: &mut Event, out: &mut Outbox) {
        event.set("packet_type", packet.packet_type);
        event.set("packet_count", packet.packet_count);
        event.set("network_version", packet.network_version);
        event.set("client_id", packet.client_id);
        let conn = self.connections.entry(source).or_insert_with(|| Connection::new(source));
        if packet.packet_type == PACKET_INITIAL {
            // python: timestamp = time.monotonic() (positive), cookie = os.urandom(20)
            let timestamp = now + 1.0;
            let cookie: [u8; 20] = random_bytes();
            conn.pending = Some((timestamp, cookie));
            if let Some(challenge) = handshake::build_challenge(packet, timestamp, &cookie, &random_bytes()) {
                let datagram = handshake::wrap_raw(&challenge, RAW_ENVELOPE_BYTES);
                event.set("sent_challenge_length", datagram.len());
                event.set("challenge_mode", "raw");
                out.datagrams.push((source, datagram));
            }
        } else if packet.packet_type == PACKET_RESPONSE {
            let matches = conn.pending.is_some_and(|(timestamp, cookie)| packet.timestamp == timestamp && packet.cookie == cookie);
            event.set("matches_challenge", matches);
            if matches {
                if let Some(ack) = handshake::build_ack(packet, &random_bytes()) {
                    let datagram = handshake::wrap_raw(&ack, RAW_ENVELOPE_BYTES);
                    event.set("sent_ack_length", datagram.len());
                    out.datagrams.push((source, datagram));
                }
            }
        }
    }

    /// returns false when the event must not be recorded (python `continue`).
    fn handle_session_packet(&mut self, now: f64, source: SocketAddr, datagram: &[u8], parse_error: &str, event: &mut Event, out: &mut Outbox) -> bool {
        let keys = self.keys.as_ref().expect("keys checked above");
        let Some(packet) = decode_client_packet(datagram, &keys.client) else {
            event.set("ares_inspection", "invalid_or_unverified");
            event.set("parse_error", parse_error);
            return true;
        };
        let decoded = &packet.decoded;
        let header = read_packet_header(decoded).expect("authenticated packets carry the header");
        event.set("ares_inspection", "authenticated");
        event.set("counter", packet.counter);
        event.set("client_packet_sequence", header.sequence);
        event.set("stateless_header", packet.stateless_header);
        event.set("payload_bits", decoded.bit_count);
        let first_bunch = inspect_first_bunch(decoded);
        if let Some(info) = &first_bunch {
            event.set("channel_index", info.channel_index);
            if let Some(kind) = info.control_message_type {
                event.set("control_message_type", kind);
            }
        }

        let Some(conn) = self.connections.get_mut(&source) else {
            // a client of another responder instance: decoded, logged, never answered
            return true;
        };
        let ack_mask = conn.record_client_sequence(header.sequence);
        let mut reply = Reply { conn, gcm: &keys.server, out, event, now, client_sequence: header.sequence, ack_mask, stateless_header: packet.stateless_header };

        if self.config.features.retransmit && reply.conn.challenge_sent {
            reply.resend_lost(header.acked, header.history);
        }
        // per-packet work that runs before the dispatch chain (input ack, authority)
        // lands here as those systems come online.

        let message_type = first_bunch.as_ref().and_then(|b| b.control_message_type);
        let pending = reply.conn.pending.is_some();
        if decoded.bit_count == 232 && pending {
            reply.conn.hello = Some(decoded.clone());
        } else if !reply.conn.challenge_sent && pending && decoded.bit_count == 77 {
            let Some(hello) = reply.conn.hello.clone() else {
                return false; // python `continue`: no hello yet, nothing recorded
            };
            match control_reply(&hello, decoded, 3, &[""], None) {
                Ok((reply_packet, client_sequence, server_sequence)) => {
                    let datagram = seal(&reply_packet, packet.stateless_header, &keys.server);
                    reply.event.set("sent_control_message_type", 3);
                    reply.event.set("sent_ack_probe_length", datagram.len());
                    reply.event.set("acked_client_sequence", client_sequence);
                    reply.event.set("server_sequence", server_sequence);
                    reply.out.datagrams.push((source, datagram));
                    reply.conn.next_server_sequence = (server_sequence + 1) & 0x3FFF;
                    reply.conn.base_sequence = (server_sequence + 1) & 0xFFF;
                    reply.conn.challenge_sent = true;
                }
                Err(error) => reply.event.set("challenge_error", error.to_string()),
            }
        } else if reply.conn.challenge_sent && !reply.conn.welcome_sent && pending && message_type == Some(5) {
            let hello = reply.conn.hello.clone().expect("challenge needed the hello");
            match control_reply(&hello, decoded, 1, &[WELCOME_MAP_URL, "", ""], Some(reply.conn.next_server_sequence)) {
                Ok((reply_packet, client_sequence, server_sequence)) => {
                    let datagram = seal(&reply_packet, packet.stateless_header, &keys.server);
                    reply.event.set("sent_control_message_type", 1);
                    reply.event.set("sent_welcome_probe_length", datagram.len());
                    reply.event.set("acked_client_sequence", client_sequence);
                    reply.event.set("server_sequence", server_sequence);
                    reply.out.datagrams.push((source, datagram));
                    reply.conn.next_server_sequence = (reply.conn.next_server_sequence + 1) & 0x3FFF;
                    reply.conn.welcome_sent = true;
                }
                Err(error) => reply.event.set("welcome_error", error.to_string()),
            }
        } else if reply.conn.welcome_sent && !reply.conn.join_ack_sent && pending && message_type == Some(9) {
            let join = first_bunch.as_ref().expect("message type came from the first bunch");
            if join.channel_index != 0 || !join.reliable {
                reply.event.set("join_ack_error", "join ack requires an authenticated nmt_join");
            } else {
                // the join ack is a session ack with the fixed mask 3
                let ack = session_ack_packet(reply.conn.next_server_sequence, header.sequence, 3);
                let datagram = seal(&ack, packet.stateless_header, &keys.server);
                reply.event.set("sent_join_ack_length", datagram.len());
                reply.event.set("acked_client_sequence", header.sequence);
                reply.event.set("server_sequence", reply.conn.next_server_sequence);
                reply.out.datagrams.push((source, datagram));
                reply.conn.next_server_sequence = (reply.conn.next_server_sequence + 1) & 0x3FFF;
                reply.conn.join_ack_sent = true;
                reply.conn.last_session_ack = Some(now);
            }
        } else if reply.conn.session_tick_due(now) {
            let server_sequence = reply.conn.next_server_sequence;
            match reply.send_session_ack(ack_mask) {
                Ok(length) => {
                    reply.event.set("sent_session_ack_length", length);
                    reply.event.set("acked_client_sequence", header.sequence);
                    reply.event.set("server_sequence", server_sequence);
                    reply.conn.last_session_ack = Some(now);
                    bringup::run_stage_chain(&mut reply, &self.config, self.hooks.as_mut());
                    moves::run_move_ack(&mut reply, &self.config, self.live.as_mut());
                }
                Err(error) => reply.event.set("session_ack_error", error.to_string()),
            }
        }
        true
    }

    pub fn flush(&mut self) {
        self.events.flush();
    }
}

fn seal(packet: &Bits, stateless_header: u32, gcm: &AresGcm) -> Vec<u8> {
    seal_packet(packet, stateless_header, gcm).expect("control replies have a valid stateless header")
}

/// os random bytes (python os.urandom).
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("os rng");
    bytes
}
