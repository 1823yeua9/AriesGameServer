//! per-datagram event records, with the python responder's key names so the
//! two logs can be diffed line by line. events go to a jsonl file; the
//! console gets a short line for anything that sent something or failed.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use serde_json::{Map, Value};

#[derive(Default)]
pub struct Event {
    pub fields: Map<String, Value>,
}

impl Event {
    pub fn set(&mut self, key: &str, value: impl Into<Value>) {
        self.fields.insert(key.to_string(), value.into());
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.fields.get(key)
    }

    /// appends to a list field (python event.setdefault(key, []).append(...)).
    pub fn push(&mut self, key: &str, value: impl Into<Value>) {
        let entry = self.fields.entry(key.to_string()).or_insert_with(|| Value::Array(Vec::new()));
        if let Value::Array(items) = entry {
            items.push(value.into());
        }
    }

    /// the console level for this event (spec 05 section 8): errors warn,
    /// authority work is info, the high-rate keys are debug, routine is silent.
    fn level(&self) -> Option<log::Level> {
        let keys = || self.fields.keys().map(String::as_str);
        if keys().any(|k| k.ends_with("_error")) {
            return Some(log::Level::Warn);
        }
        if keys().any(|k| !ROUTINE_KEYS.contains(&k) && !HIGH_RATE_KEYS.contains(&k)) {
            return Some(log::Level::Info);
        }
        if keys().any(|k| HIGH_RATE_KEYS.contains(&k)) {
            return Some(log::Level::Debug);
        }
        None
    }
}

pub struct EventLog {
    file: Option<BufWriter<File>>,
    pub total: u64,
}

/// python's quiet-mode routine keys (range_udp_handshake.py:1378-1393): every
/// packet has them, so they never make a console line on their own.
const ROUTINE_KEYS: [&str; 41] = [
    "elapsed_seconds", "source_port", "length", "ares_inspection", "ciphertext_length", "counter", "stateless_header", "payload_bits",
    "control_prefix_hex", "bunch_start_bit", "bunch_header_end_bit", "bunch_is_control", "bunch_is_open", "bunch_replication_paused",
    "bunch_reliable", "channel_index", "has_exports", "has_must_map", "bunch_partial", "header_flag_4", "channel_sequence",
    "channel_sequence_start_bit", "hardcoded_name", "channel_name_index", "bunch_bit_count", "bunch_body_start_bit", "bunch_end_bit",
    "control_message_type", "sent_session_ack_length", "acked_client_sequence", "server_sequence", "sent_move_ack_length",
    "move_ack_value_source", "move_ack_value", "move_ack_newest_number", "move_ack_timestamp", "move_ack_oldest", "move_ack_last_acked",
    "move_ack_count", "decoded_packet_hex", "client_packet_sequence",
];

/// authority keys that fire many times a second: debug only.
const HIGH_RATE_KEYS: [&str; 6] = [
    "sent_equippable_auth_states", "input_ack_received_event_id", "input_ack_received_frame_length", "input_ack_rejected_event_id",
    "received_activation_events", "movement_ack_path",
];

impl EventLog {
    pub fn new(path: Option<&Path>) -> Self {
        let file = path.and_then(|path| {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            File::create(path).ok().map(BufWriter::new)
        });
        Self { file, total: 0 }
    }

    pub fn record(&mut self, event: Event) {
        self.total += 1;
        if let Some(level) = event.level().filter(|level| log::log_enabled!(target: "range", *level)) {
            let summary: Vec<String> = event.fields.iter().filter(|(k, _)| !ROUTINE_KEYS.contains(&k.as_str())).map(|(k, v)| format!("{k}={v}")).collect();
            log::log!(target: "range", level, "{}", summary.join(" "));
        }
        if let Some(file) = self.file.as_mut() {
            let _ = serde_json::to_writer(&mut *file, &Value::Object(event.fields));
            let _ = file.write_all(b"\n");
        }
    }

    pub fn flush(&mut self) {
        if let Some(file) = self.file.as_mut() {
            let _ = file.flush();
        }
    }
}
