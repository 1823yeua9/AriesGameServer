//! fixture helpers shared by the spec 08 golden tests (server/fixtures/world_*.json,
//! written by server/tools/golden/world_abilities.py).
#![allow(dead_code)]

use aries_bits::Bits;
use serde_json::Value;

pub fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/{name}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).expect("fixture json")
}

/// the python exception name when the case is a guard, else none.
pub fn python_error(case: &Value) -> Option<&str> {
    case.get("error").and_then(Value::as_str)
}

/// a builder result against {"hex", "bit_count"} or {"error": name}.
pub fn check<E: std::fmt::Debug>(result: Result<Bits, E>, case: &Value, label: &str) {
    match python_error(case) {
        Some(error) => assert!(result.is_err(), "{label}: python raised {error}, rust built {result:?}"),
        None => {
            let bits = result.unwrap_or_else(|error| panic!("{label}: python built it, rust failed: {error:?}"));
            assert_eq!(bits.bit_count as u64, case["bit_count"].as_u64().expect("bit_count"), "{label} bit count");
            assert_eq!(bits.hex(), case["hex"].as_str().expect("hex"), "{label}");
        }
    }
}

pub fn u32_of(value: &Value) -> u32 {
    u32::try_from(value.as_u64().unwrap_or_else(|| panic!("not a u32: {value}"))).expect("u32 range")
}

pub fn f64_of(value: &Value) -> f64 {
    value.as_f64().unwrap_or_else(|| panic!("not a number: {value}"))
}

pub fn vec3(value: &Value) -> [f64; 3] {
    let items = value.as_array().unwrap_or_else(|| panic!("not a vector: {value}"));
    assert_eq!(items.len(), 3);
    [f64_of(&items[0]), f64_of(&items[1]), f64_of(&items[2])]
}

/// exact float equality (bit patterns, so -0.0 and nan count).
pub fn same(actual: f64, expected: f64, label: &str) {
    assert_eq!(actual.to_bits(), expected.to_bits(), "{label}: rust {actual:?} python {expected:?}");
}

pub fn same3(actual: [f64; 3], expected: [f64; 3], label: &str) {
    for axis in 0..3 {
        same(actual[axis], expected[axis], &format!("{label}[{axis}]"));
    }
}

/// a compact row's fields ("-" is python None).
pub fn fields(row: &Value) -> Vec<&str> {
    row.as_str().expect("row string").split(' ').collect()
}

pub fn float_field(text: &str) -> f64 {
    text.parse().unwrap_or_else(|_| panic!("bad float {text}"))
}

/// 16 hex digits of an f64 bit pattern.
pub fn bits_field(text: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(text, 16).unwrap_or_else(|_| panic!("bad bits {text}")))
}

pub fn optional(text: &str) -> Option<&str> {
    (text != "-").then_some(text)
}

/// per-channel reliable sequences as ruh's take_channel_sequence hands them out:
/// B + the channel's first dynamic offset (ruh:786, or 0 for a channel the
/// system opens itself) + its sends so far.
pub struct Sequences {
    base: u32,
    first: std::collections::HashMap<u32, u32>,
    next: std::collections::HashMap<u32, u32>,
}

impl Sequences {
    pub fn new(section: &Value) -> Self {
        let first_offsets = &section["first_offsets"];
        let first = first_offsets["offsets"].as_object().unwrap().iter().map(|(channel, offset)| (channel.parse().unwrap(), u32_of(offset))).collect();
        Self { base: u32_of(&first_offsets["base"]), first, next: Default::default() }
    }

    fn take(&mut self, channel: u32, is_open: bool) -> u32 {
        let offset = match self.next.get(&channel) {
            Some(offset) => *offset,
            // a dynamic channel's open resets its first offset to 0 (python sets the dict entry first)
            None if is_open => 0,
            None => *self.first.get(&channel).unwrap_or_else(|| panic!("channel {channel} sent before any open")),
        };
        self.next.insert(channel, offset + 1);
        (self.base + offset) & 0xFFF
    }
}

/// the sends a system returned against python's recorded send_reliable calls.
pub fn check_sends(actual: &[aries_world::bots::ChannelSend], expected: &Value, sequences: &mut Sequences, label: &str) {
    let expected = expected.as_array().expect("sends");
    assert_eq!(actual.len(), expected.len(), "{label}: rust sent {actual:?}");
    for (k, (send, want)) in actual.iter().zip(expected).enumerate() {
        let label = format!("{label} send {k}");
        assert_eq!(send.channel, u32_of(&want["channel"]), "{label} channel");
        assert_eq!(send.has_exports, want["has_exports"].as_bool().unwrap(), "{label} has_exports");
        assert_eq!(send.is_open, want["is_open"].as_bool().unwrap(), "{label} is_open");
        assert_eq!(send.is_close, want["is_close"].as_bool().unwrap(), "{label} is_close");
        assert_eq!(sequences.take(send.channel, send.is_open), u32_of(&want["sequence"]), "{label} sequence");
        assert_eq!(send.body.bit_count as u64, want["bit_count"].as_u64().unwrap(), "{label} bit count");
        assert_eq!(send.body.hex(), want["hex"].as_str().unwrap(), "{label} body");
    }
}
