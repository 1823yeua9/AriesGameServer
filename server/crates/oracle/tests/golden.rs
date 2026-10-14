//! GObjects owner-block pointer decode against the python (all seven slots).

use aries_oracle::decode_array_pointer;

#[test]
fn array_pointer_decode_matches_python() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/oracle_decode.json");
    let cases: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut slots = std::collections::HashSet::new();
    for case in cases.as_array().unwrap() {
        let hex = case["owner"].as_str().unwrap();
        let owner: [u8; 0x80] = (0..0x80).map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap()).collect::<Vec<_>>().try_into().unwrap();
        let expected: u64 = case["decoded"].as_str().unwrap().parse().unwrap();
        assert_eq!(decode_array_pointer(&owner), expected);
        let seed = u32::from_le_bytes(owner[0x78..0x7C].try_into().unwrap());
        let mixed_seed = seed ^ ((seed ^ (seed >> 15)) >> 12) ^ (seed << 25);
        slots.insert(0x2545_F491_4F6C_DD1Du64.wrapping_mul(mixed_seed as u64) % 7);
    }
    assert_eq!(slots.len(), 7, "every decode slot covered");
}
