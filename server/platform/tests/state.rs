#![allow(non_upper_case_globals)]
use aries_platform::{Config, Platform, inventory::Inventory, player_id};
use serde_json::{Value, json};
use std::fs;

const weapon: &str = "00000000-0000-0000-0000-000000000010";
const skin: &str = "00000000-0000-0000-0000-000000000011";
const level: &str = "00000000-0000-0000-0000-000000000012";
const chroma: &str = "00000000-0000-0000-0000-000000000013";
const variant: &str = "00000000-0000-0000-0000-000000000014";
const legacy: &str = "00000000-0000-0000-0000-000000000015";

fn catalog() -> Value {
    json!({"schema_version":1,"skins":[{"name":"Test skin","weapon_id":weapon,"skin_id":skin,"skin_level_id":level,"chroma_id":chroma,"legacy_skin_ids":[legacy],"entitlements":[
        {"type_id":"00000000-0000-0000-0000-000000000001","item_id":skin},
        {"type_id":"00000000-0000-0000-0000-000000000002","item_id":level},
        {"type_id":"00000000-0000-0000-0000-000000000003","item_id":chroma},
        {"type_id":"00000000-0000-0000-0000-000000000003","item_id":variant}]}]})
}
fn loadout(chroma_id: &str) -> Value {
    json!({"version":1,"guns":[{"iD":weapon,"skinId":skin,"skinLevelId":level,"chromaId":chroma_id}]})
}
fn inventory(directory: &std::path::Path) -> Inventory {
    let catalog_path = directory.join("catalog.json");
    fs::write(&catalog_path, catalog().to_string()).unwrap();
    Inventory::open(Some(catalog_path), Some(directory.join("skins.json"))).unwrap()
}

#[test]
fn grant_equip_variants_and_restart() {
    let directory = tempfile::tempdir().unwrap();
    let mut model = inventory(directory.path());
    assert!(model.put_loadout(loadout(variant)).is_err());
    model.grant(Some(skin)).unwrap();
    let response = model.put_loadout(loadout(variant)).unwrap();
    assert_eq!(response["guns"][0]["chromaId"], variant);
    let reopened = inventory(directory.path());
    assert_eq!(reopened.get_loadout(), model.get_loadout());
    assert_eq!(
        reopened
            .entitlements()
            .values()
            .map(Vec::len)
            .sum::<usize>(),
        4
    );
    assert_eq!(reopened.equipped[weapon], skin);
}

#[test]
fn invalid_component_does_not_mutate_state() {
    let directory = tempfile::tempdir().unwrap();
    let mut model = inventory(directory.path());
    model.grant(None).unwrap();
    let before = fs::read(directory.path().join("skins.json")).unwrap();
    assert!(model.put_loadout(loadout(legacy)).is_err());
    assert!(model.loadout.is_none());
    assert_eq!(
        before,
        fs::read(directory.path().join("skins.json")).unwrap()
    );
}

#[test]
fn persistence_failure_keeps_memory_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let mut model = inventory(directory.path());
    fs::create_dir(directory.path().join("skins.json")).unwrap();
    assert!(model.grant(None).is_err());
    assert!(model.owned.is_empty());
    assert_eq!(model.version, 1);
}

#[test]
fn legacy_skin_migration_is_saved_once() {
    let directory = tempfile::tempdir().unwrap();
    inventory(directory.path());
    let mut old_loadout = loadout(variant);
    old_loadout["guns"][0]["skinId"] = json!(legacy);
    fs::write(directory.path().join("skins.json"), json!({"schema_version":1,"player_id":player_id,"version":4,"owned":[legacy],"equipped":{weapon:legacy},"loadout":old_loadout}).to_string()).unwrap();
    let first = inventory(directory.path());
    assert_eq!(first.version, 5);
    assert_eq!(first.get_loadout()["guns"][0]["skinId"], skin);
    assert_eq!(first.get_loadout()["guns"][0]["chromaId"], variant);
    let second = inventory(directory.path());
    assert_eq!(second.version, 5);
}

#[test]
fn untrusted_account_state_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("skins.json");
    fs::write(&path, json!({"schema_version":1,"player_id":"another-account","owned":[],"equipped":{},"loadout":null,"version":1}).to_string()).unwrap();
    assert!(Inventory::open(None, Some(path)).is_err());
}

#[test]
fn signed_loadout_version_matches_python() {
    let mut model = Inventory::open(None, None).unwrap();
    assert_eq!(
        model.put_loadout(json!({"version":-1,"guns":[]})).unwrap()["version"],
        1
    );
}

#[test]
fn range_transitions_have_fresh_identity_and_notifications() {
    let platform = Platform::new(Config::default()).unwrap();
    let mut provision = platform.range_requests.subscribe();
    let mut events = platform.events.subscribe();
    let first = platform.start_range();
    assert_eq!(provision.try_recv().unwrap(), first);
    assert_eq!(platform.session_snapshot()["loopState"], "INGAME");
    assert_eq!(events.try_recv().unwrap()[0], 8);
    assert!(!platform.leave_range("unrelated-match"));
    assert!(platform.leave_range(&first));
    assert_eq!(platform.session_snapshot()["loopState"], "MENUS");
    assert_ne!(platform.start_range(), first);
}

#[test]
fn admin_requires_exact_token() {
    let platform = Platform::new(Config::default()).unwrap();
    assert!(!platform.authorized("", true));
    assert!(!platform.authorized("Bearer wrong", true));
    assert!(platform.authorized(&format!("Bearer {}", platform.config.admin_token), true));
}
