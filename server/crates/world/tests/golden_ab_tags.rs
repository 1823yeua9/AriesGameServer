//! gameplay-tag net index (spec 08 section 3) against server/fixtures/world_tags.json:
//! gameplay_tag_net_index.predict_net_indices run on fake memory images.

mod golden_ab_common;

use std::collections::HashMap;

use aries_world::tags::{TagNode, predict_net_indices, true_bit_num};
use golden_ab_common::{check, fields, fixture, python_error, u32_of};

#[test]
fn net_indices_match_python() {
    let f = fixture("world_tags");
    let cases = f["cases"].as_array().unwrap();
    assert!(cases.len() > 40);
    for case in cases {
        let label = case["label"].as_str().unwrap();
        let nodes: Vec<TagNode> = case["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                let parts = fields(row);
                assert_eq!(parts.len(), 3, "{label}");
                TagNode { name: parts[0].to_string(), number: parts[1].parse().unwrap(), net_index: parts[2].parse().unwrap() }
            })
            .collect();
        let common: Vec<String> = case["common"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
        let setting = case["setting"].as_i64().unwrap() as i32;
        let result = predict_net_indices(&nodes, &common, setting);
        if let Some(error) = python_error(case) {
            assert!(result.is_err(), "{label}: python raised {error}, rust gave {result:?}");
            continue;
        }
        let result = result.unwrap_or_else(|error| panic!("{label}: {error}"));
        let want: HashMap<String, u32> = case["indices"].as_object().unwrap().iter().map(|(name, index)| (name.clone(), u32_of(index))).collect();
        assert_eq!(result.indices, want, "{label} indices");
        assert_eq!(result.true_bit_num, u32_of(&case["true_bit_num"]), "{label} true bits");
        assert_eq!(result.first_bit_segment, case["first_bit_segment"].as_i64().unwrap(), "{label} first segment");
        assert_eq!(result.constructed, case["constructed"].as_bool().unwrap(), "{label} constructed");
    }
}

/// the client's own node map (tools/golden/_capture_live_tags.py, read-only)
/// replayed by python: our computation on the same rows, both paths, and the
/// production B18 opens with the live indices. skipped until a capture exists.
#[test]
fn live_capture_matches_python() {
    let f = fixture("world_tags_live");
    if f["captured"] == false {
        eprintln!("no live tag capture yet: run tools/golden/_capture_live_tags.py with the client up, then export world_tags_live");
        return;
    }
    let capture = fixture("world_tags_live_capture");
    let nodes: Vec<TagNode> = capture["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| TagNode { name: n[0].as_str().unwrap().to_string(), number: u32_of(&n[1]), net_index: u16::try_from(u32_of(&n[2])).unwrap() })
        .collect();
    let common: Vec<String> = capture["common"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    let setting = capture["setting"].as_i64().unwrap() as i32;
    let unbuilt: Vec<TagNode> = nodes.iter().map(|n| TagNode { net_index: 0xFFFF, ..n.clone() }).collect();
    let mut client_indices = HashMap::new();
    for (label, nodes, want) in [("client", &nodes, &f["client"]), ("computed", &unbuilt, &f["computed"])] {
        let result = predict_net_indices(nodes, &common, setting).unwrap_or_else(|error| panic!("{label}: {error}"));
        let indices: HashMap<String, u32> = want["indices"].as_object().unwrap().iter().map(|(name, index)| (name.clone(), u32_of(index))).collect();
        assert_eq!(result.indices, indices, "{label} indices");
        assert_eq!(result.true_bit_num, u32_of(&want["true_bit_num"]), "{label} true bits");
        assert_eq!(result.first_bit_segment, want["first_bit_segment"].as_i64().unwrap(), "{label} first segment");
        assert_eq!(result.constructed, want["constructed"].as_bool().unwrap(), "{label} constructed");
        if label == "client" {
            client_indices = result.indices.clone();
            for open in f["opens"].as_array().unwrap() {
                let index = open["index"].as_u64().unwrap() as usize;
                check(aries_world::abilities::build_ability_open_from_tags(index, &result), open, &format!("live open {index}"));
            }
        }
    }
    let python_live = &capture["python_live"]["tuning"];
    for (tag, index) in python_live.as_object().unwrap() {
        assert_eq!(client_indices.get(tag).copied(), index.as_u64().map(|i| i as u32), "{tag}: the replay must give what python read live");
    }
}

#[test]
fn live_tag_count_is_ten_bits() {
    let f = fixture("world_tags");
    let live = f["cases"].as_array().unwrap().iter().find(|c| c["label"] == "random_0").unwrap();
    assert_eq!(live["true_bit_num"], 10);
    for pair in f["true_bits"].as_array().unwrap() {
        let n = pair[0].as_u64().unwrap() as usize;
        assert_eq!(true_bit_num(n) as u64, pair[1].as_u64().unwrap(), "n = {n}");
    }
}
