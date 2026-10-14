//! content transform and actor headers against server/fixtures/content.json.

use aries_bits::Bits;
use aries_rep::block::dynamic_actor_reference;
use aries_rep::{Spawn, encode_short_actor_rep_payload};
use serde_json::Value;

fn fixture() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/content.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("content.json")).expect("json")
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
}

#[test]
fn content_transform_matches_python() {
    let f = fixture();
    let cases = f["transform"].as_array().unwrap();
    assert!(cases.len() > 250);
    for case in cases {
        let bits = case["bits"].as_u64().unwrap() as usize;
        let guid = case["guid"].as_u64().unwrap() as u32;
        let plain = Bits::new(unhex(case["plain"].as_str().unwrap()), bits).unwrap();
        let encoded = encode_short_actor_rep_payload(&plain, 0, guid).unwrap();
        assert_eq!(encoded, unhex(case["encoded"].as_str().unwrap()), "{bits} bits guid {guid}");
    }
    for case in f["offset"].as_array().unwrap() {
        let plain = Bits::new(unhex(case["plain"].as_str().unwrap()), 9).unwrap();
        let encoded = encode_short_actor_rep_payload(&plain, case["offset"].as_u64().unwrap() as u32, case["guid"].as_u64().unwrap() as u32).unwrap();
        assert_eq!(encoded, unhex(case["encoded"].as_str().unwrap()));
    }
}

#[test]
fn actor_references_match_python() {
    for case in fixture()["actor_reference"].as_array().unwrap() {
        let archetype = case["archetype"].as_array().unwrap();
        let spawn = case["location"].as_array().map(|l| Spawn {
            location: [l[0].as_f64().unwrap(), l[1].as_f64().unwrap(), l[2].as_f64().unwrap()],
            yaw: case["yaw"].as_f64().unwrap_or(0.0),
        });
        let bits = dynamic_actor_reference(
            (archetype[0].as_str().unwrap(), archetype[1].as_str().unwrap()),
            case["actor"].as_u64().unwrap() as u32,
            case["archetype_guid"].as_u64().unwrap() as u32,
            case["package_guid"].as_u64().unwrap() as u32,
            case["level"].as_u64().unwrap() as u32,
            spawn,
        )
        .unwrap();
        assert_eq!(bits.bit_count as u64, case["bit_count"].as_u64().unwrap(), "{}", case["name"]);
        assert_eq!(bits.hex(), case["hex"].as_str().unwrap(), "{}", case["name"]);
    }
}
