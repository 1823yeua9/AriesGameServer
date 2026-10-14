//! stateless handshake against server/fixtures/handshake.json.

use aries_net::handshake::{RAW_ENVELOPE_BYTES, build_ack, build_challenge, parse_handshake, wrap_raw};

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
}

#[test]
fn handshake_matches_python() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/handshake.json");
    let cases: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    for case in cases.as_array().unwrap() {
        let initial = parse_handshake(&unhex(case["initial"].as_str().unwrap())).unwrap();
        let p = &case["parsed"];
        assert_eq!(initial.travel_count as u64, p["travel_count"].as_u64().unwrap());
        assert_eq!(initial.client_id as u64, p["client_id"].as_u64().unwrap());
        assert_eq!(initial.network_version as u64, p["network_version"].as_u64().unwrap());
        let cookie: [u8; 20] = unhex(case["cookie"].as_str().unwrap()).try_into().unwrap();
        let padding: [u8; 8] = unhex(case["padding"].as_str().unwrap()).try_into().unwrap();
        let challenge = build_challenge(&initial, case["timestamp"].as_f64().unwrap(), &cookie, &padding).unwrap();
        assert_eq!(challenge.bit_count, 377);
        assert_eq!(wrap_raw(&challenge, RAW_ENVELOPE_BYTES), unhex(case["challenge"].as_str().unwrap()));
        let response = parse_handshake(&unhex(case["response"].as_str().unwrap())).unwrap();
        assert_eq!(response.cookie, cookie);
        assert_eq!(response.timestamp, case["timestamp"].as_f64().unwrap());
        let ack = build_ack(&response, &padding).unwrap();
        assert_eq!(wrap_raw(&ack, RAW_ENVELOPE_BYTES), unhex(case["ack"].as_str().unwrap()));
    }
}
