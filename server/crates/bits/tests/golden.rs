//! replays server/fixtures/bits.json (python BitWriter output) bit for bit.

use aries_bits::{BitReader, BitWriter, append_quantized_vector, append_quantized_vector10, append_rotator_bytes, append_rotator_shorts};
use serde_json::Value;

fn load(name: &str) -> Vec<Value> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str::<Vec<Value>>(&text).expect("fixture json")
}

fn vec3(value: &Value) -> [f64; 3] {
    let a = value.as_array().expect("vector");
    [a[0].as_f64().unwrap(), a[1].as_f64().unwrap(), a[2].as_f64().unwrap()]
}

#[test]
fn bits_match_python() {
    let cases = load("bits.json");
    assert!(cases.len() > 50);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let mut w = BitWriter::new();
        for op in case["ops"].as_array().unwrap() {
            let op = op.as_array().unwrap();
            match op[0].as_str().unwrap() {
                "bits" => w.append_bits(op[1].as_u64().unwrap(), op[2].as_u64().unwrap() as u32),
                "packed" => w.append_packed_uint(op[1].as_u64().unwrap()),
                "sint" => {
                    w.append_serialize_int(op[1].as_u64().unwrap(), op[2].as_u64().unwrap());
                }
                "fstring" => w.append_fstring_ascii(op[1].as_str().unwrap()),
                "qv10" => append_quantized_vector10(&mut w, vec3(&op[1])),
                "qv" => append_quantized_vector(&mut w, vec3(&op[1]), op[2].as_f64().unwrap()),
                "rotbytes" => {
                    let r = vec3(&op[1]);
                    append_rotator_bytes(&mut w, r[0], r[1], r[2]);
                }
                "rotshorts" => {
                    let r = vec3(&op[1]);
                    append_rotator_shorts(&mut w, r[0], r[1], r[2]);
                }
                other => panic!("unknown op {other}"),
            }
        }
        let bits = w.finish().unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(bits.bit_count as u64, case["bit_count"].as_u64().unwrap(), "{name} bit count");
        assert_eq!(bits.hex(), case["hex"].as_str().unwrap(), "{name} bytes");
    }
}

#[test]
fn reader_round_trips() {
    let mut w = BitWriter::new();
    w.append_bits(1, 1);
    w.append_packed_uint(300001);
    w.append_serialize_int(57, 59);
    w.append_fstring_ascii("BombTeam");
    w.append_f64(1351.8);
    w.append_bits(0xABC, 12);
    let bits = w.finish().unwrap();
    let mut r = BitReader::from_bits(&bits);
    assert!(r.read_bool());
    assert_eq!(r.read_packed_uint(), 300001);
    assert_eq!(r.read_serialize_int(59), 57);
    assert_eq!(r.read_fstring(), "BombTeam");
    assert_eq!(r.read_f64(), 1351.8);
    assert_eq!(r.read_bits(12), 0xABC);
    assert!(r.at_end() && !r.is_error());
    r.read_bits(1);
    assert!(r.is_error());
}

#[test]
fn errors_stick() {
    let mut w = BitWriter::new();
    w.append_bits(4, 2);
    w.append_bits(1, 1);
    assert!(w.is_error());
    assert_eq!(w.bit_count(), 0);
    let mut w = BitWriter::new();
    w.append_serialize_int(59, 59);
    assert!(w.finish().is_err());
    let mut w = BitWriter::new();
    w.append_fstring_ascii("");
    assert!(w.finish().is_err());
}
