//! every equippable skin of the live catalog against the python
//! (server/fixtures/guns_catalog_sweep.json, tools/golden/world_guns.py
//! section_catalog_sweep): catalog_cdo_path for every skin / level / chroma id
//! and the open of every level x chroma combination on its weapon (bought gun,
//! knife b18, classic a21). python digests one text line per result with
//! zlib.crc32 per weapon; this rebuilds the same lines and digests.
//!
//! the catalog is the shared live file (3 MB), not a fixture copy: the test
//! skips when it is missing or no longer the file the fixture was exported from.

use std::path::PathBuf;

use aries_bits::Bits;
use aries_rep::RepResult;
use aries_world::cosmetics;
use aries_world::loader::{self, PurchaseAllocator};
use aries_world::melee;
use aries_world::skins::{self, CatalogEntry, CosmeticCdoPath, CosmeticKind, EquippedCdoPaths, EquippedCosmetic, SkinCatalog};
use aries_world::store;
use serde_json::{Value, json};

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// zlib.crc32 (ieee, reflected 0xEDB88320).
fn crc32(data: &[u8]) -> u32 {
    let table: Vec<u32> = (0..256u32).map(|n| (0..8).fold(n, |c, _| if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 })).collect();
    !data.iter().fold(!0u32, |crc, &byte| table[((crc ^ byte as u32) & 0xFF) as usize] ^ (crc >> 8))
}

fn digest(lines: &[String]) -> u32 {
    crc32(lines.join("\n").as_bytes())
}

/// python component_ids: the ids sharing the default's entitlement type, in entitlement order.
fn component_ids(entry: &CatalogEntry, default_id: &str) -> Vec<String> {
    let default_id = default_id.to_lowercase();
    let type_id = &entry.entitlements.iter().find(|e| e.item_id == default_id).unwrap().type_id;
    entry.entitlements.iter().filter(|e| e.type_id == *type_id).map(|e| e.item_id.clone()).collect()
}

/// one open result: (line, bits when it built).
fn body_line(prefix: &str, result: RepResult<Bits>) -> (String, Option<usize>) {
    match result {
        Ok(bits) => (format!("{prefix}|{}|{}", bits.bit_count, bits.hex()), Some(bits.bit_count)),
        Err(error) => (format!("{prefix}|err|{error}"), None),
    }
}

#[derive(Default)]
struct Weapon {
    paths: Vec<String>,
    gun: Vec<(String, Option<usize>)>,
    knife: Vec<(String, Option<usize>)>,
    a21: Vec<(String, Option<usize>)>,
}

fn live_catalog(expected_bytes: u64) -> Option<String> {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut project = std::env::var_os("ARIES_PROJECT_ROOT").map(PathBuf::from).unwrap_or_else(|| manifest.join("../../../.."));
    if !project.join("local_backend").is_dir() {
        project = PathBuf::from("C:/Users/Cayden/Desktop/valorant proj");
    }
    let path = project.join("local_backend").join(skins::SKIN_CATALOG_FILE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("skip: no catalog at {}", path.display());
        return None;
    };
    if text.len() as u64 != expected_bytes {
        eprintln!("skip: {} changed since the fixture was exported", path.display());
        return None;
    }
    Some(text)
}

#[test]
fn every_catalog_skin_matches_python() {
    let f = fixture("guns_catalog_sweep");
    let Some(text) = live_catalog(f["catalog_bytes"].as_u64().unwrap()) else {
        return;
    };
    let catalog = SkinCatalog::parse(&text).unwrap();
    let class_paths: Vec<(String, String)> = f["class_paths"].as_array().unwrap().iter().map(|p| (p[0].as_str().unwrap().into(), p[1].as_str().unwrap().into())).collect();

    let mut weapons: Vec<(String, Weapon)> = Vec::new();
    for entry in catalog.entries() {
        let weapon_id = entry.weapon_id.to_lowercase();
        let at = match weapons.iter().position(|(id, _)| *id == weapon_id) {
            Some(at) => at,
            None => {
                weapons.push((weapon_id.clone(), Weapon::default()));
                weapons.len() - 1
            }
        };
        let out = &mut weapons[at].1;
        let levels = component_ids(entry, &entry.skin_level_id);
        let chromas = component_ids(entry, &entry.chroma_id);
        let mut resolved: Vec<((CosmeticKind, String), CosmeticCdoPath)> = Vec::new();
        let kinds = [(CosmeticKind::Skin, vec![entry.skin_id.clone()]), (CosmeticKind::Level, levels.clone()), (CosmeticKind::Chroma, chromas.clone())];
        for (kind, ids) in kinds {
            for asset_id in ids {
                match skins::catalog_cdo_path(entry, kind, &asset_id) {
                    Ok(path) => {
                        out.paths.push(format!(
                            "{}|{}|{asset_id}|{}|{}|{}|{}",
                            entry.skin_id,
                            kind.as_str(),
                            path.packaged as u8,
                            path.source_asset,
                            path.package_path,
                            path.object_name
                        ));
                        resolved.push(((kind, asset_id), path));
                    }
                    Err(error) => out.paths.push(format!("{}|{}|{asset_id}|err|{error}", entry.skin_id, kind.as_str())),
                }
            }
        }
        // python dict: a repeated id keeps the last resolution
        let find = |kind: CosmeticKind, id: &str| resolved.iter().rev().find(|((k, i), _)| *k == kind && i == id).map(|(_, path)| path.clone());
        let Some(skin) = find(CosmeticKind::Skin, &entry.skin_id) else {
            continue;
        };
        let item = store::STORE_ITEMS.iter().find(|item| item.weapon_id == weapon_id);
        for level_id in &levels {
            for chroma_id in &chromas {
                let (Some(level), Some(chroma)) = (find(CosmeticKind::Level, level_id), find(CosmeticKind::Chroma, chroma_id)) else {
                    continue;
                };
                let paths = EquippedCdoPaths { weapon_id: weapon_id.clone(), skin: skin.clone(), level, chroma };
                let prefix = format!("{}|{level_id}|{chroma_id}", entry.skin_id);
                if let Some(item) = item.filter(|item| item.is_gun) {
                    let class_path = &class_paths.iter().find(|(id, _)| *id == item.weapon_id).unwrap().1;
                    let load = loader::plan_gun_load(class_path, item, &mut PurchaseAllocator::default()).unwrap();
                    out.gun.push(body_line(&prefix, loader::gun_open_body(&load, Some(&paths))));
                }
                if weapon_id == melee::MELEE_WEAPON_ID {
                    out.knife.push(body_line(&prefix, melee::build_starting_melee_cosmetic_open(&paths)));
                }
                if weapon_id == cosmetics::BASE_PISTOL_WEAPON_ID {
                    let weapon_asset = entry.raw.get("source").and_then(|s| s.get("weapon_asset")).and_then(|a| a.as_str()).unwrap();
                    let cosmetic = EquippedCosmetic {
                        weapon_id: weapon_id.clone(),
                        skin_id: paths.skin.asset_id.clone(),
                        level_id: paths.level.asset_id.clone(),
                        chroma_id: paths.chroma.asset_id.clone(),
                        name: entry.name.clone(),
                        weapon_asset: weapon_asset.to_string(),
                        skin_asset: paths.skin.source_asset.clone(),
                        level_asset: paths.level.source_asset.clone(),
                        chroma_asset: paths.chroma.source_asset.clone(),
                    };
                    out.a21.push(body_line(&prefix, cosmetics::build_base_pistol_initial_cosmetic_open(&cosmetic, &paths)));
                }
            }
        }
    }

    let expected = f["weapons"].as_array().unwrap();
    assert_eq!(weapons.len(), expected.len(), "weapons in the catalog");
    for ((weapon_id, out), row) in weapons.iter().zip(expected) {
        assert_eq!(row["weapon_id"], json!(weapon_id));
        assert_eq!(row["paths"], json!([out.paths.len(), digest(&out.paths)]), "{weapon_id} cdo paths");
        for (kind, opens) in [("gun", &out.gun), ("knife", &out.knife), ("a21", &out.a21)] {
            if opens.is_empty() {
                assert!(row.get(kind).is_none(), "{weapon_id} {kind}: python built opens");
                continue;
            }
            let lines: Vec<String> = opens.iter().map(|(line, _)| line.clone()).collect();
            let sizes: Vec<usize> = opens.iter().filter_map(|(_, bits)| *bits).collect();
            let summary = json!([opens.len(), digest(&lines), sizes.iter().max(), opens.len() - sizes.len()]);
            assert_eq!(row[kind], summary, "{weapon_id} {kind} opens");
        }
    }
}
