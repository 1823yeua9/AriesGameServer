//! the B18 tuning hook (spec 08 section 2.2, 04 B18) against
//! server/fixtures/world_ability_open_hook.json: ruh's own hook statement run
//! on a filled tag cache for every starting index.

mod golden_ab_common;

use std::collections::HashMap;

use aries_world::abilities::{build_ability_open_from_tags, starting_ability_tunings};
use aries_world::tags::TagNetIndex;
use golden_ab_common::{check, fixture, u32_of};
use serde_json::json;

#[test]
fn hook_opens_match_python() {
    let f = fixture("world_ability_open_hook");
    assert_eq!(f["line"], 4046);
    let (mut tuned, mut fallbacks) = (0, 0);
    for cache in f["caches"].as_array().unwrap() {
        let label = cache["label"].as_str().unwrap();
        let tags = TagNetIndex {
            indices: cache["indices"].as_object().unwrap().iter().map(|(name, index)| (name.clone(), u32_of(index))).collect::<HashMap<_, _>>(),
            true_bit_num: u32_of(&cache["true_bit_num"]),
            first_bit_segment: cache["first_bit_segment"].as_i64().unwrap(),
            constructed: cache["constructed"].as_bool().unwrap(),
        };
        let activation = cache["activation"].as_bool().unwrap();
        for row in cache["rows"].as_array().unwrap() {
            let index = row["index"].as_u64().unwrap() as usize;
            let label = format!("{label} index {index}");
            let event = &row["event"];
            let kept_default = row.get("default").is_some();
            // ruh:4045: the hook only runs for the tuned abilities with ability_activation on
            if !(activation && starting_ability_tunings(index).is_some()) {
                assert!(kept_default && event.as_object().unwrap().is_empty(), "{label}: python changed the open");
                continue;
            }
            let result = build_ability_open_from_tags(index, &tags);
            if kept_default {
                assert!(result.is_err(), "{label}: python fell back, rust built {result:?}");
                assert!(event.get("ability_open_tuning_error").is_some(), "{label}: no error event");
                fallbacks += 1;
            } else {
                check(result, row, &label);
                assert_eq!(event["ability_open_tuning"], json!([index, tags.true_bit_num, tags.constructed]), "{label} event");
                assert!(event.get("ability_open_tuning_error").is_none());
                tuned += 1;
            }
        }
    }
    assert!(tuned >= 15 && fallbacks >= 8, "{tuned} tuned, {fallbacks} fallbacks");
}
