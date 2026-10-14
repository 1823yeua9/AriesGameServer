use aries_world::PyRandom;

#[test]
fn matches_cpython_random() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pyrandom.json");
    let cases: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    for case in cases.as_array().unwrap() {
        let seed = case["seed"].as_u64().unwrap();
        let mut r = PyRandom::new(seed);
        for v in case["u32"].as_array().unwrap() {
            assert_eq!(r.next_u32() as u64, v.as_u64().unwrap(), "seed {seed}");
        }
        let uniform = case["uniform"].as_array().unwrap();
        let mut fresh = PyRandom::new(seed);
        assert_eq!(fresh.uniform(-30.0, 30.0), uniform[0].as_f64().unwrap());
        for v in &uniform[1..] {
            assert_eq!(r.uniform(700.0, 1800.0), v.as_f64().unwrap(), "seed {seed}");
        }
    }
}
