//! session acks, nmt replies, first-bunch inspection and ack history vs python.

use aries_bits::Bits;
use aries_net::control::{control_reply, inspect_first_bunch, received_history_mask, session_ack_packet};
use aries_net::{AresGcm, seal_packet};
use serde_json::Value;

fn fixture() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/control.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
}

fn key() -> AresGcm {
    AresGcm::new(&std::array::from_fn(|i| i as u8))
}

fn u(v: &Value) -> u32 {
    v.as_u64().unwrap() as u32
}

#[test]
fn session_acks_match_python() {
    for case in fixture()["session_ack"].as_array().unwrap() {
        let packet = session_ack_packet(u(&case["server"]), u(&case["client"]), u(&case["mask"]));
        assert_eq!(packet.bit_count, 77);
        let frame = seal_packet(&packet, u(&case["stateless"]), &key()).unwrap();
        assert_eq!(frame, unhex(case["frame"].as_str().unwrap()));
    }
}

#[test]
fn control_replies_match_python() {
    for case in fixture()["replies"].as_array().unwrap() {
        let hello = Bits::new(unhex(case["hello"].as_str().unwrap()), 232).unwrap();
        let short = Bits::new(unhex(case["short"].as_str().unwrap()), case["short_bits"].as_u64().unwrap() as usize).unwrap();
        let sh = u(&case["stateless"]);
        let (packet, client_sequence, server_sequence) = control_reply(&hello, &short, 3, &[""], None).unwrap();
        let expected = &case["challenge"];
        assert_eq!(seal_packet(&packet, sh, &key()).unwrap(), unhex(expected[0].as_str().unwrap()), "challenge");
        assert_eq!((client_sequence, server_sequence), (u(&expected[1]), u(&expected[2])));

        let login = Bits::new(unhex(case["login"].as_str().unwrap()), case["login_bits"].as_u64().unwrap() as usize).unwrap();
        let (packet, client_sequence, server_sequence) =
            control_reply(&hello, &login, 1, &["/Game/Maps/Poveglia/Range", "", ""], Some(u(&case["server_sequence"]))).unwrap();
        let expected = &case["welcome"];
        assert_eq!(seal_packet(&packet, sh, &key()).unwrap(), unhex(expected[0].as_str().unwrap()), "welcome");
        assert_eq!((client_sequence, server_sequence), (u(&expected[1]), u(&expected[2])));
    }
}

#[test]
fn first_bunch_inspection_matches_python() {
    for case in fixture()["inspect"].as_array().unwrap() {
        let bits = Bits::new(unhex(case["packet"].as_str().unwrap()), case["bits"].as_u64().unwrap() as usize).unwrap();
        let info = inspect_first_bunch(&bits).unwrap();
        let e = &case["info"];
        assert_eq!(info.bunch_start_bit as u64, e["bunch_start_bit"].as_u64().unwrap());
        assert_eq!(info.reliable, e["bunch_reliable"].as_bool().unwrap());
        assert_eq!(info.channel_index as u64, e["channel_index"].as_u64().unwrap());
        assert_eq!(info.channel_sequence.map(|s| s as u64), e["channel_sequence"].as_u64());
        assert_eq!(info.channel_name_index.map(|s| s as u64), e["channel_name_index"].as_u64());
        assert_eq!(info.bunch_bit_count as u64, e["bunch_bit_count"].as_u64().unwrap());
        assert_eq!(info.body_start_bit as u64, e["bunch_body_start_bit"].as_u64().unwrap());
        assert_eq!(info.end_bit as u64, e["bunch_end_bit"].as_u64().unwrap());
        assert_eq!(info.control_message_type.map(|t| t as u64), e["control_message_type"].as_u64());
    }
}

#[test]
fn ack_history_matches_python() {
    for case in fixture()["history"].as_array().unwrap() {
        let recent: Vec<u32> = case["recent"].as_array().unwrap().iter().map(u).collect();
        assert_eq!(received_history_mask(recent, u(&case["current"])), u(&case["mask"]));
    }
}

#[test]
fn client_and_server_sealing_round_trip() {
    use aries_net::client::{open_server_datagram, seal_client_packet};
    use aries_net::decode_client_packet;
    let gcm = key();
    for bits in [77usize, 128, 232, 1089, 1194, 1337, 4000] {
        let bytes: Vec<u8> = (0..bits.div_ceil(8)).map(|i| (i * 37 + bits) as u8).collect();
        let mut bytes = bytes;
        if bits % 8 != 0 {
            let last = bytes.len() - 1;
            bytes[last] &= (1u8 << (bits % 8)) - 1;
        }
        let plain = Bits::new(bytes, bits).unwrap();
        let datagram = seal_client_packet(&plain, 0x1234_5678 ^ bits as u32, 12, &gcm);
        let back = decode_client_packet(&datagram, &gcm).expect("client packet decodes");
        assert_eq!(back.decoded, plain, "{bits}");
        assert_eq!(back.stateless_header, 12);
        let datagram = seal_packet(&plain, 9, &gcm).unwrap();
        let (opened, stateless) = open_server_datagram(&datagram, &gcm).expect("server datagram opens");
        assert_eq!((opened, stateless), (plain, 9));
    }
}
