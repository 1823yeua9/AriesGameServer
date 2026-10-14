//! skin catalog + saved selection (skin_inventory.py) and equipped cosmetic
//! resolution to catalog cdo paths (equipped_cosmetic.py). spec 07 7.4-7.5.
//!
//! skin_catalog.json and offline_skin_state.json are shared with the platform
//! backend, which owns every write. this is the read path only: when a saved
//! legacy id needs migrating, python's SkinInventory rewrites the state file
//! (skin_inventory.py:191-198); here the migration is applied in memory, flagged
//! by `migration_pending`, and the file is left alone (07 open question 4).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// the offline account the range resolves skins for (backend.py:36 offline_player_id).
pub const OFFLINE_PLAYER_ID: &str = "7c9f41a4-65e2-4d81-a8d5-f879c3e5ab61";
pub const SKIN_CATALOG_FILE: &str = "skin_catalog.json";
pub const SKIN_STATE_FILE: &str = "offline_skin_state.json";

/// a failed load or resolution.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SkinError {
    /// python SkinInventoryError, same message
    #[error("{0}")]
    Inventory(String),
    /// what python raises as another exception (json, io, a malformed top level, int())
    #[error("{0}")]
    Other(String),
}

fn inventory_error(message: impl Into<String>) -> SkinError {
    SkinError::Inventory(message.into())
}

// ---------------------------------------------------------------- json with python dict semantics

/// a json document as python's json.loads sees it: objects keep insertion
/// order and a repeated key replaces the value in place (dict semantics).
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i128),
    Float(f64),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn parse(text: &str) -> Result<Json, SkinError> {
        serde_json::from_str(text).map_err(|error| SkinError::Other(format!("invalid json: {error}")))
    }

    /// dict.get on an object; None for a missing key or a non-object.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(entries) => entries.iter().find(|(name, _)| name == key).map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Json> {
        match self {
            Json::Object(entries) => entries.iter_mut().find(|(name, _)| name == key).map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// dict assignment: an existing key keeps its place. no-op on a non-object.
    pub fn set(&mut self, key: impl Into<String>, value: Json) {
        if let Json::Object(entries) = self {
            let key = key.into();
            match entries.iter_mut().find(|(name, _)| *name == key) {
                Some(entry) => entry.1 = value,
                None => entries.push((key, value)),
            }
        }
    }

    /// del dict[key]; returns the value.
    pub fn remove(&mut self, key: &str) -> Option<Json> {
        match self {
            Json::Object(entries) => entries.iter().position(|(name, _)| name == key).map(|at| entries.remove(at).1),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Json>> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Json>> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&Vec<(String, Json)>> {
        match self {
            Json::Object(entries) => Some(entries),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Int(value) => i64::try_from(*value).ok(),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Float(value) => Some(*value),
            Json::Int(value) => Some(*value as f64),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }

    pub fn is_object(&self) -> bool {
        matches!(self, Json::Object(_))
    }

    /// python `value == 1` (1, 1.0 and True all compare equal).
    fn equals_one(&self) -> bool {
        match self {
            Json::Int(value) => *value == 1,
            Json::Float(value) => *value == 1.0,
            Json::Bool(value) => *value,
            _ => false,
        }
    }
}

impl<'de> serde::Deserialize<'de> for Json {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(JsonVisitor)
    }
}

struct JsonVisitor;

impl<'de> serde::de::Visitor<'de> for JsonVisitor {
    type Value = Json;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a json value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Json, E> {
        Ok(Json::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Json, E> {
        Ok(Json::Int(value as i128))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Json, E> {
        Ok(Json::Int(value as i128))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Json, E> {
        Ok(Json::Float(value))
    }

    fn visit_str<E>(self, value: &str) -> Result<Json, E> {
        Ok(Json::Str(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Json, E> {
        Ok(Json::Str(value))
    }

    fn visit_unit<E>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }

    fn visit_none<E>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }

    fn visit_some<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<Json, D::Error> {
        serde::Deserialize::deserialize(deserializer)
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(Json::Array(items))
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
        let mut object = Json::Object(Vec::new());
        while let Some((key, value)) = map.next_entry::<String, Json>()? {
            object.set(key, value);
        }
        Ok(object)
    }
}

// ---------------------------------------------------------------- python helpers

/// python SkinInventory._valid_uuid (skin_inventory.py:37-44):
/// str(uuid.UUID(v)) == v.lower(), i.e. canonical 8-4-4-4-12 hex in any case.
pub fn valid_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

fn valid_uuid_json(value: Option<&Json>) -> bool {
    value.and_then(Json::as_str).is_some_and(valid_uuid)
}

/// python str.lower()
fn lower(value: &str) -> String {
    value.to_lowercase()
}

/// python int(value) for the state's version field.
fn py_int_value(value: &Json) -> Result<i128, SkinError> {
    let error = || SkinError::Other("version is not an integer".into());
    match value {
        Json::Int(value) => Ok(*value),
        Json::Bool(value) => Ok(*value as i128),
        Json::Float(value) if value.is_finite() => Ok(value.trunc().clamp(i128::MIN as f64, i128::MAX as f64) as i128),
        Json::Str(text) => match crate::store::py_int(text) {
            Some(Some(value)) => Ok(value),
            Some(None) => Ok(i128::MAX),
            None => Err(error()),
        },
        _ => Err(error()),
    }
}

/// loadout.get("guns", loadout.get("Guns", [])): Some(guns) when it is a list.
fn loadout_guns(loadout: &Json) -> Option<&Vec<Json>> {
    match loadout.get("guns") {
        Some(guns) => guns.as_array(),
        None => loadout.get("Guns").map_or(Some(&EMPTY), Json::as_array),
    }
}

fn loadout_guns_mut(loadout: &mut Json) -> Option<&mut Vec<Json>> {
    let key = if loadout.contains_key("guns") { "guns" } else { "Guns" };
    loadout.get_mut(key).and_then(Json::as_array_mut)
}

static EMPTY: Vec<Json> = Vec::new();

/// gun.get("iD", gun.get("ID")) when it is a str.
fn gun_weapon_id(gun: &Json) -> Option<&str> {
    match gun.get("iD") {
        Some(value) => value.as_str(),
        None => gun.get("ID").and_then(Json::as_str),
    }
}

/// python SkinInventory._valid_loadout (skin_inventory.py:216-223).
fn valid_loadout(loadout: &Json) -> bool {
    loadout.is_object() && loadout_guns(loadout).is_some_and(|guns| guns.len() <= 128 && guns.iter().all(Json::is_object))
}

// ---------------------------------------------------------------- catalog

/// one catalog entitlement, lowercased (every python use lowercases).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entitlement {
    pub type_id: String,
    pub item_id: String,
}

/// one skin_catalog.json entry. ids are kept as written; `raw` is the whole
/// entry, read lazily the way python reads entry["source"].
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogEntry {
    pub name: String,
    pub weapon_id: String,
    pub skin_id: String,
    pub skin_level_id: String,
    pub chroma_id: String,
    pub entitlements: Vec<Entitlement>,
    pub raw: Json,
}

impl CatalogEntry {
    /// python _granted_component_ids (skin_inventory.py:225-238): the item ids
    /// sharing the default component's entitlement type.
    pub fn granted_component_ids(&self, default_id: &str) -> HashSet<String> {
        let default_id = lower(default_id);
        let Some(type_id) = self.entitlements.iter().find(|e| e.item_id == default_id).map(|e| e.type_id.clone()) else {
            return HashSet::new();
        };
        self.entitlements.iter().filter(|e| e.type_id == type_id).map(|e| e.item_id.clone()).collect()
    }

    fn source(&self) -> Option<&Json> {
        self.raw.get("source").filter(|source| source.is_object())
    }
}

/// the validated skin catalog, in file order (standard_cdo_paths takes the first
/// match, 07 open question 11).
#[derive(Debug, Clone, Default)]
pub struct SkinCatalog {
    entries: Vec<CatalogEntry>,
    by_skin_id: HashMap<String, usize>,
    legacy_skin_ids: HashMap<String, String>,
    legacy_level_ids: HashMap<String, String>,
    legacy_chroma_ids: HashMap<String, String>,
}

impl SkinCatalog {
    pub fn parse(text: &str) -> Result<Self, SkinError> {
        Self::from_json(&Json::parse(text)?)
    }

    /// python SkinInventory._read_catalog (skin_inventory.py:46-130), same checks and messages.
    pub fn from_json(data: &Json) -> Result<Self, SkinError> {
        if !data.is_object() {
            return Err(SkinError::Other("skin catalog is not an object".into()));
        }
        let Some(items) = data.get("skins").and_then(Json::as_array).filter(|_| data.get("schema_version").is_some_and(Json::equals_one)) else {
            return Err(inventory_error("unsupported skin catalog schema"));
        };
        let mut catalog = Self::default();
        for item in items {
            if !item.is_object() {
                return Err(inventory_error("skin catalog entry is not an object"));
            }
            const REQUIRED: [&str; 4] = ["weapon_id", "skin_id", "skin_level_id", "chroma_id"];
            if REQUIRED.iter().any(|key| !valid_uuid_json(item.get(key))) {
                return Err(inventory_error("skin catalog entry has an invalid ID"));
            }
            let id = |key: &str| item.get(key).and_then(Json::as_str).expect("validated above").to_string();
            let Some(name) = item.get("name").and_then(Json::as_str).filter(|name| !name.is_empty()) else {
                return Err(inventory_error("skin catalog entry needs a name"));
            };
            let raw_entitlements = match item.get("entitlements") {
                None => Some(&EMPTY),
                Some(value) => value.as_array(),
            };
            let entitlements: Option<Vec<Entitlement>> = raw_entitlements.and_then(|values| {
                values
                    .iter()
                    .map(|value| {
                        let (type_id, item_id) = (value.get("type_id"), value.get("item_id"));
                        (value.is_object() && valid_uuid_json(type_id) && valid_uuid_json(item_id)).then(|| Entitlement {
                            type_id: lower(type_id.and_then(Json::as_str).expect("validated")),
                            item_id: lower(item_id.and_then(Json::as_str).expect("validated")),
                        })
                    })
                    .collect()
            });
            let Some(entitlements) = entitlements else {
                return Err(inventory_error("skin catalog entitlements are invalid"));
            };
            let mut default_types = Vec::with_capacity(3);
            for key in &REQUIRED[1..] {
                let default_id = lower(&id(key));
                let types: HashSet<&str> = entitlements.iter().filter(|e| e.item_id == default_id).map(|e| e.type_id.as_str()).collect();
                if types.len() != 1 {
                    return Err(inventory_error("skin catalog default needs one entitlement type"));
                }
                default_types.push(types.into_iter().next().expect("one type").to_string());
            }
            if default_types.iter().collect::<HashSet<_>>().len() != default_types.len() {
                return Err(inventory_error("skin catalog entitlement categories overlap"));
            }
            let skin_id = lower(&id("skin_id"));
            if catalog.by_skin_id.contains_key(&skin_id) {
                return Err(inventory_error(format!("duplicate skin ID: {skin_id}")));
            }
            let legacy_skin_ids = match item.get("legacy_skin_ids") {
                None => Some(&EMPTY),
                Some(value) => value.as_array(),
            };
            let Some(legacy_skin_ids) = legacy_skin_ids.filter(|ids| ids.iter().all(|value| valid_uuid_json(Some(value)))) else {
                return Err(inventory_error("invalid legacy skin IDs"));
            };
            for legacy_id in legacy_skin_ids {
                let legacy_id = lower(legacy_id.as_str().expect("validated"));
                if legacy_id == skin_id || catalog.legacy_skin_ids.contains_key(&legacy_id) {
                    return Err(inventory_error("duplicate legacy skin ID"));
                }
                catalog.legacy_skin_ids.insert(legacy_id, skin_id.clone());
            }
            for (key, type_id, is_level) in [("legacy_level_ids", &default_types[1], true), ("legacy_chroma_ids", &default_types[2], false)] {
                let empty = Json::Object(Vec::new());
                let aliases = item.get(key).unwrap_or(&empty);
                let Some(aliases) = aliases.as_object().filter(|pairs| pairs.iter().all(|(old_id, new_id)| valid_uuid(old_id) && valid_uuid_json(Some(new_id)))) else {
                    return Err(inventory_error(format!("invalid {key}")));
                };
                let valid_ids: HashSet<&str> = entitlements.iter().filter(|e| e.type_id == *type_id).map(|e| e.item_id.as_str()).collect();
                let destination = if is_level { &mut catalog.legacy_level_ids } else { &mut catalog.legacy_chroma_ids };
                for (old_id, new_id) in aliases {
                    let (old_id, new_id) = (lower(old_id), lower(new_id.as_str().expect("validated")));
                    if old_id == new_id || !valid_ids.contains(new_id.as_str()) || destination.get(&old_id).is_some_and(|mapped| *mapped != new_id) {
                        return Err(inventory_error(format!("invalid {key} mapping")));
                    }
                    destination.insert(old_id, new_id);
                }
            }
            catalog.by_skin_id.insert(skin_id, catalog.entries.len());
            catalog.entries.push(CatalogEntry {
                name: name.to_string(),
                weapon_id: id("weapon_id"),
                skin_id: id("skin_id"),
                skin_level_id: id("skin_level_id"),
                chroma_id: id("chroma_id"),
                entitlements,
                raw: item.clone(),
            });
        }
        if catalog.legacy_skin_ids.keys().any(|legacy_id| catalog.by_skin_id.contains_key(legacy_id)) {
            return Err(inventory_error("legacy skin ID collides with catalog"));
        }
        for is_level in [true, false] {
            let current: HashSet<String> = catalog
                .entries
                .iter()
                .flat_map(|entry| entry.granted_component_ids(if is_level { &entry.skin_level_id } else { &entry.chroma_id }))
                .collect();
            let destination = if is_level { &catalog.legacy_level_ids } else { &catalog.legacy_chroma_ids };
            if destination.keys().any(|legacy_id| current.contains(legacy_id)) {
                return Err(inventory_error("legacy component ID collides with catalog"));
            }
        }
        Ok(catalog)
    }

    /// entries in file order.
    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    /// catalog[skin_id] (lowercase id).
    pub fn get(&self, skin_id: &str) -> Option<&CatalogEntry> {
        self.by_skin_id.get(skin_id).map(|&index| &self.entries[index])
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ---------------------------------------------------------------- saved state

/// skins owned and equipped by the offline account (skin_inventory.SkinInventory, read path).
#[derive(Debug, Clone)]
pub struct SkinInventory {
    pub player_id: String,
    pub catalog: Arc<SkinCatalog>,
    /// lowercase skin ids
    pub owned: HashSet<String>,
    /// lowercase weapon id -> lowercase skin id, in file order
    pub equipped: Vec<(String, String)>,
    /// the saved loadout (after an in-memory migration, if one was due)
    pub loadout: Option<Json>,
    pub version: i128,
    /// python would have rewritten offline_skin_state.json here (legacy ids or
    /// non-lowercase ids mapped); the platform backend owns that write
    pub migration_pending: bool,
}

impl SkinInventory {
    /// python SkinInventory(player_id, catalog_path, state_path): the catalog
    /// must exist, a missing state file means nothing owned.
    pub fn load(player_id: &str, catalog_path: &Path, state_path: Option<&Path>) -> Result<Self, SkinError> {
        if !catalog_path.is_file() {
            return Err(inventory_error(format!("skin catalog is missing: {}", catalog_path.display())));
        }
        let catalog = Arc::new(SkinCatalog::parse(&read_text(catalog_path)?)?);
        let state = match state_path.filter(|path| path.is_file()) {
            Some(path) => Some(Json::parse(&read_text(path)?)?),
            None => None,
        };
        Self::from_documents(player_id, catalog, state.as_ref())
    }

    /// python _read_state (skin_inventory.py:132-198) over an already parsed
    /// catalog and state document (None: no state file).
    pub fn from_documents(player_id: &str, catalog: Arc<SkinCatalog>, state: Option<&Json>) -> Result<Self, SkinError> {
        let mut inventory = Self { player_id: player_id.to_string(), catalog, owned: HashSet::new(), equipped: Vec::new(), loadout: None, version: 1, migration_pending: false };
        if let Some(data) = state {
            inventory.read_state(data)?;
        }
        Ok(inventory)
    }

    fn read_state(&mut self, data: &Json) -> Result<(), SkinError> {
        if !data.is_object() {
            return Err(SkinError::Other("skin state is not an object".into()));
        }
        if !data.get("schema_version").is_some_and(Json::equals_one) || data.get("player_id").and_then(Json::as_str) != Some(self.player_id.as_str()) {
            return Err(inventory_error("skin state does not match the local account"));
        }
        let empty_list = Json::Array(Vec::new());
        let empty_dict = Json::Object(Vec::new());
        let (Some(owned), Some(equipped)) = (data.get("owned").unwrap_or(&empty_list).as_array(), data.get("equipped").unwrap_or(&empty_dict).as_object()) else {
            return Err(inventory_error("skin state is malformed"));
        };
        let owned: Option<Vec<&str>> = owned.iter().map(Json::as_str).collect();
        let equipped: Option<Vec<(&str, &str)>> = equipped.iter().map(|(key, value)| value.as_str().map(|value| (key.as_str(), value))).collect();
        let (Some(owned), Some(equipped)) = (owned, equipped) else {
            return Err(inventory_error("skin state is malformed"));
        };
        let catalog = Arc::clone(&self.catalog);
        let map_skin = |value: &str| {
            let value = lower(value);
            catalog.legacy_skin_ids.get(&value).cloned().unwrap_or(value)
        };
        let mapped_owned: Vec<String> = owned.iter().map(|value| map_skin(value)).collect();
        let mapped_equipped: Vec<(&str, String)> = equipped.iter().map(|&(weapon_id, skin_id)| (weapon_id, map_skin(skin_id))).collect();
        let mut loadout = data.get("loadout").filter(|value| !value.is_null()).cloned();
        let mut loadout_mapped = false;
        if let Some(guns) = loadout.as_mut().filter(|loadout| loadout.is_object()).and_then(loadout_guns_mut) {
            for gun in guns.iter_mut().filter(|gun| gun.is_object()) {
                for (keys, aliases) in [
                    (["skinId", "SkinID"], &catalog.legacy_skin_ids),
                    (["skinLevelId", "SkinLevelID"], &catalog.legacy_level_ids),
                    (["chromaId", "ChromaID"], &catalog.legacy_chroma_ids),
                ] {
                    for key in keys {
                        if let Some(mapped) = gun.get(key).and_then(Json::as_str).and_then(|value| aliases.get(&lower(value))) {
                            gun.set(key, Json::Str(mapped.clone()));
                            loadout_mapped = true;
                        }
                    }
                }
            }
        }
        let migrated = mapped_owned.iter().zip(&owned).any(|(mapped, raw)| mapped != raw)
            || mapped_equipped.iter().zip(&equipped).any(|((_, mapped), (_, raw))| mapped != raw)
            || loadout_mapped;
        if mapped_owned.iter().any(|skin_id| catalog.get(skin_id).is_none()) {
            return Err(inventory_error("skin state refers to an unknown catalog skin"));
        }
        let invalid_equipped = mapped_equipped.iter().any(|(weapon_id, skin_id)| {
            !valid_uuid(weapon_id) || catalog.get(skin_id).is_none_or(|entry| lower(&entry.weapon_id) != lower(weapon_id)) || !mapped_owned.contains(skin_id)
        });
        if invalid_equipped {
            return Err(inventory_error("skin state has an invalid equipped entry"));
        }
        if loadout.as_ref().is_some_and(|loadout| !valid_loadout(loadout)) {
            return Err(inventory_error("saved player loadout is malformed"));
        }
        self.owned = mapped_owned.into_iter().collect();
        self.equipped.clear();
        for (weapon_id, skin_id) in mapped_equipped {
            let weapon_id = lower(weapon_id);
            match self.equipped.iter_mut().find(|(key, _)| *key == weapon_id) {
                Some(entry) => entry.1 = skin_id,
                None => self.equipped.push((weapon_id, skin_id)),
            }
        }
        self.loadout = loadout;
        self.version = py_int_value(data.get("version").unwrap_or(&Json::Int(1)))?.max(1);
        if migrated {
            if let Some(loadout) = self.loadout.as_mut() {
                apply_equipped(&self.catalog, &self.equipped, loadout);
            }
            // python ints never overflow; only a >38-digit version saturates here
            self.version = self.version.saturating_add(1);
            if let Some(loadout) = self.loadout.as_mut() {
                let key = if loadout.contains_key("version") { "version" } else { "Version" };
                loadout.set(key, Json::Int(self.version));
            }
            // python _save() would write the state file now; the platform owns that
            self.migration_pending = true;
        }
        Ok(())
    }

    /// equipped.get(weapon_id) with an already-lowercased weapon id.
    pub fn equipped_skin(&self, weapon_id: &str) -> Option<&str> {
        self.equipped.iter().find(|(key, _)| key == weapon_id).map(|(_, skin_id)| skin_id.as_str())
    }
}

fn read_text(path: &Path) -> Result<String, SkinError> {
    std::fs::read_to_string(path).map_err(|error| SkinError::Other(format!("{}: {error}", path.display())))
}

/// python SkinInventory._apply_equipped (skin_inventory.py:240-269): force each
/// equipped skin into its gun entry, resetting level / chroma to the catalog
/// defaults unless the saved ones are granted.
fn apply_equipped(catalog: &SkinCatalog, equipped: &[(String, String)], loadout: &mut Json) {
    let Some(guns) = loadout_guns_mut(loadout) else {
        return;
    };
    for gun in guns.iter_mut() {
        let Some(weapon_id) = gun_weapon_id(gun).map(lower) else {
            continue;
        };
        let Some(skin_id) = equipped.iter().find(|(key, _)| *key == weapon_id).map(|(_, skin)| skin.as_str()).filter(|skin| !skin.is_empty()) else {
            continue;
        };
        let entry = catalog.get(skin_id).expect("equipped skins are validated against the catalog");
        let keys = if gun.contains_key("SkinID") { ["SkinID", "SkinLevelID", "ChromaID"] } else { ["skinId", "skinLevelId", "chromaId"] };
        let selected = |key: &str| gun.get(key).and_then(Json::as_str).map(lower);
        let keep = selected(keys[0]).is_some_and(|skin| skin == skin_id)
            && selected(keys[1]).is_some_and(|level| entry.granted_component_ids(&entry.skin_level_id).contains(&level))
            && selected(keys[2]).is_some_and(|chroma| entry.granted_component_ids(&entry.chroma_id).contains(&chroma));
        if keep {
            continue;
        }
        gun.set(keys[0], Json::Str(skin_id.to_string()));
        gun.set(keys[1], Json::Str(entry.skin_level_id.clone()));
        gun.set(keys[2], Json::Str(entry.chroma_id.clone()));
    }
}

// ---------------------------------------------------------------- equipped cosmetic resolution

/// a saved owned/equipped selection (equipped_cosmetic.py:14-24).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EquippedCosmetic {
    pub weapon_id: String,
    pub skin_id: String,
    pub level_id: String,
    pub chroma_id: String,
    pub name: String,
    pub weapon_asset: String,
    pub skin_asset: String,
    pub level_asset: String,
    pub chroma_asset: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CosmeticKind {
    Skin,
    Level,
    Chroma,
}

impl CosmeticKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CosmeticKind::Skin => "skin",
            CosmeticKind::Level => "level",
            CosmeticKind::Chroma => "chroma",
        }
    }

    /// python's kind strings; anything else is "unknown cosmetic kind".
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "skin" => Some(CosmeticKind::Skin),
            "level" => Some(CosmeticKind::Level),
            "chroma" => Some(CosmeticKind::Chroma),
            _ => None,
        }
    }
}

/// one selected catalog asset's package and cdo (equipped_cosmetic.py:27-34).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CosmeticCdoPath {
    pub kind: CosmeticKind,
    pub asset_id: String,
    pub source_asset: String,
    pub package_path: String,
    pub object_name: String,
    /// false only for the one catalog level marked level_asset_packaged: false
    pub packaged: bool,
}

/// equipped_cosmetic.py:37-42.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EquippedCdoPaths {
    pub weapon_id: String,
    pub skin: CosmeticCdoPath,
    pub level: CosmeticCdoPath,
    pub chroma: CosmeticCdoPath,
}

impl EquippedCdoPaths {
    pub fn all_packaged(&self) -> bool {
        self.skin.packaged && self.level.packaged && self.chroma.packaged
    }

    /// skin, level, chroma with their kinds, in wire order.
    pub fn in_order(&self) -> [(CosmeticKind, &CosmeticCdoPath); 3] {
        [(CosmeticKind::Skin, &self.skin), (CosmeticKind::Level, &self.level), (CosmeticKind::Chroma, &self.chroma)]
    }
}

/// python _asset_path (equipped_cosmetic.py:48-60): a relative Equippables/
/// .uasset path, judged the way PurePosixPath sees it.
fn asset_path(value: Option<&Json>) -> Result<String, SkinError> {
    let Some(value) = value.and_then(Json::as_str) else {
        return Err(inventory_error("cosmetic catalog asset path is missing"));
    };
    // PurePosixPath drops empty and "." components; suffix comes from the last part
    let parts: Vec<&str> = value.split('/').filter(|part| !part.is_empty() && *part != ".").collect();
    let name = parts.last().copied().unwrap_or("");
    let suffix = match name.rfind('.') {
        Some(at) if at > 0 && at < name.len() - 1 => &name[at..],
        _ => "",
    };
    if value.starts_with('/') || !value.starts_with("Equippables/") || suffix != ".uasset" || parts.contains(&"..") || value.contains('\\') {
        return Err(inventory_error("cosmetic catalog asset path is invalid"));
    }
    Ok(value.to_string())
}

/// python _field (equipped_cosmetic.py:63-71): every present spelling must be a
/// valid uuid and they must agree; returns it lowercased.
fn saved_field(gun: &Json, lower_name: &str, upper_name: &str) -> Result<String, SkinError> {
    let values: Vec<&Json> = [lower_name, upper_name].iter().filter_map(|key| gun.get(key)).collect();
    if values.is_empty() {
        return Err(inventory_error(format!("saved cosmetic field is missing: {lower_name}")));
    }
    let Some(values) = values.iter().map(|value| value.as_str().filter(|text| valid_uuid(text))).collect::<Option<Vec<&str>>>() else {
        return Err(inventory_error(format!("saved cosmetic field is invalid: {lower_name}")));
    };
    if values.iter().map(|value| lower(value)).collect::<HashSet<_>>().len() != 1 {
        return Err(inventory_error(format!("saved cosmetic field conflicts: {lower_name}")));
    }
    Ok(lower(values[0]))
}

/// python _component_asset (equipped_cosmetic.py:74-106): the extractor records
/// entitlement ids and unlock asset paths in the same order; both must agree
/// with the default before a selected level / chroma resolves.
fn component_asset(entry: &CatalogEntry, component_id: &str, kind: CosmeticKind) -> Result<String, SkinError> {
    let (default_id, group_name, default_asset_key) = match kind {
        CosmeticKind::Level => (&entry.skin_level_id, "levels", "level_asset"),
        _ => (&entry.chroma_id, "chromas", "chroma_asset"),
    };
    let default_id = lower(default_id);
    let types: HashSet<&str> = entry.entitlements.iter().filter(|e| e.item_id == default_id).map(|e| e.type_id.as_str()).collect();
    if types.len() != 1 {
        return Err(inventory_error("catalog default component type is ambiguous"));
    }
    let type_id = types.into_iter().next().expect("one type");
    let component_ids: Vec<&str> = entry.entitlements.iter().filter(|e| e.type_id == type_id).map(|e| e.item_id.as_str()).collect();
    let Some(source) = entry.source() else {
        return Err(inventory_error("cosmetic catalog source is missing"));
    };
    let Some(unlock_assets) = source.get("unlock_assets").filter(|value| value.is_object()) else {
        return Err(inventory_error("cosmetic catalog unlock paths are missing"));
    };
    let asset_paths = unlock_assets.get(group_name).and_then(Json::as_array);
    let unique = component_ids.iter().collect::<HashSet<_>>().len() == component_ids.len();
    let Some(asset_paths) = asset_paths.filter(|paths| paths.len() == component_ids.len() && unique) else {
        return Err(inventory_error("cosmetic catalog component mapping is ambiguous"));
    };
    let mapped: Vec<(&str, String)> = component_ids.iter().zip(asset_paths).map(|(id, path)| asset_path(Some(path)).map(|path| (*id, path))).collect::<Result<_, _>>()?;
    let lookup = |id: &str| mapped.iter().find(|(key, _)| *key == id).map(|(_, path)| path.as_str());
    let default_path = lookup(&default_id);
    if default_path != Some(asset_path(source.get(default_asset_key))?.as_str()) {
        return Err(inventory_error("cosmetic catalog default asset does not match unlock order"));
    }
    lookup(component_id).map(str::to_string).ok_or_else(|| inventory_error("saved cosmetic component is not granted"))
}

/// python resolve_equipped_cosmetic (equipped_cosmetic.py:109-165): the saved
/// owned/equipped selection, or None when no skin is equipped for the weapon.
pub fn resolve_equipped_cosmetic(inventory: &SkinInventory, weapon_id: &str) -> Result<Option<EquippedCosmetic>, SkinError> {
    if !valid_uuid(weapon_id) {
        return Err(inventory_error("weapon ID is invalid"));
    }
    let weapon_id = lower(weapon_id);
    let Some(skin_id) = inventory.equipped_skin(&weapon_id) else {
        return Ok(None);
    };
    let entry = inventory.catalog.get(skin_id).filter(|entry| lower(&entry.weapon_id) == weapon_id && inventory.owned.contains(skin_id));
    let Some(entry) = entry else {
        return Err(inventory_error("skin is not owned and equipped for weapon"));
    };
    let Some(loadout) = inventory.loadout.as_ref().filter(|loadout| loadout.is_object()) else {
        return Err(inventory_error("saved player loadout is missing"));
    };
    let guns = loadout_guns(loadout).map_or(&[][..], Vec::as_slice);
    let matches: Vec<&Json> = guns.iter().filter(|gun| gun.is_object() && gun_weapon_id(gun).is_some_and(|id| lower(id) == weapon_id)).collect();
    if matches.len() != 1 {
        return Err(inventory_error("expected one saved loadout entry for equipped weapon"));
    }
    let gun = matches[0];
    if saved_field(gun, "skinId", "SkinID")? != skin_id {
        return Err(inventory_error("saved skin differs from equipped skin"));
    }
    let level_id = saved_field(gun, "skinLevelId", "SkinLevelID")?;
    let chroma_id = saved_field(gun, "chromaId", "ChromaID")?;
    let Some(source) = entry.source() else {
        return Err(inventory_error("cosmetic catalog source is missing"));
    };
    Ok(Some(EquippedCosmetic {
        weapon_asset: asset_path(source.get("weapon_asset"))?,
        skin_asset: asset_path(source.get("skin_asset"))?,
        level_asset: component_asset(entry, &level_id, CosmeticKind::Level)?,
        chroma_asset: component_asset(entry, &chroma_id, CosmeticKind::Chroma)?,
        weapon_id,
        skin_id: skin_id.to_string(),
        level_id,
        chroma_id,
        name: entry.name.clone(),
    }))
}

/// python catalog_cdo_path (equipped_cosmetic.py:168-212): a selected catalog
/// uuid to its exact asset's package and cdo. the catalog picks the asset; only
/// the unreal package / cdo spelling comes from its filename.
pub fn catalog_cdo_path(entry: &CatalogEntry, kind: CosmeticKind, asset_id: &str) -> Result<CosmeticCdoPath, SkinError> {
    let Some(source) = entry.source() else {
        return Err(inventory_error("cosmetic catalog source is missing"));
    };
    if !valid_uuid(asset_id) {
        return Err(inventory_error("cosmetic asset ID is invalid"));
    }
    let asset_id = lower(asset_id);
    let source_asset = match kind {
        CosmeticKind::Skin => {
            if entry.raw.get("skin_id").and_then(Json::as_str).is_none_or(|skin_id| asset_id != lower(skin_id)) {
                return Err(inventory_error("skin ID is not in catalog entry"));
            }
            asset_path(source.get("skin_asset"))?
        }
        _ => component_asset(entry, &asset_id, kind)?,
    };
    let stem = source_asset.strip_suffix(".uasset").unwrap_or(&source_asset);
    let parts: Vec<&str> = stem.split('/').collect();
    let component_ok = |part: &&str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if !parts.iter().all(component_ok) {
        return Err(inventory_error("cosmetic asset path has an invalid component"));
    }
    let packaged = !(kind == CosmeticKind::Level
        && source.get("level_asset_packaged") == Some(&Json::Bool(false))
        && source.get("level_asset").and_then(Json::as_str) == Some(source_asset.as_str()));
    Ok(CosmeticCdoPath {
        kind,
        asset_id,
        package_path: format!("/Game/{}", parts.join("/")),
        object_name: format!("Default__{}_C", parts.last().expect("split yields one part")),
        source_asset,
        packaged,
    })
}

/// python resolve_equipped_cdo_paths (equipped_cosmetic.py:215-226).
pub fn resolve_equipped_cdo_paths(inventory: &SkinInventory, weapon_id: &str) -> Result<Option<EquippedCdoPaths>, SkinError> {
    let Some(cosmetic) = resolve_equipped_cosmetic(inventory, weapon_id)? else {
        return Ok(None);
    };
    let entry = inventory.catalog.get(&cosmetic.skin_id).expect("resolved from the catalog");
    Ok(Some(EquippedCdoPaths {
        skin: catalog_cdo_path(entry, CosmeticKind::Skin, &cosmetic.skin_id)?,
        level: catalog_cdo_path(entry, CosmeticKind::Level, &cosmetic.level_id)?,
        chroma: catalog_cdo_path(entry, CosmeticKind::Chroma, &cosmetic.chroma_id)?,
        weapon_id: cosmetic.weapon_id,
    }))
}

/// python select_base_pistol_cosmetic_loadout (range_udp_handshake.py:96-115)
/// on an inventory: (selection, its cdo paths), or None when unskinned. a21
/// reads a fresh inventory (no cache) so collection changes since launch apply.
pub fn select_cosmetic_loadout(inventory: &SkinInventory, weapon_id: &str) -> Result<Option<(EquippedCosmetic, EquippedCdoPaths)>, SkinError> {
    let Some(cosmetic) = resolve_equipped_cosmetic(inventory, weapon_id)? else {
        return Ok(None);
    };
    let paths = resolve_equipped_cdo_paths(inventory, weapon_id)?.expect("resolved above");
    Ok(Some((cosmetic, paths)))
}

// ---------------------------------------------------------------- cache

/// python cached_skin_inventory (range_gun_loader.py:248-273): one inventory
/// reused until the state file's modification time changes (python rebuilt it
/// per purchase at first and stalled the responder ~4 s).
#[derive(Debug)]
pub struct InventoryCache {
    player_id: String,
    catalog_path: PathBuf,
    state_path: PathBuf,
    cached: Option<(Option<SystemTime>, Arc<SkinInventory>)>,
}

impl InventoryCache {
    pub fn new(player_id: &str, catalog_path: PathBuf, state_path: PathBuf) -> Self {
        Self { player_id: player_id.to_string(), catalog_path, state_path, cached: None }
    }

    /// skin_catalog.json and offline_skin_state.json in the backend dir.
    pub fn in_dir(player_id: &str, dir: &Path) -> Self {
        Self::new(player_id, dir.join(SKIN_CATALOG_FILE), dir.join(SKIN_STATE_FILE))
    }

    pub fn get(&mut self) -> Result<Arc<SkinInventory>, SkinError> {
        let stamp = std::fs::metadata(&self.state_path).and_then(|meta| meta.modified()).ok();
        if let Some((cached_stamp, inventory)) = &self.cached
            && *cached_stamp == stamp
        {
            return Ok(Arc::clone(inventory));
        }
        let inventory = Arc::new(SkinInventory::load(&self.player_id, &self.catalog_path, Some(&self.state_path))?);
        if inventory.migration_pending {
            log::warn!("{} has legacy skin ids; resolved in memory, not rewritten (the platform backend owns it)", self.state_path.display());
        }
        log::debug!("skin inventory loaded: {} skins, {} owned, {} equipped", inventory.catalog.len(), inventory.owned.len(), inventory.equipped.len());
        self.cached = Some((stamp, Arc::clone(&inventory)));
        Ok(inventory)
    }
}
