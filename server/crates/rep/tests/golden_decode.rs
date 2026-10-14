//! client decode (spec 06) against server/fixtures/decode_*.json, exported
//! from the python decoders by tools/golden/decode.py. every expected value
//! there is python's own result (or its exception text) for the same input.
//! set ARIES_DECODE_PACKETS to another decode_packets.json (for example the
//! all-captures sweep) to run the packet test over it.

use std::collections::HashSet;

use aries_bits::{BitWriter, Bits};
use aries_net::{AresGcm, decode_client_packet};
use aries_rep::decode::*;
use aries_rep::encode_block_payload;
use serde_json::{Map, Value, json};

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{}", path.display()))).expect("json")
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
}

fn bits(hex: &Value, bit_count: &Value) -> Bits {
    // raw struct: python passes (bytes, bit_count) pairs that need not agree in length
    Bits { bytes: unhex(hex.as_str().unwrap()), bit_count: n(bit_count) }
}

fn n(value: &Value) -> usize {
    value.as_u64().unwrap() as usize
}

fn max_id(value: &Value) -> Option<u32> {
    value.as_u64().map(|v| v as u32)
}

fn label(case: &Value) -> String {
    case.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| case.to_string().chars().take(160).collect())
}

/// python's exception text, except UnicodeDecodeError (whose text is codec detail) maps to BadString.
fn check_error(error: &DecodeError, expected: &Value, what: &str) {
    if expected["error_type"] == "UnicodeDecodeError" {
        assert_eq!(*error, DecodeError::BadString, "{what}");
    } else {
        assert_eq!(error.to_string(), expected["error"].as_str().unwrap(), "{what}");
    }
}

/// a result against python's value-or-error: `expected` is either the value or {"error", "error_type"}.
fn expect<T>(result: DecodeResult<T>, expected: &Value, to_json: impl FnOnce(T) -> Value, what: &str) {
    let python_error = expected.as_object().filter(|o| o.contains_key("error_type"));
    match (result, python_error) {
        (Ok(value), None) => assert_eq!(to_json(value), *expected, "{what}"),
        (Err(error), Some(_)) => check_error(&error, expected, what),
        (Ok(value), Some(_)) => panic!("{what}: rust decoded {} but python raised {}", to_json(value), expected),
        (Err(error), None) => panic!("{what}: rust raised {error:?} but python returned {expected}"),
    }
}

/// a merged case: python's result keys sit beside the inputs, or "error" does.
fn expect_merged<T>(result: DecodeResult<T>, case: &Value, keys: &[&str], to_json: impl FnOnce(T) -> Value, what: &str) {
    if case.get("error_type").is_some() {
        match result {
            Err(error) => check_error(&error, case, what),
            Ok(value) => panic!("{what}: rust decoded {} but python raised {}", to_json(value), case["error"]),
        }
        return;
    }
    let expected: Map<String, Value> = keys.iter().map(|k| (k.to_string(), case[*k].clone())).collect();
    match result {
        Ok(value) => assert_eq!(to_json(value), Value::Object(expected), "{what}"),
        Err(error) => panic!("{what}: rust raised {error:?} but python returned {expected:?}"),
    }
}

fn bunch_json(bunch: &BunchView) -> Value {
    json!({"start_bit": bunch.start_bit, "is_control": bunch.is_control, "is_reliable": bunch.is_reliable,
           "is_partial": bunch.is_partial, "channel_index": bunch.channel_index,
           "channel_sequence": bunch.channel_sequence, "body_start_bit": bunch.body_start_bit, "end_bit": bunch.end_bit})
}

fn block_json(block: ActorContentBlock) -> Value {
    json!({"bit_offset": block.bit_offset, "bit_count": block.bit_count, "has_rep_layout": block.has_rep_layout,
           "is_actor": block.is_actor, "payload": block.payload.hex()})
}

fn event_json(event: &InputEvent) -> Value {
    let mut out = Map::new();
    out.insert("event_id".into(), json!(event.event_id));
    out.insert("kind".into(), json!(event.kind));
    for (key, value) in [("slot", event.slot), ("flag", event.flag), ("state", event.state), ("action", event.action)] {
        if let Some(value) = value {
            out.insert(key.into(), json!(value));
        }
    }
    if let Some(guid) = event.object_guid {
        out.insert("object_guid".into(), json!(guid));
    }
    Value::Object(out)
}

fn stream_json(stream: &InputStream) -> Value {
    json!({"marker_bit": stream.marker_bit, "end_bit": stream.end_bit, "first_event_id": stream.first_event_id,
           "last_event_id": stream.last_event_id, "events": stream.events.iter().map(event_json).collect::<Vec<_>>()})
}

fn streams_json(streams: Vec<InputStream>) -> Value {
    Value::Array(streams.iter().map(stream_json).collect())
}

fn purchase_json(request: &PurchaseRequest) -> Value {
    json!({"field": request.field, "item_path": request.item_path, "target_slot": request.target_slot,
           "purchasable_type": request.purchasable_type, "channel_sequence": request.channel_sequence})
}

fn settings_json(request: &RangeSettingsRequest) -> Value {
    json!({"field": request.field, "channel_sequence": request.channel_sequence, "bot_armor": request.bot_armor,
           "bot_strafe": request.bot_strafe, "infinite_ammo": request.infinite_ammo, "skill_test": request.skill_test,
           "plant_difficulty": request.plant_difficulty, "defuse_difficulty": request.defuse_difficulty,
           "practice_mode": request.practice_mode, "bot_amount_index": request.bot_amount_index,
           "distance_index": request.distance_index, "bot_enabled": request.bot_enabled,
           "is_init_settings_flow": request.is_init_settings_flow, "object_path": request.object_path})
}

#[test]
fn primitives_match_python() {
    let f = fixture("decode_primitives");
    for case in f["read_bits"].as_array().unwrap() {
        let data = unhex(case["hex"].as_str().unwrap());
        expect_merged(read_bits(&data, n(&case["offset"]), n(&case["width"])), case, &["value"], |v| json!({"value": v}), "read_bits");
    }
    for case in f["packed"].as_array().unwrap() {
        let data = unhex(case["hex"].as_str().unwrap());
        let result = read_packed_uint(&data, n(&case["offset"]), n(&case["limit"]));
        expect_merged(result, case, &["value", "end"], |(v, e)| json!({"value": v, "end": e}), &format!("packed {case}"));
    }
    for case in f["header_packed"].as_array().unwrap() {
        let data = unhex(case["hex"].as_str().unwrap());
        let mut reader = BoundedReader::new(&data, n(&case["offset"]), n(&case["limit"]));
        let result = reader.header_packed().map(|v| (v, reader.position));
        expect_merged(result, case, &["value", "end"], |(v, e)| json!({"value": v, "end": e}), &format!("header packed {case}"));
    }
    for case in f["serialize_int"].as_array().unwrap() {
        let data = unhex(case["hex"].as_str().unwrap());
        let mut reader = BoundedReader::new(&data, n(&case["offset"]), n(&case["limit"]));
        let result = reader.serialize_int(case["maximum"].as_u64().unwrap()).map(|v| (v, reader.position));
        expect_merged(result, case, &["value", "end"], |(v, e)| json!({"value": v, "end": e}), &format!("serialize_int {case}"));
    }
    for case in f["fstring"].as_array().unwrap() {
        let data = unhex(case["hex"].as_str().unwrap());
        let mut reader = BoundedReader::new(&data, n(&case["offset"]), n(&case["limit"]));
        let result = reader.fstring().map(|v| (v, reader.position));
        expect_merged(result, case, &["value", "end"], |(v, e)| json!({"value": v, "end": e}), &format!("fstring {case}"));
    }
    let references = f["object_reference"].as_array().unwrap();
    assert!(references.len() > 60);
    for case in references {
        let data = unhex(case["hex"].as_str().unwrap());
        let result = read_object_reference(&data, n(&case["offset"]), n(&case["limit"]));
        expect_merged(result, case, &["value", "end"], |(v, e)| json!({"value": v, "end": e}), &format!("object reference {case}"));
    }
}

#[test]
fn content_transform_matches_python() {
    let f = fixture("decode_content");
    for group in ["spec", "random"] {
        let cases = f[group].as_array().unwrap();
        assert!(!cases.is_empty());
        for case in cases {
            let wire = bits(&case["wire"], &case["bits"]);
            let plain = decode_content_payload(&wire, case["map"].as_u64().unwrap() as u32).unwrap();
            assert_eq!(plain.hex(), case["plain"].as_str().unwrap(), "{group} {case}");
            assert_eq!(plain.bit_count, wire.bit_count);
        }
    }
    // test_content_transform_roundtrip: python's encoder output decodes back, and ours encodes the same wire
    for case in f["roundtrip"].as_array().unwrap() {
        let wire = bits(&case["wire"], &case["bits"]);
        let decoded = decode_content_payload(&wire, 10).unwrap();
        assert_eq!(decoded.hex(), case["decoded"].as_str().unwrap());
        assert_eq!(decoded.hex(), case["plain"].as_str().unwrap());
        assert_eq!(encode_block_payload(&decoded, 10).unwrap(), wire);
    }
    for case in f["errors"].as_array().unwrap() {
        let wire = bits(&case["wire"], &case["bits"]);
        expect_merged(decode_content_payload(&wire, 4), case, &["plain"], |p| json!({"plain": p.hex()}), &format!("content error {case}"));
    }
}

#[test]
fn actor_blocks_match_python() {
    let cases = fixture("decode_blocks");
    for case in cases.as_array().unwrap() {
        let packet = bits(&case["hex"], &case["bit_count"]);
        let result = read_actor_content_block(&packet, n(&case["start"]), n(&case["end"]), case["map"].as_u64().unwrap() as u32);
        let keys = ["bit_offset", "bit_count", "has_rep_layout", "is_actor", "payload"];
        expect_merged(result, case, &keys, block_json, &label(case));
    }
}

#[test]
fn input_streams_match_python() {
    let f = fixture("decode_inputs");
    let scans = f["scan"].as_array().unwrap();
    for case in scans.iter().filter(|c| c.get("skip_scan").is_none()) {
        let payload = bits(&case["hex"], &case["bit_count"]);
        let result = scan_received_input_streams(&payload, max_id(&case["max_id"]));
        expect_merged(result, case, &["streams"], |s| json!({"streams": streams_json(s)}), &format!("scan {} max {}", label(case), case["max_id"]));
    }
    for case in f["parse_at"].as_array().unwrap() {
        let source = &scans[n(&case["scan"])];
        let payload = bits(&source["hex"], &source["bit_count"]);
        let result = parse_input_stream_at(&payload, n(&case["marker"]), max_id(&case["max_id"]));
        let what = format!("parse_at {} marker {}", label(source), case["marker"]);
        expect_merged(result, case, &["stream"], |s| json!({"stream": s.as_ref().map(stream_json)}), &what);
    }
    for case in f["actor_inputs"].as_array().unwrap() {
        let packet = bits(&case["hex"], &json!(case["hex"].as_str().unwrap().len() * 4));
        let result = decode_received_actor_inputs(&packet, n(&case["start"]), n(&case["end"]), max_id(&case["max_id"]));
        let what = format!("actor inputs {} max {}", label(case), case["max_id"]);
        expect_merged(result, case, &["streams"], |s| json!({"streams": streams_json(s)}), &what);
    }
}

/// the first bunch's extra header fields against python's inspect_control_bunch.
fn check_first_bunch_extras(bunch: &BunchView, inspect: &Value, name: &str) {
    if inspect.get("bunch_start_bit").and_then(Value::as_u64) != Some(bunch.start_bit as u64) || inspect.get("bunch_body_start_bit").is_none() {
        return;
    }
    let flag = |key: &str| inspect.get(key).and_then(Value::as_bool).unwrap_or(false);
    assert_eq!(bunch.is_open, flag("bunch_is_open"), "{name} open");
    assert_eq!(bunch.replication_paused, flag("bunch_replication_paused"), "{name} paused");
    assert_eq!(bunch.has_exports, flag("has_exports"), "{name} exports");
    assert_eq!(bunch.has_must_map, flag("has_must_map"), "{name} must map");
    assert_eq!(bunch.header_flag_4, flag("header_flag_4"), "{name} flag 4");
    assert_eq!(bunch.hardcoded_name, flag("hardcoded_name"), "{name} hardcoded name");
    assert_eq!(bunch.channel_name_index, inspect.get("channel_name_index").and_then(Value::as_u64), "{name} name index");
    assert_eq!(bunch.body_start_bit as u64, inspect["bunch_body_start_bit"].as_u64().unwrap(), "{name} body");
    assert_eq!(bunch.end_bit as u64, inspect["bunch_end_bit"].as_u64().unwrap(), "{name} end");
}

fn check_packet(case: &Value) {
    let name = label(case);
    let decoded = bits(&case["hex"], &case["bit_count"]);
    let walk = iter_bunches(&decoded);
    match (&walk, case.get("walk_error")) {
        (Err(error), Some(expected)) => check_error(error, expected, &format!("{name} walk")),
        (Ok(bunches), None) => {
            let expected = case["bunches"].as_array().unwrap();
            assert_eq!(bunches.len(), expected.len(), "{name} bunch count");
            for (index, (bunch, want)) in bunches.iter().zip(expected).enumerate() {
                let what = format!("{name} bunch {index}");
                let header: Map<String, Value> = bunch_json(bunch).as_object().unwrap().keys().map(|k| (k.clone(), want[k].clone())).collect();
                assert_eq!(bunch_json(bunch), Value::Object(header), "{what} header");
                let body = bunch.body(&decoded);
                assert_eq!(body.bit_count, bunch.bit_count(), "{what} body bits");
                for bit in 0..body.bit_count {
                    assert_eq!(read_bits(&body.bytes, bit, 1), read_bits(&decoded.bytes, bunch.body_start_bit + bit, 1), "{what} body bit {bit}");
                }

                let map = if bunch.channel_index == CONTROLLER_CHANNEL { CONTROLLER_ACTOR_MAP_VALUE } else { PAWN_ACTOR_MAP_VALUE };
                assert_eq!(map as u64, want["block_map"].as_u64().unwrap(), "{what} map");
                let block = read_actor_content_block(&decoded, bunch.body_start_bit, bunch.end_bit, map);
                if let (Ok(block), Some(rpc)) = (&block, want.get("rpc_header")) {
                    let field_max = rpc["field_max"].as_u64().unwrap();
                    let header = read_rpc_header(&block.payload, field_max);
                    let to_json = |h: RpcHeader| json!({"field_max": field_max, "field": h.field, "payload_bits": h.payload_bits, "params_start": h.params_start});
                    expect(header, rpc, to_json, &format!("{what} rpc header"));
                } else {
                    assert!(block.as_ref().map_or(true, |b| b.has_rep_layout), "{what}: python read no rpc header");
                }
                expect(block, &want["block"], block_json, &format!("{what} block"));
                let purchase = decode_controller_rpc(&decoded, bunch);
                expect(purchase, &want["purchase"], |p| p.as_ref().map_or(Value::Null, purchase_json), &format!("{what} purchase"));
                let settings = decode_range_settings_rpc(&decoded, bunch);
                expect(settings, &want["settings"], |r| r.as_ref().map_or(Value::Null, settings_json), &format!("{what} settings"));
                let inputs = decode_received_actor_inputs(&decoded, bunch.body_start_bit, bunch.end_bit, None);
                expect(inputs, &want["inputs"], streams_json, &format!("{what} inputs"));
            }
            if let Some(first) = bunches.first() {
                check_first_bunch_extras(first, &case["inspect"], &name);
            }
            let purchases = decode_purchase_requests_in(&decoded, bunches);
            assert_eq!(purchases, decode_purchase_requests(&decoded), "{name} decode once");
            let settings = decode_range_settings_requests_in(&decoded, bunches);
            assert_eq!(settings, decode_range_settings_requests(&decoded), "{name} decode once");
        }
        (Ok(bunches), Some(expected)) => panic!("{name}: rust walked {} bunches but python raised {expected}", bunches.len()),
        (Err(error), None) => panic!("{name}: rust walk raised {error:?} but python walked"),
    }
    let purchases: Vec<Value> = decode_purchase_requests(&decoded).iter().map(purchase_json).collect();
    assert_eq!(Value::Array(purchases), case["purchases"], "{name} purchases");
    let settings: Vec<Value> = decode_range_settings_requests(&decoded).iter().map(settings_json).collect();
    assert_eq!(Value::Array(settings), case["settings"], "{name} settings");

    // the input ack's read: the first bunch with the packet inspector's bounds
    let inspect = &case["inspect"];
    if let Some(start) = inspect.get("bunch_body_start_bit") {
        let end = n(&inspect["bunch_end_bit"]);
        let expected = case.get("first_inputs").unwrap_or_else(|| &case["bunches"][0]["inputs"]);
        let result = decode_received_actor_inputs(&decoded, n(start), end, None);
        expect(result, expected, streams_json, &format!("{name} first inputs"));
    } else {
        assert!(case.get("first_inputs").is_none());
    }

    // real captures keep the client's 0xcc trailer bits; masking then restoring them is lossless
    // (mutants are named <base>_flipN / <base>_cutN and are left out: a cut moves the last byte)
    let captured = name.starts_with("capture_") || name == "warden_fixture" || NAMED.contains(&name.as_str());
    if captured && !decoded.bit_count.is_multiple_of(8) {
        let mut zeroed = decoded.clone();
        let last = zeroed.bytes.len() - 1;
        zeroed.bytes[last] &= (1u8 << (decoded.bit_count % 8)) - 1;
        assert_eq!(with_client_trailer(&zeroed), decoded, "{name} trailer");
    }
}

/// the named real captures in decode_packets (spec 06 13.4 / 13.7).
const NAMED: [&str; 12] = [
    "acr", "settings", "ch5_field92", "ch5_field70", "ch5_field154", "ch5_unreliable_field83", "ch5_subobject", "ch8_plus_ch4", "ch8_overrun",
    "ch8_subobject", "multi_stream", "slot_two_object",
];

#[test]
fn packets_match_python() {
    let cases = match std::env::var("ARIES_DECODE_PACKETS") {
        Ok(path) => serde_json::from_str::<Value>(&std::fs::read_to_string(path).unwrap()).unwrap(),
        Err(_) => fixture("decode_packets"),
    };
    let cases = cases.as_array().unwrap();
    assert!(cases.len() > 500);
    for name in NAMED.iter().chain(&["warden_fixture", "spec_repeat_purchase", "spec_sell", "spec_settings_three", "hello", "ack_only"]) {
        assert!(cases.iter().any(|c| c["name"] == *name), "fixture lacks {name}");
    }
    for case in cases {
        check_packet(case);
    }
}

#[test]
fn net_decoded_datagrams_match_python() {
    for case in fixture("decode_datagrams").as_array().unwrap() {
        let name = label(case);
        let key: [u8; 32] = unhex(case["key"].as_str().unwrap()).try_into().unwrap();
        let packet = decode_client_packet(&unhex(case["datagram"].as_str().unwrap()), &AresGcm::new(&key));
        let Some(packet) = packet else {
            assert!(case["decoded"].is_null(), "{name}: net rejected a datagram python decoded");
            continue;
        };
        assert_eq!(packet.counter as u64, case["counter"].as_u64().unwrap(), "{name}");
        assert_eq!(packet.stateless_header as u64, case["stateless_header"].as_u64().unwrap(), "{name}");
        // net zeroes the trailer bits python keeps; restoring them gives python's bytes exactly
        let decoded = with_client_trailer(&packet.decoded);
        assert_eq!(decoded, bits(&case["decoded"], &case["bit_count"]), "{name}");
        let purchases: Vec<Value> = decode_purchase_requests(&packet.decoded).iter().map(purchase_json).collect();
        assert_eq!(Value::Array(purchases), case["purchases"], "{name}");
    }
}

fn packet_named<'a>(cases: &'a Value, name: &str) -> &'a Value {
    cases.as_array().unwrap().iter().find(|c| c["name"] == name).unwrap()
}

/// local_backend/test_range_client_rpc.py, with its literal expectations.
#[test]
fn python_client_rpc_tests() {
    let cases = fixture("decode_packets");
    let warden = packet_named(&cases, "warden_fixture");
    let packet = bits(&warden["hex"], &warden["bit_count"]);
    // test_walks_the_captured_controller_bunch (fixture: body 125, end 882)
    let bunches = iter_bunches(&packet).unwrap();
    assert_eq!(bunches.len(), 1);
    assert_eq!((bunches[0].channel_index, bunches[0].is_reliable, bunches[0].channel_sequence), (5, true, Some(863)));
    assert_eq!((bunches[0].body_start_bit, bunches[0].end_bit), (125, 882));
    // test_decodes_the_captured_warden_purchase
    let requests = decode_purchase_requests(&packet);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].field, SERVER_PURCHASE_FIELD);
    assert_eq!(requests[0].item_path.as_deref(), Some("/Game/Equippables/Guns/Rifles/BattleRifle/BattleRifle.BattleRifle_C"));
    assert_eq!((requests[0].target_slot, requests[0].purchasable_type), (0, 0));
    // test_garbage_yields_no_requests: decode_purchase_requests(bytes(64), 400)
    assert!(decode_purchase_requests(&Bits { bytes: vec![0; 64], bit_count: 400 }).is_empty());
}

/// local_backend/test_range_client_input.py, rebuilt with our writer and encoder.
#[test]
fn python_client_input_tests() {
    // test_content_transform_roundtrip
    for bit_count in [9usize, 15, 31, 32, 33, 63, 64, 65, 95, 96, 97, 343, 431, 1023] {
        let mut plain: Vec<u8> = (0..bit_count.div_ceil(8)).map(|i| ((i * 37 + 19) & 0xFF) as u8).collect();
        if bit_count % 8 != 0 {
            *plain.last_mut().unwrap() &= (1u8 << (bit_count % 8)) - 1;
        }
        let plain = Bits::new(plain, bit_count).unwrap();
        let wire = encode_block_payload(&plain, 10).unwrap();
        assert_eq!(decode_content_payload(&wire, 10).unwrap(), plain, "{bit_count}");
    }

    // test_synthetic_received_activation
    let mut payload = BitWriter::new();
    payload.append_bits(0, 13);
    payload.append_bits(0xA3, 8);
    payload.append_bits(1, 1);
    payload.append_bits(42, 32);
    payload.append_packed_uint(2);
    payload.append_bits(1, 1);
    payload.append_bits(1, 4);
    payload.append_bits((1 << 8) | (2 << 5) | (2 << 1) | 1, 11);
    payload.append_bits(1, 1);
    payload.append_bits(7, 4);
    payload.append_bits(0xA5A5_5A5A, 32);
    payload.append_bits(0, 1);
    payload.append_bits(0, 16);
    let payload = payload.finish().unwrap();
    let wire = encode_block_payload(&payload, 10).unwrap();
    let mut body = BitWriter::new();
    body.append_bits(0, 1);
    body.append_bits(1, 1);
    body.append_packed_uint(payload.bit_count as u64);
    body.append_bits_value(&wire);
    let body = body.finish().unwrap();
    let streams = decode_received_actor_inputs(&body, 0, body.bit_count, None).unwrap();
    assert_eq!(streams.len(), 1);
    assert_eq!((streams[0].first_event_id, streams[0].last_event_id), (42, 43));
    let first = &streams[0].events[0];
    assert_eq!((first.kind, first.slot, first.flag, first.state, first.action), (1, Some(2), Some(1), Some(1), Some(2)));
    assert_eq!(streams[0].events[1].kind, 7);
    assert_eq!(decode_received_actor_inputs(&body, 0, body.bit_count, Some(43)).unwrap(), streams);
    assert!(decode_received_actor_inputs(&body, 0, body.bit_count, Some(41)).unwrap().is_empty());

    // test_first_captured_channel8_packet
    let packet = unhex(concat!(
        "200a2183ffffffff470f08080e7e1988917d93132198b5ea5aaea301ca0e",
        "c955a30fd194da748ad20fe660abeeaf6430c159400b58796b0668f7ed82b6",
        "d5e8838e9a0ab60733"
    ));
    let packet = Bits { bit_count: packet.len() * 8, bytes: packet };
    let block = read_actor_content_block(&packet, 104, 553, PAWN_ACTOR_MAP_VALUE).unwrap();
    assert_eq!((block.bit_offset, block.bit_count), (122, 431));
    let streams = scan_received_input_streams(&block.payload, None).unwrap();
    assert_eq!(streams.len(), 1);
    assert_eq!((streams[0].marker_bit, streams[0].end_bit), (315, 415));
    assert_eq!((streams[0].first_event_id, streams[0].last_event_id), (0, 1));
    assert_eq!(streams[0].events.iter().map(|e| e.kind).collect::<Vec<_>>(), [6, 4]);
    assert_eq!(streams[0].events[1].object_guid, Some(0));

    // test_captured_slot_two_event
    let packet = unhex(concat!(
        "c034319cffffffff9f0908c80f3e1b88857d13aa6f45a06ca876d21ec60e",
        "ea4dd5b7a234a902a3512f65d00a416202b33288d830a45bebdb8d12e628",
        "f6a9b5d12b0ac07e681c8c75e1c5830933"
    ));
    let packet = Bits { bit_count: packet.len() * 8, bytes: packet };
    let streams = decode_received_actor_inputs(&packet, 104, 609, None).unwrap();
    assert_eq!(streams.len(), 1);
    assert_eq!((streams[0].marker_bit, streams[0].first_event_id, streams[0].last_event_id), (315, 370, 372));
    assert_eq!(streams[0].events.iter().map(|e| e.kind).collect::<Vec<_>>(), [7, 1, 5]);
    let event = &streams[0].events[1];
    assert_eq!((event.slot, event.state, event.action), (Some(2), Some(0), Some(0)));

    // test_incomplete_stream_not_acknowledged
    let mut payload = BitWriter::new();
    payload.append_bits(0xA3, 8);
    payload.append_bits(1, 1);
    payload.append_bits(100, 32);
    payload.append_packed_uint(1);
    payload.append_bits(1, 1);
    payload.append_bits(1, 4);
    payload.append_bits(5, 11);
    assert!(scan_received_input_streams(&payload.finish().unwrap(), None).unwrap().is_empty());

    // test_empty_or_unknown_stream_not_acknowledged
    let suffixes: [&[(u64, u32)]; 3] = [&[(0, 1)], &[(1, 1), (0, 32), (2, 8)], &[(1, 1), (7, 32), (2, 8), (1, 1), (15, 4), (0, 1)]];
    for suffix in suffixes {
        let mut payload = BitWriter::new();
        payload.append_bits(0xA3, 8);
        for &(value, width) in suffix {
            payload.append_bits(value, width);
        }
        assert!(scan_received_input_streams(&payload.finish().unwrap(), None).unwrap().is_empty());
    }
}

/// the responder's dedup rule (range_udp_handshake.py:1864-1867, 1934-1937):
/// skip a sequence already in the per-connection set, else record it first.
#[test]
fn dedup_keeps_first_of_each_sequence() {
    let purchase = |sequence| PurchaseRequest { field: 156, item_path: None, target_slot: 0, purchasable_type: 0, channel_sequence: sequence };
    let mut seen = HashSet::new();
    let kept = take_new_purchases(vec![purchase(5), purchase(6), purchase(5)], &mut seen);
    assert_eq!(kept.iter().map(|p| p.channel_sequence).collect::<Vec<_>>(), [5, 6]);
    let kept = take_new_purchases(vec![purchase(6), purchase(7)], &mut seen);
    assert_eq!(kept.iter().map(|p| p.channel_sequence).collect::<Vec<_>>(), [7]);
    assert_eq!(seen, HashSet::from([5, 6, 7]));

    let mut seen = HashSet::from([4094]);
    let requests = vec![RangeSettingsRequest::new(174, 4094), RangeSettingsRequest::new(177, 4095), RangeSettingsRequest::new(172, 0)];
    let kept = take_new_range_settings(requests, &mut seen);
    assert_eq!(kept.iter().map(|r| (r.field, r.channel_sequence)).collect::<Vec<_>>(), [(177, 4095), (172, 0)]);
}
