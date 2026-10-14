use crate::player_id;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
};
use uuid::Uuid;

const permanent_type: &str = "4e60e748-bce6-4faa-9327-ebbe6089d5fe";

#[derive(Clone)]
pub struct Inventory {
    pub catalog: BTreeMap<String, Value>,
    pub owned: BTreeSet<String>,
    pub equipped: BTreeMap<String, String>,
    pub loadout: Option<Value>,
    pub version: u64,
    state_path: Option<PathBuf>,
}

pub fn string(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}
pub fn field<'a>(value: &'a Value, lower: &str, upper: &str) -> &'a Value {
    value
        .get(lower)
        .or_else(|| value.get(upper))
        .unwrap_or(&Value::Null)
}
fn valid_uuid(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|s| Uuid::parse_str(s).is_ok_and(|id| id.to_string() == s.to_lowercase()))
}
fn components(entry: &Value, key: &str) -> BTreeSet<String> {
    let items = entry["entitlements"].as_array().expect("validated catalog");
    let default_id = string(&entry[key]).to_lowercase();
    let type_id = items
        .iter()
        .find(|item| string(&item["item_id"]).to_lowercase() == default_id)
        .map(|item| string(&item["type_id"]).to_lowercase())
        .unwrap_or_default();
    items
        .iter()
        .filter(|item| string(&item["type_id"]).to_lowercase() == type_id)
        .map(|item| string(&item["item_id"]).to_lowercase())
        .collect()
}

impl Inventory {
    pub fn open(
        catalog_path: Option<PathBuf>,
        state_path: Option<PathBuf>,
    ) -> Result<Self, String> {
        let mut result = Self {
            catalog: BTreeMap::new(),
            owned: BTreeSet::new(),
            equipped: BTreeMap::new(),
            loadout: None,
            version: 1,
            state_path,
        };
        let mut skin_aliases = BTreeMap::new();
        let mut level_aliases = BTreeMap::new();
        let mut chroma_aliases = BTreeMap::new();
        if let Some(path) = catalog_path {
            let data: Value = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            if data["schema_version"] != 1 {
                return Err("unsupported skin catalog schema".into());
            }
            for entry in data["skins"].as_array().ok_or("skin catalog needs skins")? {
                if !["weapon_id", "skin_id", "skin_level_id", "chroma_id"]
                    .iter()
                    .all(|key| valid_uuid(&entry[*key]))
                    || string(&entry["name"]).is_empty()
                {
                    return Err("skin catalog entry is invalid".into());
                }
                let items = entry["entitlements"]
                    .as_array()
                    .ok_or("skin catalog entitlements are invalid")?;
                if items
                    .iter()
                    .any(|item| !valid_uuid(&item["type_id"]) || !valid_uuid(&item["item_id"]))
                {
                    return Err("skin catalog entitlement ID is invalid".into());
                }
                let mut types = BTreeSet::new();
                for key in ["skin_id", "skin_level_id", "chroma_id"] {
                    let matching: BTreeSet<_> = items
                        .iter()
                        .filter(|item| {
                            string(&item["item_id"]).eq_ignore_ascii_case(string(&entry[key]))
                        })
                        .map(|item| string(&item["type_id"]).to_lowercase())
                        .collect();
                    if matching.len() != 1 || !types.insert(matching.into_iter().next().unwrap()) {
                        return Err(
                            "skin catalog entitlement categories overlap or are missing".into()
                        );
                    }
                }
                let skin_id = string(&entry["skin_id"]).to_lowercase();
                if result
                    .catalog
                    .insert(skin_id.clone(), entry.clone())
                    .is_some()
                {
                    return Err("duplicate skin ID".into());
                }
                if let Some(aliases) = entry.get("legacy_skin_ids") {
                    for alias in aliases.as_array().ok_or("invalid legacy skin IDs")? {
                        let alias_id = string(alias).to_lowercase();
                        if !valid_uuid(alias)
                            || alias_id == skin_id
                            || skin_aliases.insert(alias_id, skin_id.clone()).is_some()
                        {
                            return Err("invalid legacy skin ID".into());
                        }
                    }
                }
                for (key, default_key, destination) in [
                    ("legacy_level_ids", "skin_level_id", &mut level_aliases),
                    ("legacy_chroma_ids", "chroma_id", &mut chroma_aliases),
                ] {
                    if let Some(aliases) = entry.get(key) {
                        for (old, new) in
                            aliases.as_object().ok_or("invalid legacy component IDs")?
                        {
                            let new_id = string(new).to_lowercase();
                            if !valid_uuid(&json!(old))
                                || !valid_uuid(new)
                                || old.eq_ignore_ascii_case(&new_id)
                                || !components(entry, default_key).contains(&new_id)
                            {
                                return Err("invalid legacy component ID".into());
                            }
                            if destination
                                .insert(old.to_lowercase(), new_id.clone())
                                .is_some_and(|previous| previous != new_id)
                            {
                                return Err("conflicting legacy component ID".into());
                            }
                        }
                    }
                }
            }
            if skin_aliases
                .keys()
                .any(|id| result.catalog.contains_key(id))
            {
                return Err("legacy skin ID collides with catalog".into());
            }
            for (aliases, key) in [
                (&level_aliases, "skin_level_id"),
                (&chroma_aliases, "chroma_id"),
            ] {
                let current: BTreeSet<_> = result
                    .catalog
                    .values()
                    .flat_map(|entry| components(entry, key))
                    .collect();
                if aliases.keys().any(|id| current.contains(id)) {
                    return Err("legacy component ID collides with catalog".into());
                }
            }
        }
        if let Some(path) = &result.state_path
            && path.exists()
        {
            let data: Value = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            if data["schema_version"] != 1 || data["player_id"] != player_id {
                return Err("skin state does not match the local account".into());
            }
            for value in data["owned"].as_array().ok_or("skin state is malformed")? {
                let id = value
                    .as_str()
                    .ok_or("skin state is malformed")?
                    .to_lowercase();
                let id = skin_aliases.get(&id).unwrap_or(&id);
                if !result.catalog.contains_key(id) {
                    return Err("skin state refers to an unknown catalog skin".into());
                }
                result.owned.insert(id.clone());
            }
            for (weapon, value) in data["equipped"]
                .as_object()
                .ok_or("skin state is malformed")?
            {
                let id = value
                    .as_str()
                    .ok_or("skin state is malformed")?
                    .to_lowercase();
                let id = skin_aliases.get(&id).unwrap_or(&id);
                if !result.owned.contains(id)
                    || !string(&result.catalog[id]["weapon_id"]).eq_ignore_ascii_case(weapon)
                {
                    return Err("skin state has an invalid equipped entry".into());
                }
                result.equipped.insert(weapon.to_lowercase(), id.clone());
            }
            result.version = data["version"]
                .as_u64()
                .ok_or("invalid skin state version")?
                .max(1);
            if !data["loadout"].is_null() {
                let mut loadout = data["loadout"].clone();
                Self::validate_loadout(&loadout)?;
                let guns_key = if loadout.get("guns").is_some() {
                    "guns"
                } else {
                    "Guns"
                };
                if let Some(guns) = loadout[guns_key].as_array_mut() {
                    for gun in guns {
                        for (keys, aliases) in [
                            (["skinId", "SkinID"], &skin_aliases),
                            (["skinLevelId", "SkinLevelID"], &level_aliases),
                            (["chromaId", "ChromaID"], &chroma_aliases),
                        ] {
                            for key in keys {
                                if let Some(id) = aliases.get(&string(&gun[key]).to_lowercase()) {
                                    gun[key] = json!(id);
                                }
                            }
                        }
                    }
                }
                result.loadout = Some(loadout);
            }
            let migrated = data["owned"] != json!(result.owned)
                || data["equipped"] != json!(result.equipped)
                || data["loadout"] != json!(result.loadout);
            if migrated {
                result.version = result.version.saturating_add(1);
                if let Some(mut loadout) = result.loadout.take() {
                    result.apply_equipped(&mut loadout);
                    let key = if loadout.get("version").is_some() {
                        "version"
                    } else {
                        "Version"
                    };
                    loadout[key] = json!(result.version);
                    result.loadout = Some(loadout);
                }
                result.save()?;
            }
        }
        Ok(result)
    }

    fn validate_loadout(value: &Value) -> Result<(), String> {
        if !value.is_object() {
            return Err("invalid_player_loadout".into());
        }
        let guns = field(value, "guns", "Guns");
        if guns.is_null() && value.get("guns").is_none() && value.get("Guns").is_none() {
            return Ok(());
        }
        if !guns
            .as_array()
            .is_some_and(|items| items.len() <= 128 && items.iter().all(Value::is_object))
        {
            return Err("invalid_player_loadout".into());
        }
        Ok(())
    }

    fn apply_equipped(&self, loadout: &mut Value) {
        let guns_key = if loadout.get("guns").is_some() {
            "guns"
        } else {
            "Guns"
        };
        if let Some(guns) = loadout[guns_key].as_array_mut() {
            for gun in guns {
                let weapon = string(field(gun, "iD", "ID")).to_lowercase();
                let Some(skin_id) = self.equipped.get(&weapon) else {
                    continue;
                };
                let entry = &self.catalog[skin_id];
                let keys = if gun.get("SkinID").is_some() {
                    ["SkinID", "SkinLevelID", "ChromaID"]
                } else {
                    ["skinId", "skinLevelId", "chromaId"]
                };
                if string(&gun[keys[0]]).eq_ignore_ascii_case(skin_id)
                    && components(entry, "skin_level_id")
                        .contains(&string(&gun[keys[1]]).to_lowercase())
                    && components(entry, "chroma_id")
                        .contains(&string(&gun[keys[2]]).to_lowercase())
                {
                    continue;
                }
                for (key, source) in keys
                    .into_iter()
                    .zip(["skin_id", "skin_level_id", "chroma_id"])
                {
                    gun[key] = entry[source].clone();
                }
            }
        }
    }

    pub fn get_loadout(&self) -> Value {
        let mut loadout = self.loadout.clone().unwrap_or_else(|| json!({"Subject":player_id,"Version":self.version,"Guns":[],"Sprays":[],"Identity":{}}));
        self.apply_equipped(&mut loadout);
        loadout
    }

    fn save(&self) -> Result<(), String> {
        if let Some(path) = &self.state_path {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let temporary = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
            let data = json!({"schema_version":1,"player_id":player_id,"version":self.version,"owned":self.owned,"equipped":self.equipped,"loadout":self.loadout});
            fs::write(&temporary, serde_json::to_vec_pretty(&data).unwrap())
                .map_err(|e| e.to_string())?;
            if let Err(error) = fs::rename(&temporary, path) {
                let _ = fs::remove_file(temporary);
                return Err(error.to_string());
            }
        }
        Ok(())
    }

    pub fn put_loadout(&mut self, request: Value) -> Result<Value, String> {
        Self::validate_loadout(&request)?;
        let mut next = self.clone();
        if let Some(guns) = field(&request, "guns", "Guns").as_array() {
            for gun in guns {
                let Some(weapon) = field(gun, "iD", "ID").as_str() else {
                    continue;
                };
                let weapon = weapon.to_lowercase();
                let skin_id = string(field(gun, "skinId", "SkinID")).to_lowercase();
                let Some(entry) = self.catalog.get(&skin_id) else {
                    next.equipped.remove(&weapon);
                    continue;
                };
                if !self.owned.contains(&skin_id) {
                    return Err("skin_not_owned".into());
                }
                if !string(&entry["weapon_id"]).eq_ignore_ascii_case(&weapon) {
                    return Err("skin_does_not_match_weapon".into());
                }
                if !components(entry, "skin_level_id")
                    .contains(&string(field(gun, "skinLevelId", "SkinLevelID")).to_lowercase())
                    || !components(entry, "chroma_id")
                        .contains(&string(field(gun, "chromaId", "ChromaID")).to_lowercase())
                {
                    return Err("skin_level_or_chroma_not_granted".into());
                }
                next.equipped.insert(weapon, skin_id);
            }
        }
        let requested_version = request
            .get("version")
            .map(|v| v.as_i64().ok_or("invalid version"))
            .transpose()?
            .unwrap_or(0);
        next.version = self
            .version
            .saturating_add(u64::from(self.loadout.is_some()))
            .max(requested_version.saturating_add(1).max(1) as u64);
        let mut loadout = request;
        loadout["subject"] = json!(player_id);
        loadout["version"] = json!(next.version);
        next.apply_equipped(&mut loadout);
        next.loadout = Some(loadout.clone());
        next.save()?;
        *self = next;
        Ok(loadout)
    }

    pub fn snapshot(&self) -> Value {
        let mut available: Vec<_> = self.catalog.iter().map(|(id, item)| json!({"weapon_id":item["weapon_id"],"skin_id":id,"name":item["name"],"owned":self.owned.contains(id)})).collect();
        available.sort_by(|a, b| string(&a["name"]).cmp(string(&b["name"])));
        json!({"version":self.version,"available":available,"equipped":self.equipped.iter().map(|(w,s)| json!({"weapon_id":w,"skin_id":s})).collect::<Vec<_>>()})
    }

    pub fn grant(&mut self, skin_id: Option<&str>) -> Result<Value, String> {
        let mut next = self.clone();
        if let Some(id) = skin_id {
            let id = id.to_lowercase();
            if !self.catalog.contains_key(&id) {
                return Err("unknown_skin_id".into());
            }
            next.owned.insert(id);
        } else {
            next.owned.extend(self.catalog.keys().cloned());
        }
        if next.owned != self.owned {
            next.version += 1;
            next.save()?;
            *self = next;
        }
        Ok(self.snapshot())
    }

    pub fn equip(&mut self, weapon: &str, skin: &str) -> Result<Value, String> {
        let weapon = weapon.to_lowercase();
        let skin = skin.to_lowercase();
        if !self
            .catalog
            .get(&skin)
            .is_some_and(|entry| string(&entry["weapon_id"]).eq_ignore_ascii_case(&weapon))
        {
            return Err("skin_does_not_match_weapon".into());
        }
        if !self.owned.contains(&skin) {
            return Err("skin_not_owned".into());
        }
        if !self
            .loadout
            .as_ref()
            .and_then(|v| field(v, "guns", "Guns").as_array())
            .is_some_and(|guns| {
                guns.iter()
                    .any(|gun| string(field(gun, "iD", "ID")).eq_ignore_ascii_case(&weapon))
            })
        {
            return Err("weapon_not_in_player_loadout".into());
        }
        let mut next = self.clone();
        next.equipped.insert(weapon, skin);
        next.version += 1;
        let mut loadout = next.loadout.take().unwrap();
        next.apply_equipped(&mut loadout);
        loadout["version"] = json!(next.version);
        next.loadout = Some(loadout);
        next.save()?;
        *self = next;
        Ok(self.snapshot())
    }

    pub fn entitlements(&self) -> indexmap::IndexMap<String, Vec<Value>> {
        // Python dicts preserve first-insertion order, including deduplicated
        // IDs across skins. Preserve that wire ordering as well as the values.
        let mut groups: indexmap::IndexMap<String, indexmap::IndexMap<String, Value>> =
            indexmap::IndexMap::new();
        for id in &self.owned {
            for item in self.catalog[id]["entitlements"].as_array().unwrap() {
                let type_id = string(&item["type_id"]).to_lowercase();
                let item_id = string(&item["item_id"]).to_lowercase();
                let instance = Uuid::new_v5(
                    &Uuid::NAMESPACE_URL,
                    format!("offline:{player_id}:{type_id}:{item_id}").as_bytes(),
                );
                groups.entry(type_id).or_default().insert(item_id.clone(), json!({"TypeID":permanent_type,"ItemId":item_id,"InstanceId":instance,"DynamicOptionAssetIDs":[]}));
            }
        }
        groups
            .into_iter()
            .map(|(key, value)| (key, value.into_values().collect()))
            .collect()
    }
}
