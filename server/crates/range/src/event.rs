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

    /// true when anything was sent or went wrong (worth a console line).
    fn notable(&self) -> bool {
        self.fields.keys().any(|k| k.starts_with("sent_") || k.ends_with("_error") || k.starts_with("purchase") || k.starts_with("bot_") || k.contains("cloudburst"))
    }
}

pub struct EventLog {
    file: Option<BufWriter<File>>,
    pub total: u64,
}

/// keys that are bookkeeping on every packet; left out of the console line.
const QUIET_KEYS: [&str; 10] =
    ["elapsed_seconds", "source_port", "length", "ares_inspection", "ciphertext_length", "counter", "stateless_header", "payload_bits", "control_prefix_hex", "acked_client_sequence"];

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
        if event.notable() {
            let summary: Vec<String> = event
                .fields
                .iter()
                .filter(|(k, _)| !QUIET_KEYS.contains(&k.as_str()) && !k.starts_with("bunch_") && !k.ends_with("_payload"))
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            log::info!(target: "range", "{}", summary.join(" "));
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
