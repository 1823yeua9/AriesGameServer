//! replays server/fixtures/net.json: packet transform, gcm wrap, full server
//! frames and client packet decode, all against the python responder.

use aries_bits::Bits;
use aries_net::frame::wrap_gcm;
use aries_net::keys::derive_receive_key;
use aries_net::transform::{inverse_short_packet, transform_short_packet};
use aries_net::{AresGcm, BunchHeader, PacketInfo, build_server_frame, decode_client_packet};
use serde_json::Value;

fn fixture() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/net.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("net.json")).expect("json")
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
}

fn key32(text: &str) -> [u8; 32] {
    unhex(text).try_into().unwrap()
}

#[test]
fn transform_matches_python() {
    let f = fixture();
    let cases = f["transform"].as_array().unwrap();
    assert!(cases.len() >= 70);
    for case in cases {
        let bits = case["bits"].as_u64().unwrap() as usize;
        let counter = case["counter"].as_u64().unwrap() as u32;
        let plain = unhex(case["plain"].as_str().unwrap());
        let mut forward = plain.clone();
        assert!(transform_short_packet(&mut forward, bits, counter));
        assert_eq!(forward, unhex(case["forward"].as_str().unwrap()), "forward {bits} bits counter {counter}");
        let mut inverse = plain.clone();
        assert!(inverse_short_packet(&mut inverse, bits, counter));
        assert_eq!(inverse, unhex(case["inverse"].as_str().unwrap()), "inverse {bits} bits counter {counter}");
        // and they undo each other
        assert!(inverse_short_packet(&mut forward, bits, counter));
        let mask_last = |v: &mut Vec<u8>| {
            if !bits.is_multiple_of(8) {
                let last = v.len() - 1;
                v[last] &= (1u8 << (bits % 8)) - 1;
            }
        };
        let (mut a, mut b) = (forward, plain);
        mask_last(&mut a);
        mask_last(&mut b);
        assert_eq!(a, b, "round trip {bits}");
    }
}

#[test]
fn gcm_wrap_matches_python() {
    let f = fixture();
    for case in f["gcm"].as_array().unwrap() {
        let gcm = AresGcm::new(&key32(case["key"].as_str().unwrap()));
        let bits = case["bits"].as_u64().unwrap() as usize;
        let plain = Bits::new(unhex(case["plain"].as_str().unwrap()), bits).unwrap();
        assert_eq!(wrap_gcm(&plain, &gcm, 0), unhex(case["wrapped"].as_str().unwrap()), "{bits} bits");
    }
}

#[test]
fn server_frames_match_python() {
    let f = fixture();
    for case in f["frames"].as_array().unwrap() {
        let h = &case["header"];
        let header = BunchHeader {
            channel_index: h["channel_index"].as_u64().unwrap() as u32,
            channel_name_index: h["channel_name_index"].as_u64().unwrap() as u32,
            channel_sequence: h["channel_sequence"].as_u64().unwrap() as u32,
            is_open: h["is_open"].as_bool().unwrap(),
            is_close: h["is_close"].as_bool().unwrap(),
            is_reliable: h["is_reliable"].as_bool().unwrap(),
            has_exports: h["has_exports"].as_bool().unwrap(),
            ..Default::default()
        };
        let i = &case["info"];
        let info = PacketInfo {
            server_sequence: i["server_sequence"].as_u64().unwrap() as u32,
            acked_client_sequence: i["acked_client_sequence"].as_u64().unwrap() as u32,
            ack_history_mask: i["ack_history_mask"].as_u64().unwrap() as u32,
            stateless_header: i["stateless_header"].as_u64().unwrap() as u32,
        };
        let body = Bits::new(unhex(case["body"].as_str().unwrap()), case["bits"].as_u64().unwrap() as usize).unwrap();
        let gcm = AresGcm::new(&key32(case["key"].as_str().unwrap()));
        let (frame, packet_bits) = build_server_frame(&body, &header, &info, &gcm).unwrap();
        assert_eq!(packet_bits as u64, case["packet_bits"].as_u64().unwrap());
        assert_eq!(frame, unhex(case["frame"].as_str().unwrap()), "frame on channel {}", header.channel_index);
    }
}

#[test]
fn client_packets_decode() {
    let f = fixture();
    for case in f["client"].as_array().unwrap() {
        let gcm = AresGcm::new(&key32(case["key"].as_str().unwrap()));
        let packet = decode_client_packet(&unhex(case["datagram"].as_str().unwrap()), &gcm).expect("decodes");
        assert_eq!(packet.decoded.bit_count as u64, case["bits"].as_u64().unwrap());
        assert_eq!(packet.counter as u64, case["counter"].as_u64().unwrap());
        assert_eq!(packet.stateless_header as u64, case["stateless"].as_u64().unwrap());
        assert_eq!(packet.decoded.bytes, unhex(case["decoded"].as_str().unwrap()));
        // a flipped ciphertext byte must fail the tag
        let mut bad = unhex(case["datagram"].as_str().unwrap());
        bad[20] ^= 0x10;
        assert!(decode_client_packet(&bad, &gcm).is_none());
    }
}

#[test]
fn receive_key_derivation_matches_python() {
    let f = fixture();
    for case in f["keys"].as_array().unwrap() {
        let key = derive_receive_key(case["player_key"].as_str().unwrap());
        assert_eq!(key.to_vec(), unhex(case["receive_key"].as_str().unwrap()));
    }
}
