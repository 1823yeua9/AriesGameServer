//! skin selection and cosmetic opens against the python
//! (server/fixtures/guns_skins.json, guns_cosmetics.json).
//!
//! guns_skins.json holds the saved state snapshot, python's resolution of every
//! catalog weapon on the full catalog, a catalog subset that resolves the same,
//! and synthetic edge cases stored as patches on a small base catalog / state.
//! it is read with the crate's ordered Json so patched documents keep python's
//! dict order.

use std::sync::Arc;

use aries_bits::Bits;
use aries_rep::{RepError, RepResult};
use aries_world::cosmetics::{self, BasePistolOpen, CosmeticActorLayout};
use aries_world::loader;
use aries_world::melee::{self, MeleeOpen};
use aries_world::skins::{self, CosmeticCdoPath, CosmeticKind, EquippedCdoPaths, EquippedCosmetic, InventoryCache, Json, SkinCatalog, SkinError, SkinInventory};
use serde_json::{Value, json};

fn fixture_text(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../fixtures/{name}.json"));
    std::fs::read_to_string(path).unwrap()
}

fn at<'a>(json: &'a Json, key: &str) -> &'a Json {
    json.get(key).unwrap_or_else(|| panic!("missing {key}"))
}

fn text(json: &Json) -> &str {
    json.as_str().unwrap_or_else(|| panic!("not a str: {json:?}"))
}

fn items(json: &Json) -> &Vec<Json> {
    json.as_array().unwrap_or_else(|| panic!("not a list: {json:?}"))
}

fn index(json: &Json) -> usize {
    json.as_i64().unwrap() as usize
}

fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Object(pairs.into_iter().map(|(key, value)| (key.to_string(), value)).collect())
}

fn st(value: &str) -> Json {
    Json::Str(value.to_string())
}

fn cosmetic_json(c: &EquippedCosmetic) -> Json {
    obj(vec![
        ("weapon_id", st(&c.weapon_id)),
        ("skin_id", st(&c.skin_id)),
        ("level_id", st(&c.level_id)),
        ("chroma_id", st(&c.chroma_id)),
        ("name", st(&c.name)),
        ("weapon_asset", st(&c.weapon_asset)),
        ("skin_asset", st(&c.skin_asset)),
        ("level_asset", st(&c.level_asset)),
        ("chroma_asset", st(&c.chroma_asset)),
    ])
}

fn cdo_json(p: &CosmeticCdoPath) -> Json {
    obj(vec![
        ("kind", st(p.kind.as_str())),
        ("asset_id", st(&p.asset_id)),
        ("source_asset", st(&p.source_asset)),
        ("package_path", st(&p.package_path)),
        ("object_name", st(&p.object_name)),
        ("packaged", Json::Bool(p.packaged)),
    ])
}

fn paths_json(p: &EquippedCdoPaths) -> Json {
    obj(vec![("weapon_id", st(&p.weapon_id)), ("skin", cdo_json(&p.skin)), ("level", cdo_json(&p.level)), ("chroma", cdo_json(&p.chroma))])
}

fn error_json(error: &SkinError) -> Json {
    match error {
        SkinError::Inventory(message) => obj(vec![("error", st("SkinInventoryError")), ("message", st(message))]),
        SkinError::Other(message) => obj(vec![("error", st("Other")), ("message", st(message))]),
    }
}

/// python value(fn): {"value": v} or {"error", "message"}.
fn value_json<T>(result: Result<Option<T>, SkinError>, to_json: impl Fn(&T) -> Json) -> Json {
    match result {
        Ok(value) => obj(vec![("value", value.as_ref().map_or(Json::Null, to_json))]),
        Err(error) => error_json(&error),
    }
}

/// python query_inventory's per-weapon record.
fn query_json(inventory: &SkinInventory, weapon_id: &str) -> Json {
    obj(vec![
        ("weapon_id", st(weapon_id)),
        ("cosmetic", value_json(skins::resolve_equipped_cosmetic(inventory, weapon_id), cosmetic_json)),
        ("cdo_paths", value_json(skins::resolve_equipped_cdo_paths(inventory, weapon_id), paths_json)),
        ("saved", value_json(loader::select_saved_cdo_paths(inventory, weapon_id), paths_json)),
        ("standard", value_json(loader::standard_cdo_paths(inventory, weapon_id), paths_json)),
    ])
}

/// python inventory_dict + the rewrite flag.
fn inventory_json(inventory: &SkinInventory, rewritten: Option<bool>) -> Json {
    let mut owned: Vec<&String> = inventory.owned.iter().collect();
    owned.sort();
    let mut equipped = inventory.equipped.clone();
    equipped.sort();
    let mut pairs = Vec::new();
    if let Some(rewritten) = rewritten {
        pairs.push(("rewritten", Json::Bool(rewritten)));
    }
    pairs.extend([
        ("owned", Json::Array(owned.into_iter().map(|id| st(id)).collect())),
        ("equipped", Json::Array(equipped.iter().map(|(k, v)| Json::Array(vec![st(k), st(v)])).collect())),
        ("loadout", inventory.loadout.clone().unwrap_or(Json::Null)),
        ("version", Json::Int(inventory.version)),
    ]);
    obj(pairs)
}

fn check_body(bits: &Bits, expected: &Json, bodies: &[Json], label: &str) {
    assert_eq!(bits.bit_count as i64, at(expected, "bit_count").as_i64().unwrap(), "{label} bits");
    assert_eq!(bits.hex(), text(&bodies[index(at(expected, "body"))]), "{label}");
}

/// the a21 classic open and the b18 knife open for this inventory.
fn check_opens(inventory: &SkinInventory, pistol: &Json, melee_case: &Json, bodies: &[Json], label: &str) {
    match skins::select_cosmetic_loadout(inventory, cosmetics::BASE_PISTOL_WEAPON_ID) {
        Err(error) => {
            assert!(pistol.get("kind").is_none(), "{label}: python selected, rust failed with {error}");
            assert_eq!(error_json(&error), *pistol, "{label} pistol selection");
        }
        Ok(selection) => match cosmetics::base_pistol_open(selection.as_ref()) {
            Ok((bits, kind)) => {
                let kind = match kind {
                    BasePistolOpen::Cosmetic => "cosmetic",
                    BasePistolOpen::Unshipped => "unshipped",
                    BasePistolOpen::Unselected => "unselected",
                };
                assert_eq!(kind, text(at(pistol, "kind")), "{label} pistol kind");
                check_body(&bits, pistol, bodies, &format!("{label} pistol"));
            }
            Err(error) => assert!(pistol.get("error").is_some(), "{label}: rust pistol open failed: {error}"),
        },
    }
    let (bits, kind) = melee::melee_open(Ok(inventory)).unwrap();
    let kind = match kind {
        MeleeOpen::Cosmetic(_) => "cosmetic",
        MeleeOpen::Unselected => "unselected",
        MeleeOpen::Error(_) => "error",
    };
    assert_eq!(kind, text(at(melee_case, "kind")), "{label} melee kind");
    check_body(&bits, melee_case, bodies, &format!("{label} melee"));
}

fn walk<'a>(doc: &'a mut Json, path: &[Json]) -> &'a mut Json {
    path.iter().fold(doc, |node, key| match key {
        Json::Str(key) => node.get_mut(key).unwrap(),
        Json::Int(i) => &mut node.as_array_mut().unwrap()[*i as usize],
        other => panic!("bad path element {other:?}"),
    })
}

/// the exporter's json_patch: set (in place or appended), del, push.
fn apply_patch(doc: &mut Json, ops: &Json) {
    for op in items(ops) {
        let op = items(op);
        let path = items(&op[1]);
        match text(&op[0]) {
            "set" if path.is_empty() => *doc = op[2].clone(),
            "push" => walk(doc, path).as_array_mut().unwrap().push(op[2].clone()),
            kind => {
                let (last, parents) = path.split_last().unwrap();
                let parent = walk(doc, parents);
                match (kind, last) {
                    ("del", Json::Str(key)) => {
                        parent.remove(key);
                    }
                    ("del", Json::Int(i)) => {
                        parent.as_array_mut().unwrap().remove(*i as usize);
                    }
                    ("set", Json::Str(key)) => parent.set(key.clone(), op[2].clone()),
                    ("set", Json::Int(i)) => parent.as_array_mut().unwrap()[*i as usize] = op[2].clone(),
                    other => panic!("bad op {other:?}"),
                }
            }
        }
    }
}

fn inventory_from(player: &str, catalog: &Json, state: Option<&Json>) -> Result<SkinInventory, SkinError> {
    let catalog = Arc::new(SkinCatalog::from_json(catalog)?);
    SkinInventory::from_documents(player, catalog, state)
}

fn check_against_saved(f: &Json, inventory: &SkinInventory, expected_inventory: &Json, label: &str) {
    assert_eq!(inventory_json(inventory, None), Json::parse(text(expected_inventory)).unwrap(), "{label} inventory");
    for query in items(at(f, "weapons")) {
        let query = Json::parse(text(query)).unwrap();
        assert_eq!(query_json(inventory, text(at(&query, "weapon_id"))), query, "{label}");
    }
    let bodies = items(at(f, "bodies"));
    check_opens(inventory, at(f, "base_pistol_open"), at(f, "melee_open"), bodies, label);
}

fn skins_fixture() -> Json {
    Json::parse(&fixture_text("guns_skins")).unwrap()
}

#[test]
fn saved_selection_on_catalog_subset_matches_python() {
    let f = skins_fixture();
    let player = text(at(&f, "player"));
    let subset = at(&f, "subset");
    let catalog = Json::parse(text(at(subset, "catalog_text"))).unwrap();
    let state = Json::parse(text(at(subset, "state_text"))).unwrap();
    let inventory = inventory_from(player, &catalog, Some(&state)).unwrap();
    assert_eq!(inventory.migration_pending, at(subset, "state_rewritten").as_bool().unwrap());
    check_against_saved(&f, &inventory, at(subset, "inventory"), "subset");
}

/// the live catalog the exporter read (shared with the platform backend), when
/// it is still the same file; the state is the fixture's snapshot.
#[test]
fn saved_selection_on_full_catalog_matches_python() {
    let f = skins_fixture();
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut project = std::env::var_os("ARIES_PROJECT_ROOT").map(std::path::PathBuf::from).unwrap_or_else(|| manifest.join("../../../.."));
    if !project.join("local_backend").is_dir() {
        project = std::path::PathBuf::from("C:/Users/Cayden/Desktop/valorant proj");
    }
    let catalog_path = project.join("local_backend").join(skins::SKIN_CATALOG_FILE);
    let Ok(catalog_text) = std::fs::read_to_string(&catalog_path) else {
        eprintln!("skip: no catalog at {}", catalog_path.display());
        return;
    };
    let expected = at(&f, "catalog");
    if catalog_text.len() as i64 != at(expected, "bytes").as_i64().unwrap() {
        eprintln!("skip: {} changed since the fixture was exported", catalog_path.display());
        return;
    }
    let catalog = SkinCatalog::parse(&catalog_text).unwrap();
    assert_eq!(catalog.len() as i64, at(expected, "skins").as_i64().unwrap());
    let state = Json::parse(text(at(&f, "state_text"))).unwrap();
    let inventory = SkinInventory::from_documents(text(at(&f, "player")), Arc::new(catalog), Some(&state)).unwrap();
    assert_eq!(inventory.migration_pending, at(&f, "state_rewritten").as_bool().unwrap());
    check_against_saved(&f, &inventory, at(&f, "inventory"), "full");
}

#[test]
fn skin_edge_cases_match_python() {
    let f = skins_fixture();
    let player = text(at(&f, "player"));
    let base_catalog = Json::parse(text(at(&f, "base_catalog_text"))).unwrap();
    let base_state = Json::parse(text(at(&f, "base_state_text"))).unwrap();
    let queries = items(at(&f, "queries"));
    let bodies = items(at(&f, "bodies"));
    for case in items(at(&f, "cases")) {
        let name = text(at(case, "name"));
        let mut catalog = base_catalog.clone();
        apply_patch(&mut catalog, at(case, "catalog_patch"));
        let state = case.get("state_absent").is_none().then(|| {
            let mut state = base_state.clone();
            apply_patch(&mut state, at(case, "state_patch"));
            state
        });
        let result = inventory_from(player, &catalog, state.as_ref());
        let load = at(case, "load");
        if load.get("error").is_some() {
            let error = result.expect_err(name);
            match text(at(load, "error")) {
                "SkinInventoryError" => assert_eq!(error, SkinError::Inventory(text(at(load, "message")).into()), "{name}"),
                other => assert!(matches!(error, SkinError::Other(_)), "{name}: python {other}, rust {error:?}"),
            }
            continue;
        }
        let inventory = result.unwrap_or_else(|e| panic!("{name}: {e}"));
        let expected = Json::parse(text(load)).unwrap();
        assert_eq!(inventory_json(&inventory, Some(inventory.migration_pending)), expected, "{name} load");
        for query in items(at(case, "queries")) {
            let query = Json::parse(text(&queries[index(query)])).unwrap();
            assert_eq!(query_json(&inventory, text(at(&query, "weapon_id"))), query, "{name}");
        }
        check_opens(&inventory, at(case, "base_pistol_open"), at(case, "melee_open"), bodies, name);
    }
}

#[test]
fn catalog_cdo_path_matches_python() {
    let f = skins_fixture();
    let catalog = SkinCatalog::parse(text(at(&f, "cdo_path_catalog"))).unwrap();
    for row in items(at(&f, "cdo_paths")) {
        let row = Json::parse(text(row)).unwrap();
        let entry = catalog.get(text(at(&row, "skin_id"))).unwrap();
        let Some(kind) = CosmeticKind::from_name(text(at(&row, "kind"))) else {
            // python takes any kind string; an unknown one always fails
            assert!(row.get("error").is_some(), "{row:?}");
            continue;
        };
        let result = skins::catalog_cdo_path(entry, kind, text(at(&row, "asset_id")));
        let got = match result {
            Ok(path) => obj(vec![("value", cdo_json(&path))]),
            Err(error) => error_json(&error),
        };
        let mut expected = row.clone();
        for key in ["skin_id", "kind", "asset_id"] {
            expected.remove(key);
        }
        assert_eq!(got, expected, "{row:?}");
    }
}

#[test]
fn inventory_cache_reloads_on_state_change() {
    let f = skins_fixture();
    let subset = at(&f, "subset");
    let dir = std::env::temp_dir().join(format!("aries_world_skins_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(skins::SKIN_CATALOG_FILE), text(at(subset, "catalog_text"))).unwrap();
    let state_path = dir.join(skins::SKIN_STATE_FILE);
    std::fs::write(&state_path, text(at(subset, "state_text"))).unwrap();
    let mut cache = InventoryCache::in_dir(skins::OFFLINE_PLAYER_ID, &dir);
    let first = cache.get().unwrap();
    assert!(Arc::ptr_eq(&first, &cache.get().unwrap()), "same mtime reuses the inventory");
    let file = std::fs::OpenOptions::new().write(true).open(&state_path).unwrap();
    file.set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000)).unwrap();
    drop(file);
    let second = cache.get().unwrap();
    assert!(!Arc::ptr_eq(&first, &second), "a new mtime reloads");
    assert_eq!(first.equipped, second.equipped);
    // a missing state file is an empty selection, a missing catalog an error
    std::fs::remove_file(&state_path).unwrap();
    assert!(cache.get().unwrap().equipped.is_empty());
    std::fs::remove_file(dir.join(skins::SKIN_CATALOG_FILE)).unwrap();
    assert!(matches!(InventoryCache::in_dir(skins::OFFLINE_PLAYER_ID, &dir).get(), Err(SkinError::Inventory(_))));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- cosmetic opens

fn cosmetics_fixture() -> Value {
    serde_json::from_str(&fixture_text("guns_cosmetics")).unwrap()
}

fn s(value: &Value) -> &str {
    value.as_str().unwrap_or_else(|| panic!("not a str: {value}"))
}

fn u(value: &Value) -> u32 {
    value.as_u64().unwrap() as u32
}

fn check(result: RepResult<Bits>, case: &Value, label: &str) {
    match case.get("error") {
        Some(kind) => {
            let error = result.expect_err(label);
            match &error {
                RepError::Invalid(message) => assert_eq!(message, s(&case["message"]), "{label}"),
                // bit-writer failures: python raises ValueError (or its UnicodeEncodeError subclass)
                _ => assert!(matches!(s(kind), "ValueError" | "UnicodeEncodeError"), "{label}: {error}"),
            }
        }
        None => {
            let bits = result.unwrap_or_else(|e| panic!("{label}: {e}"));
            assert_eq!(bits.bit_count as u64, case["bit_count"].as_u64().unwrap(), "{label} bits");
            assert_eq!(bits.hex(), s(&case["hex"]), "{label}");
        }
    }
}

fn cdo_path(d: &Value) -> CosmeticCdoPath {
    CosmeticCdoPath {
        kind: CosmeticKind::from_name(s(&d["kind"])).unwrap(),
        asset_id: s(&d["asset_id"]).into(),
        source_asset: s(&d["source_asset"]).into(),
        package_path: s(&d["package_path"]).into(),
        object_name: s(&d["object_name"]).into(),
        packaged: d["packaged"].as_bool().unwrap(),
    }
}

fn paths(d: &Value) -> EquippedCdoPaths {
    EquippedCdoPaths { weapon_id: s(&d["weapon_id"]).into(), skin: cdo_path(&d["skin"]), level: cdo_path(&d["level"]), chroma: cdo_path(&d["chroma"]) }
}

fn layout(d: &Value) -> CosmeticActorLayout {
    let three = |v: &Value| -> [u32; 3] { std::array::from_fn(|i| u(&v[i])) };
    CosmeticActorLayout {
        weapon_id: s(&d["weapon_id"]).into(),
        channel_index: u(&d["channel"]),
        actor_net_guid: u(&d["actor"]),
        archetype_name: s(&d["archetype_name"]).into(),
        archetype_package: s(&d["archetype_package"]).into(),
        archetype_net_guid: u(&d["archetype_guid"]),
        package_net_guid: u(&d["package_guid"]),
        level_net_guid: u(&d["level_guid"]),
        pointer_handles: three(&d["pointer_handles"]),
        uuid_handles: three(&d["uuid_handles"]),
        asset_guid_pairs: std::array::from_fn(|i| (u(&d["pairs"][i][0]), u(&d["pairs"][i][1]))),
    }
}

fn layout_value(l: &CosmeticActorLayout) -> Value {
    json!({
        "weapon_id": l.weapon_id, "channel": l.channel_index, "actor": l.actor_net_guid, "archetype_name": l.archetype_name,
        "archetype_package": l.archetype_package, "archetype_guid": l.archetype_net_guid, "package_guid": l.package_net_guid,
        "level_guid": l.level_net_guid, "pointer_handles": l.pointer_handles, "uuid_handles": l.uuid_handles,
        "pairs": l.asset_guid_pairs.iter().map(|&(a, b)| json!([a, b])).collect::<Vec<_>>(),
    })
}

fn cosmetic(d: &Value) -> EquippedCosmetic {
    let f = |key: &str| s(&d[key]).to_string();
    EquippedCosmetic {
        weapon_id: f("weapon_id"),
        skin_id: f("skin_id"),
        level_id: f("level_id"),
        chroma_id: f("chroma_id"),
        name: f("name"),
        weapon_asset: f("weapon_asset"),
        skin_asset: f("skin_asset"),
        level_asset: f("level_asset"),
        chroma_asset: f("chroma_asset"),
    }
}

#[test]
fn cosmetic_opens_match_python() {
    let f = cosmetics_fixture();
    for case in f["words"].as_array().unwrap() {
        let result = cosmetics::guid_text_to_words(s(&case["text"]));
        match case.get("words") {
            Some(words) => assert_eq!(json!(result.unwrap()), *words, "{}", case["text"]),
            None => assert_eq!(result.unwrap_err(), RepError::Invalid(s(&case["message"]).into()), "{}", case["text"]),
        }
    }
    assert_eq!(layout_value(&cosmetics::base_pistol_cosmetic_layout()), f["layouts"]["base_pistol"]);
    assert_eq!(layout_value(&melee::starting_melee_cosmetic_layout()), f["layouts"]["melee"]);
    for case in f["opens"].as_array().unwrap() {
        let attachments: Vec<(u32, [u32; 4])> = case["attachments"].as_array().unwrap().iter().map(|a| (u(&a[0]), std::array::from_fn(|i| u(&a[1][i])))).collect();
        let result = cosmetics::build_catalog_cosmetic_actor_open(&paths(&case["paths"]), &layout(&case["layout"]), &attachments);
        check(result, case, s(&case["name"]));
    }
    for case in f["pistol_initial"].as_array().unwrap() {
        let result = cosmetics::build_base_pistol_initial_cosmetic_open(&cosmetic(&case["cosmetic"]), &paths(&case["paths"]));
        check(result, case, s(&case["name"]));
    }
    let plain = &f["plain"];
    check(cosmetics::base_pistol_plain_open(), &plain["base_pistol"], "pistol plain");
    check(melee::melee_plain_open(), &plain["melee"], "melee plain");
    let knife = f["opens"].as_array().unwrap().iter().find(|case| case["name"] == "knife_saved").unwrap();
    check(melee::build_starting_melee_cosmetic_open(&paths(&knife["paths"])), &plain["knife_saved_open"], "knife saved");
}
