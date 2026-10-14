use aries_platform::{inventory::Inventory, player_id};
use aries_world::skins::{InventoryCache, OFFLINE_PLAYER_ID, resolve_equipped_cosmetic};
use serde_json::{Value, json};
use std::{
    fs,
    sync::Arc,
    time::{Duration, SystemTime},
};

fn id(number: u32) -> String {
    format!("00000000-0000-0000-0000-{number:012x}")
}

fn catalog_entry(number: u32) -> Value {
    json!({
        "name": format!("Shared inventory test {number}"),
        "weapon_id": id(number),
        "skin_id": id(number + 1),
        "skin_level_id": id(number + 2),
        "chroma_id": id(number + 3),
        "source": {
            "weapon_asset": "Equippables/Test/Weapon.uasset",
            "skin_asset": "Equippables/Test/Skin.uasset",
            "level_asset": "Equippables/Test/Level.uasset",
            "chroma_asset": "Equippables/Test/Default.uasset",
            "unlock_assets": {
                "levels": ["Equippables/Test/Level.uasset"],
                "chromas": ["Equippables/Test/Default.uasset", "Equippables/Test/Variant.uasset"]
            }
        },
        "entitlements": [
            {"type_id": id(1), "item_id": id(number + 1)},
            {"type_id": id(2), "item_id": id(number + 2)},
            {"type_id": id(3), "item_id": id(number + 3)},
            {"type_id": id(3), "item_id": id(number + 4)}
        ]
    })
}

fn loadout(variant: bool) -> Value {
    let guns: Vec<Value> = [100, 200]
        .into_iter()
        .map(|number| {
            json!({
                "iD": id(number),
                "skinId": id(number + 1),
                "skinLevelId": id(number + 2),
                "chromaId": id(number + if variant { 4 } else { 3 })
            })
        })
        .collect();

    json!({"version": 1, "guns": guns})
}

#[test]
fn gameplay_reads_platform_equips_and_refreshes_without_writing() {
    assert_eq!(player_id, OFFLINE_PLAYER_ID);

    let directory = tempfile::tempdir().unwrap();
    let account = directory.path().join("shared account 测试");
    fs::create_dir(&account).unwrap();

    let catalog_path = account.join("catalog.json");
    let state_path = account.join("skins.json");
    let catalog = json!({"schema_version": 1, "skins": [catalog_entry(100), catalog_entry(200)]});
    fs::write(&catalog_path, catalog.to_string()).unwrap();

    let mut platform =
        Inventory::open(Some(catalog_path.clone()), Some(state_path.clone())).unwrap();
    let mut gameplay = InventoryCache::new(player_id, catalog_path.clone(), state_path.clone());

    // A new account is empty; the gameplay reader must not create state.
    assert!(gameplay.get().unwrap().owned.is_empty());
    assert!(!state_path.exists());

    platform.grant(None).unwrap();
    platform.put_loadout(loadout(false)).unwrap();

    // Give the first saved revision a distinct timestamp without sleeping or
    // relying on filesystem clock resolution during this cache regression test.
    fs::File::options()
        .write(true)
        .open(&state_path)
        .unwrap()
        .set_times(
            fs::FileTimes::new()
                .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000)),
        )
        .unwrap();

    let saved = fs::read(&state_path).unwrap();
    let first = gameplay.get().unwrap();
    assert!(Arc::ptr_eq(&first, &gameplay.get().unwrap()));

    for number in [100, 200] {
        let cosmetic = resolve_equipped_cosmetic(&first, &id(number))
            .unwrap()
            .unwrap();
        assert_eq!(cosmetic.skin_id, id(number + 1));
        assert_eq!(cosmetic.level_id, id(number + 2));
        assert_eq!(cosmetic.chroma_id, id(number + 3));
    }
    assert_eq!(saved, fs::read(&state_path).unwrap());

    platform.put_loadout(loadout(true)).unwrap();
    let updated = fs::read(&state_path).unwrap();
    let second = gameplay.get().unwrap();
    assert!(!Arc::ptr_eq(&first, &second));

    for number in [100, 200] {
        let cosmetic = resolve_equipped_cosmetic(&second, &id(number))
            .unwrap()
            .unwrap();
        assert_eq!(cosmetic.chroma_id, id(number + 4));
    }
    assert_eq!(updated, fs::read(&state_path).unwrap());

    let restarted = Inventory::open(Some(catalog_path), Some(state_path)).unwrap();
    assert_eq!(restarted.get_loadout(), platform.get_loadout());
}
