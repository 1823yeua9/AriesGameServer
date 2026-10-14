//! R1 handler key scan against check_live_packet_keys.read_current_handler_keys
//! (fixtures/oracle_keys.json, tools/golden/oracle_keys.py): same scripted regions,
//! same VirtualQueryEx / ReadProcessMemory semantics, same first hit.

use std::collections::BTreeMap;

use aries_oracle::keys::{Region, ScanSource, scan_handler_keys};
use serde_json::Value;

const PAGE: u64 = 0x1000;
const MEM_FREE: u32 = 0x10000;
const SCAN_END: u64 = 0x7FFF_FFFF_0000;

/// the python fake: regions [base, size, state, kind, protect, readable] + page bytes.
struct Space {
    regions: Vec<(u64, u64, u32, u32, u32, bool)>,
    pages: BTreeMap<u64, Vec<u8>>,
}

impl Space {
    fn readable(&self, address: u64) -> bool {
        self.regions.iter().find(|r| r.0 <= address && address < r.0 + r.1).is_some_and(|r| r.5)
    }
}

impl ScanSource for Space {
    /// like VirtualQueryEx: from the page holding address to the end of its run of equal pages.
    fn query(&self, address: u64) -> Option<Region> {
        let page = address & !(PAGE - 1);
        for &(base, size, state, kind, protect, _) in &self.regions {
            if base <= address && address < base + size {
                return Some(Region { base: page, size: base + size - page, state, kind, protect });
            }
            if base > address {
                return Some(Region { base: page, size: base - page, state: MEM_FREE, kind: 0, protect: 0x01 });
            }
        }
        Some(Region { base: page, size: SCAN_END + 0x10000 - page, state: MEM_FREE, kind: 0, protect: 0x01 })
    }

    fn read(&self, address: u64, buffer: &mut [u8]) -> Option<usize> {
        let end = address + buffer.len() as u64;
        let mut page = address & !(PAGE - 1);
        while page < end {
            if !self.readable(page) {
                return None;
            }
            let (lo, hi) = (page.max(address), (page + PAGE).min(end));
            let target = &mut buffer[(lo - address) as usize..(hi - address) as usize];
            match self.pages.get(&page) {
                Some(data) => target.copy_from_slice(&data[(lo - page) as usize..(hi - page) as usize]),
                None => target.fill(0),
            }
            page += PAGE;
        }
        Some(buffer.len())
    }
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn handler_scan_matches_python() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/oracle_keys.json");
    let cases: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut found = 0;
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let regions = case["regions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r[0].as_u64().unwrap(), r[1].as_u64().unwrap(), r[2].as_u64().unwrap() as u32, r[3].as_u64().unwrap() as u32, r[4].as_u64().unwrap() as u32, r[5].as_bool().unwrap()))
            .collect();
        let pages = case["writes"].as_array().unwrap().iter().map(|w| (w[0].as_u64().unwrap(), hex(w[1].as_str().unwrap()))).collect();
        let space = Space { regions, pages };
        let result = scan_handler_keys(&space, case["needle"].as_u64().unwrap(), 30.0);
        match (&result, case.get("ok")) {
            (Ok(keys), Some(expected)) => {
                assert_eq!(keys.handler, expected["handler"].as_u64().unwrap(), "{name} handler");
                assert_eq!(to_hex(&keys.client_send), expected["client_send"].as_str().unwrap(), "{name} client send key");
                assert_eq!(to_hex(&keys.server_send), expected["server_send"].as_str().unwrap(), "{name} server send key");
                found += 1;
            }
            (Err(_), None) => {}
            (Ok(keys), None) => panic!("{name}: python raised {}, rust found {keys:?}", case["error"]),
            (Err(error), Some(_)) => panic!("{name}: python found the handler, rust failed: {error}"),
        }
    }
    assert!(found >= 5, "only {found} scans found a handler");
}
