//! shared rep helpers against server/fixtures/rep_shared.json.

use aries_bits::Bits;
use aries_rep::{bodies, props};
use serde_json::Value;

fn check(bits: Bits, case: &Value, label: &str) {
    assert_eq!(bits.bit_count as u64, case["bit_count"].as_u64().unwrap(), "{label} bits");
    assert_eq!(bits.hex(), case["hex"].as_str().unwrap(), "{label}");
}

fn u(v: &Value) -> u32 {
    v.as_u64().unwrap() as u32
}

#[test]
fn shared_helpers_match_python() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/rep_shared.json");
    let f: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    for c in f["bool"].as_array().unwrap() {
        check(props::bool_property(u(&c["handle"]), c["value"].as_bool().unwrap()).unwrap(), c, "bool");
    }
    for c in f["object"].as_array().unwrap() {
        check(props::object_property(u(&c["handle"]), u(&c["guid"])).unwrap(), c, "object");
    }
    for c in f["role"].as_array().unwrap() {
        check(props::autonomous_role(u(&c["width"]), u(&c["value"])).unwrap(), c, "role");
    }
    for c in f["owner"].as_array().unwrap() {
        check(bodies::channel_actor_owner_block(u(&c["actor"]), u(&c["owner"])).unwrap(), c, "owner");
    }
    for c in f["contents"].as_array().unwrap() {
        check(bodies::item_slot_contents_block(u(&c["slot"]), u(&c["item"]), bodies::PAWN_GUID).unwrap(), c, "contents");
    }
    for c in f["correction"].as_array().unwrap() {
        let guid = c["guid"].as_u64().map(|g| g as u32);
        check(bodies::inventory_correction_body(u(&c["index"]), guid).unwrap(), c, "correction");
    }
    assert_eq!(props::empty_rep().hex(), "0000");
}
