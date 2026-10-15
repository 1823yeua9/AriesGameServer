//! keybinds and roaming settings survive a restart: the config turns roaming on,
//! and what the game PUTs is what a new platform GETs back.

use aries_platform::{Config, Platform, ROAMING_SETTINGS_KEY};
use serde_json::json;

fn platform(persistence: &std::path::Path) -> std::sync::Arc<Platform> {
    Platform::new(Config { persistence_path: persistence.into(), ..Config::default() }).unwrap()
}

#[test]
fn roaming_settings_are_enabled() {
    let folder = std::env::temp_dir().join(format!("aries_prefs_config_{}", std::process::id()));
    let (status, config) = platform(&folder).route("GET", "/v1/config/na", b"");
    assert_eq!(status, 200);
    assert_eq!(config["Collapsed"]["GAME_ROAMINGSETTINGS_ENABLED"], "true");
    assert_eq!(config["Collapsed"]["GAME_ROAMINGSETTINGS_KEY"], ROAMING_SETTINGS_KEY);
}

#[test]
fn saved_preferences_survive_a_restart() {
    let folder = std::env::temp_dir().join(format!("aries_prefs_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    let route = format!("/player-preferences/v1/data-json/{ROAMING_SETTINGS_KEY}");
    // a new player gets an empty object and the game's defaults
    assert_eq!(platform(&folder).route("GET", &route, b""), (200, json!({})));
    let saved = json!({"version": 3, "actionMappings": [{"name": "Jump", "key": "SpaceBar"}], "floatSettings": [{"settingEnum": "EAresFloatSettingName::MouseSensitivity", "value": 0.61}]});
    let (status, _) = platform(&folder).route("PUT", &route, serde_json::to_vec(&saved).unwrap().as_slice());
    assert_eq!(status, 200);
    assert_eq!(platform(&folder).route("GET", &route, b""), (200, saved));
    // only plain type names, never a path
    assert_eq!(platform(&folder).route("GET", "/player-preferences/v1/data-json/..%2F..%2Fx", b"").0, 400);
    assert_eq!(platform(&folder).route("PUT", "/player-preferences/v1/data-json/..", b"{}").0, 500);
    let _ = std::fs::remove_dir_all(&folder);
}
