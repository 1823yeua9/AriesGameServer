#![allow(non_upper_case_globals)]
pub mod inventory;
mod routes;
mod transport;

use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use inventory::Inventory;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::broadcast;

pub const player_id: &str = "7c9f41a4-65e2-4d81-a8d5-f879c3e5ab61";
pub const party_id: &str = "00000000-0000-0000-0000-000000000001";
pub const agent_type: &str = "01bb38e1-da47-4e6a-9b3d-945fe4655707";
pub const rms_topic: &str = "OnJsonApiEvent_riot-messaging-service_v1_messages";

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub port: u16,
    pub admin_port: u16,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub state_path: PathBuf,
    pub skin_catalog: Option<PathBuf>,
    pub skin_state: Option<PathBuf>,
    pub persistence_path: PathBuf,
    pub startup_gate: Option<PathBuf>,
    pub auth_token: String,
    pub admin_token: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            port: 58089,
            admin_port: 58090,
            cert_path: "remoting_cert.pem".into(),
            key_path: "remoting_key.pem".into(),
            state_path: "backend_state.json".into(),
            skin_catalog: None,
            skin_state: None,
            persistence_path: "persistence".into(),
            startup_gate: None,
            auth_token: String::new(),
            admin_token: String::new(),
        }
    }
}

pub struct Session {
    pub party_state: String,
    pub previous_state: String,
    pub party_version: u64,
    pub loop_state: String,
    pub version: u64,
    pub match_id: String,
    pub player_version: u64,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            party_state: "DEFAULT".into(),
            previous_state: "DEFAULT".into(),
            party_version: 1,
            loop_state: "MENUS".into(),
            version: 1,
            match_id: String::new(),
            player_version: 1,
        }
    }
}

pub struct Platform {
    pub config: Config,
    pub inventory: Mutex<Inventory>,
    pub session: Mutex<Session>,
    pub events: broadcast::Sender<Value>,
    /// The game-server subscribes before serving requests to provision each match.
    pub range_requests: broadcast::Sender<String>,
    contracts: Value,
}

pub fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn local_jwt(audience: &str, extra: Value) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT"}"#);
    let mut claims = json!({"sub":player_id,"puuid":player_id,"iss":"https://offline.local","aud":audience,"iat":1700000000u64,"exp":4102444800u64,"acct":{"game_name":"OfflinePlayer","tag_line":"LOCAL"}});
    if let Some(extra) = extra.as_object() {
        for (key, value) in extra {
            claims[key] = value.clone();
        }
    }
    format!(
        "{header}.{}.",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
    )
}
pub fn replace_strings(value: &mut Value, from: &str, to: &str) {
    match value {
        Value::String(text) => *text = text.replace(from, to),
        Value::Array(items) => items.iter_mut().for_each(|v| replace_strings(v, from, to)),
        Value::Object(items) => items
            .values_mut()
            .for_each(|v| replace_strings(v, from, to)),
        _ => (),
    }
}

impl Platform {
    pub fn new(mut config: Config) -> Result<Arc<Self>, String> {
        if config.auth_token.is_empty() {
            config.auth_token = uuid::Uuid::new_v4().simple().to_string();
        }
        if config.admin_token.is_empty() {
            config.admin_token = format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            );
        }
        if config.admin_token.len() < 24 {
            return Err("admin token is too short".into());
        }
        let inventory = Inventory::open(config.skin_catalog.clone(), config.skin_state.clone())?;
        let mut contracts: Value = serde_json::from_str(include_str!("../data/contracts.json"))
            .map_err(|e| e.to_string())?;
        replace_strings(
            &mut contracts,
            "127.0.0.1:58089",
            &format!("127.0.0.1:{}", config.port),
        );
        Ok(Arc::new(Self {
            config,
            inventory: Mutex::new(inventory),
            session: Mutex::new(Session::default()),
            events: broadcast::channel(256).0,
            range_requests: broadcast::channel(32).0,
            contracts,
        }))
    }
    pub fn template(&self, name: &str) -> Value {
        self.contracts["templates"][name].clone()
    }
    pub fn settings(&self) -> Value {
        json!({"publisher":"local","product_id":"valorant","patchline_id":"live","default_region":"NA","region_data":{"NA":{"default_locale":"en_US"}},"riotgamesapi":{"persistence-path":self.config.persistence_path}})
    }
    pub fn party_snapshot(&self) -> Value {
        let state = self.session.lock().unwrap();
        let mut value = self.template("party");
        value["State"] = json!(state.party_state);
        value["PreviousState"] = json!(state.previous_state);
        value["Version"] = json!(state.party_version);
        value["StateTransitionReason"] =
            json!(if state.party_state == "SOLO_EXPERIENCE_STARTING" {
                "GAME_STARTING"
            } else {
                "NONE"
            });
        value
    }
    pub fn session_snapshot(&self) -> Value {
        let state = self.session.lock().unwrap();
        let mut value = self.template("session");
        value["loopState"] = json!(state.loop_state);
        value["version"] = json!(state.version);
        value
    }
    pub fn range_snapshot(&self, match_id: &str) -> Value {
        let mut value = self.template("range_match");
        replace_strings(&mut value, "{match_id}", match_id);
        value["TeamMatchToken"] = json!(local_jwt("offline-range", json!({"match_id":match_id})));
        value
    }
    pub fn rms_message(
        &self,
        service: &str,
        resource: &str,
        version: u64,
        payload: Value,
    ) -> Value {
        json!({"id":format!("offline-{service}-message-{version}"),"resource":resource,"service":service,"version":version.to_string(),"timestamp":timestamp(),"payload":serde_json::to_string(&payload).unwrap(),"ackRequired":false,"traceContext":{"traceParent":""}})
    }
    fn publish(
        &self,
        service: &str,
        resource: &str,
        version: u64,
        payload: Value,
        message_resource: Option<&str>,
    ) {
        let message = self.rms_message(
            service,
            message_resource.unwrap_or(resource),
            version,
            payload,
        );
        let _ = self.events.send(json!([8,rms_topic,{"eventType":"Update","uri":format!("/riot-messaging-service/v1/messages/{service}/{resource}"),"data":[message]}]));
    }
    fn publish_state(&self, party_changed: bool) {
        let (party_version, version, player_version) = {
            let state = self.session.lock().unwrap();
            (state.party_version, state.version, state.player_version)
        };
        if party_changed {
            self.publish(
                "ares-parties",
                &format!("parties/v1/parties/{party_id}"),
                party_version,
                json!({"method":"GET"}),
                None,
            );
            self.publish(
                "ares-session",
                &format!("v1/sessions/{player_id}"),
                version,
                self.session_snapshot(),
                Some(&format!("ares-session/v1/sessions/{player_id}")),
            );
        }
        self.publish(
            "ares-core-game",
            &format!("core-game/v1/players/{player_id}"),
            player_version,
            json!({"method":"GET"}),
            None,
        );
    }
    pub fn start_range(&self) -> String {
        let (match_id, changed) = {
            let mut state = self.session.lock().unwrap();
            let changed = state.party_state != "SOLO_EXPERIENCE_STARTING";
            if changed {
                state.previous_state = state.party_state.clone();
                state.party_state = "SOLO_EXPERIENCE_STARTING".into();
                state.party_version += 1;
                state.loop_state = "INGAME".into();
                state.version += 1;
            }
            state.match_id = uuid::Uuid::new_v4().to_string();
            state.player_version += 1;
            (state.match_id.clone(), changed)
        };
        let _ = self.range_requests.send(match_id.clone());
        self.publish_state(changed);
        match_id
    }
    pub fn leave_range(&self, requested: &str) -> bool {
        {
            let mut state = self.session.lock().unwrap();
            if state.match_id.is_empty() || state.match_id != requested {
                return false;
            }
            state.match_id.clear();
            state.player_version += 1;
            state.party_state = if state.previous_state == "SOLO_EXPERIENCE_STARTING" {
                "DEFAULT".into()
            } else {
                state.previous_state.clone()
            };
            state.previous_state = "SOLO_EXPERIENCE_STARTING".into();
            state.party_version += 1;
            state.loop_state = "MENUS".into();
            state.version += 1;
        }
        self.publish_state(true);
        true
    }
    pub fn authorized(&self, value: &str, admin: bool) -> bool {
        use subtle::ConstantTimeEq;
        let expected = if admin {
            format!("Bearer {}", self.config.admin_token)
        } else {
            format!(
                "Basic {}",
                STANDARD.encode(format!("riot:{}", self.config.auth_token))
            )
        };
        bool::from(value.as_bytes().ct_eq(expected.as_bytes()))
    }
}

pub use transport::{serve, serve_platform};
