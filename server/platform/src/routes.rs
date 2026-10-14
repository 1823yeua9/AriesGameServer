use crate::{
    Platform, agent_type, inventory::string, local_jwt, party_id, player_id, replace_strings,
    timestamp,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

type Reply = (u16, Value);
fn error(status: u16, message: &str) -> Reply {
    (status, json!({"error":message}))
}
fn has_fields(value: &Value, fields: &[&str]) -> bool {
    value.is_object() && fields.iter().all(|key| value.get(key).is_some())
}
fn string_array(value: &Value) -> bool {
    value
        .as_array()
        .is_some_and(|a| a.iter().all(Value::is_string))
}

impl Platform {
    pub fn route(&self, method: &str, target: &str, body: &[u8]) -> Reply {
        let path = target.split('?').next().unwrap_or(target);
        let parsed: Result<Value, _> =
            serde_json::from_slice(if body.is_empty() { b"{}" } else { body });
        let request = parsed.as_ref().unwrap_or(&Value::Null);
        let get = method == "GET";
        let post = method == "POST";
        if get && (path.starts_with("/v1/config/") || path.starts_with("/.a.pvp.net/v1/config/")) {
            return (200, self.template("game_config"));
        }
        if get {
            match path {
                "/riotclient/app-port" => return (200, json!(self.config.port)),
                "/riotclient/v1/auth-token" => return (200, json!(self.config.auth_token)),
                "/riotclient/machine-id" => return (200, json!("AAAAAAAAQACAAAAAAAAAAQ==")),
                "/rso-auth/configuration/v3/ready-state" => {
                    return (200, json!({"ready":true,"configAuthProgress":"Complete"}));
                }
                "/entitlements/v1/token" => {
                    let token = local_jwt("valorant-client", json!({"entitlements":[]}));
                    return (
                        200,
                        json!({"accessToken":token,"entitlements":[],"issuer":"https://offline.local","subject":player_id,"token":token}),
                    );
                }
                "/chat/v4/presences" => {
                    let mut value = self.template("presence");
                    value["time"] = json!(timestamp());
                    return (200, json!({"presences":[value]}));
                }
                "/.a.pvp.net/parties/v1/parties/customgameconfigs" => {
                    return (200, self.template("queue_config"));
                }
                "/.a.pvp.net/bloomline/v0/config/valid-queues" => {
                    return (
                        200,
                        json!({"ProcessedQueues":[],"PerformanceScoreQueues":[]}),
                    );
                }
                _ => (),
            }
            if let Some(value) = self.contracts["static_routes"].get(path) {
                return (200, value.clone());
            }
        }
        if post && path == "/voice-chat/v1/initialize-provider/valorant" {
            return if body == br#""vivox""# {
                (200, Value::Null)
            } else {
                error(400, "unsupported_local_voice_provider")
            };
        }
        if post && ["/social/v1/initialize", "/social/v2/initialize"].contains(&path) {
            return (200, Value::Null);
        }
        if post && path == "/product-session/v2/heartbeat" {
            if parsed.is_err() {
                return error(400, "invalid_product_session_heartbeat");
            }
            return if request == &json!({"phase":"Idle","sessionTTL":0})
                || request == &json!({"phase":"Gameplay","sessionTTL":0})
            {
                (200, Value::Null)
            } else {
                error(400, "unsupported_product_session_phase")
            };
        }
        if method == "PUT" && path == "/social/v1/presences/session" {
            return if has_fields(
                request,
                &[
                    "crossPlayPermissions",
                    "party",
                    "player",
                    "productData",
                    "region",
                ],
            ) {
                (200, Value::Null)
            } else {
                error(400, "invalid_presence_session")
            };
        }
        if post && path == "/mailbox/v1/mail" {
            return if request["count"].is_i64()
                && request["startIndex"].is_i64()
                && string_array(&request["includedStates"])
                && string_array(&request["tags"])
            {
                (200, json!([]))
            } else {
                error(400, "invalid_mailbox_request")
            };
        }
        if post && path == "/player-account/lookup/v2/namesets-for-puuids" {
            if !string_array(&request["puuids"])
                || request["puuids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| string(v).is_empty())
            {
                return error(400, "invalid_namesets_request");
            }
            return (
                200,
                json!({"namesets":request["puuids"].as_array().unwrap().iter().map(|id| {let mut value=self.template("nameset"); replace_strings(&mut value,"{player_id}",string(id)); value}).collect::<Vec<_>>()}),
            );
        }
        if post && path == "/.a.pvp.net/name-service/v3/players" {
            return (
                200,
                json!({"DisplayName":"OfflinePlayer#LOCAL","GameName":"OfflinePlayer","Subject":player_id,"TagLine":"LOCAL"}),
            );
        }
        if post && path == "/chat/v6/conversations" {
            if !has_fields(
                request,
                &[
                    "domain",
                    "hidden",
                    "id",
                    "password",
                    "resource",
                    "targetRegion",
                    "type",
                ],
            ) || !request["id"].is_string()
            {
                return error(400, "invalid_chat_conversation_request");
            }
            let id = string(&request["id"]);
            let match_id = self.session.lock().unwrap().match_id.clone();
            if request["domain"] != "offline"
                || request["targetRegion"] != "na"
                || request["type"] != "groupchat"
                || !(id == "offline-local-party"
                    || (!match_id.is_empty()
                        && ["offline-local-range-all", "offline-local-range-team"].contains(&id)))
            {
                return error(404, "chat_conversation_not_found");
            }
            let room = if id == "offline-local-party" {
                format!("{id}@offline.na.pvp.net/{party_id}")
            } else {
                format!("{id}@offline.na.pvp.net/{match_id}")
            };
            return (
                200,
                json!({"cid":room,"directMessages":false,"globalReadership":false,"messageHistory":false,"mid":"","muted":false,"mutedRestriction":false,"type":"groupchat","uiState":{"changedSinceHidden":false,"hidden":false},"unreadCount":"0"}),
            );
        }
        if get && path == format!("/.a.pvp.net/core-game/v1/players/{player_id}") {
            let state = self.session.lock().unwrap();
            return (
                200,
                json!({"Subject":player_id,"MatchID":state.match_id,"Version":state.player_version}),
            );
        }
        if post
            && let Some(id) = path.strip_prefix(&format!(
                "/.a.pvp.net/core-game/v1/players/{player_id}/disassociate/"
            ))
        {
            return if self.leave_range(id) {
                (200, json!({}))
            } else {
                error(404, "match_not_associated")
            };
        }
        if get && let Some(tail) = path.strip_prefix("/.a.pvp.net/core-game/v1/matches/") {
            let (id, resource) = tail.split_once('/').unwrap_or((tail, ""));
            if id.is_empty() || self.session.lock().unwrap().match_id != id {
                return error(404, "range_match_not_started");
            }
            match resource {
                "" => return (200, self.range_snapshot(id)),
                "loadouts" => {
                    return (
                        200,
                        json!({"Loadouts":[{"Subject":player_id,"CharacterID":"add6443a-41bd-e414-f6ad-e58d267f4e95","Loadout":{"Sprays":{"SpraySelections":[]},"Expressions":{"AESSelections":[]},"Items":{},"DynamicOptions":{}}}]}),
                    );
                }
                "teamchatmuctoken" | "allchatmuctoken" => {
                    return (
                        200,
                        json!({"Token":"offline-local-muc-token","Room":self.range_snapshot(id)[if resource=="teamchatmuctoken" {"TeamMUCName"} else {"AllMUCName"}]}),
                    );
                }
                _ => (),
            }
        }
        if post && path.starts_with("/.a.pvp.net/daily-ticket/v1/") && path.ends_with("/renew") {
            if path != format!("/.a.pvp.net/daily-ticket/v1/{player_id}/renew") {
                return error(404, "daily_ticket_player_not_found");
            }
            return (
                200,
                json!({"Version":1,"DailyRewards":{"RemainingLifetimeSeconds":86400,"BonusMilestonesPending":0,"Milestones":[]},"ProcessedMatches":[]}),
            );
        }
        if post && path == format!("/.a.pvp.net/mass-rewards/v1/players/{player_id}/reconcile") {
            return (
                200,
                json!({"Version":1,"Subject":player_id,"Ceremonies":[]}),
            );
        }
        if path == format!("/.a.pvp.net/personalization/v3/players/{player_id}/playerloadout") {
            if get {
                return (200, self.inventory.lock().unwrap().get_loadout());
            }
            if method == "PUT" {
                return match self.inventory.lock().unwrap().put_loadout(request.clone()) {
                    Ok(value) => (200, value),
                    Err(_) => error(400, "invalid_player_loadout"),
                };
            }
        }
        let entitlements_path = format!("/.a.pvp.net/store/v1/entitlements/{player_id}");
        if get && path == entitlements_path {
            let mut types = vec![
                json!({"ItemTypeID":agent_type,"Entitlements":[self.template("agent_entitlement")]}),
            ];
            types.extend(
                self.inventory
                    .lock()
                    .unwrap()
                    .entitlements()
                    .into_iter()
                    .map(|(id, items)| json!({"ItemTypeID":id,"Entitlements":items})),
            );
            return (200, json!({"EntitlementsByTypes":types}));
        }
        if get && let Some(id) = path.strip_prefix(&(entitlements_path + "/")) {
            if id.is_empty() || id.contains('/') {
                return error(404, "entitlement_type_not_found");
            }
            return (
                200,
                json!({"Entitlements":if id==agent_type {vec![self.template("agent_entitlement")]} else {self.inventory.lock().unwrap().entitlements().shift_remove(&id.to_lowercase()).unwrap_or_default()}}),
            );
        }
        if post && path == format!("/.a.pvp.net/store/v3/storefront/{player_id}") {
            return (200, self.template("storefront"));
        }
        if post
            && path.starts_with("/.a.pvp.net/session/v2/sessions/")
            && path.ends_with("/connect")
        {
            if path != format!("/.a.pvp.net/session/v2/sessions/{player_id}/connect") {
                return error(404, "session_player_not_found");
            }
            if !has_fields(
                request,
                &["isRestricted", "pas", "lq", "userInfoToken", "platformInfo"],
            ) {
                return error(400, "invalid_session_connect_request");
            }
            return (200, self.session_snapshot());
        }
        if get && path == format!("/.a.pvp.net/session/v1/sessions/{player_id}") {
            return (200, self.session_snapshot());
        }
        if post
            && path.starts_with("/.a.pvp.net/session/v1/sessions/")
            && path.ends_with("/heartbeat")
        {
            if path != format!("/.a.pvp.net/session/v1/sessions/{player_id}/heartbeat") {
                return error(404, "session_player_not_found");
            }
            if !has_fields(
                request,
                &[
                    "raw",
                    "subject",
                    "cXNState",
                    "clientId",
                    "loopState",
                    "loopStateMetadata",
                    "version",
                    "lastHeartbeatTime",
                    "expiredTime",
                    "heartbeatIntervalMillis",
                    "heartbeatIntervalMSString",
                    "playtimeNotification",
                    "playtimeMinutes",
                    "restrictionType",
                    "userInfoValidTime",
                    "ticket",
                    "shouldForceInvalidate",
                ],
            ) {
                return error(400, "invalid_session_heartbeat_request");
            }
            if request["subject"] != player_id {
                return error(400, "session_subject_mismatch");
            }
            let mut reply = request.clone();
            let state = self.session.lock().unwrap();
            if state.loop_state != "MENUS" {
                reply["loopState"] = json!(state.loop_state);
                reply["version"] = json!(state.version);
            }
            return (200, reply);
        }
        if get
            && path.starts_with("/.a.pvp.net/favorites/v1/players/")
            && path.ends_with("/favorites")
        {
            return (200, json!({"FavoritedContent":[]}));
        }
        if post && path == format!("/.a.pvp.net/parties/v1/players/{player_id}/startsoloexperience")
        {
            if parsed.is_err() {
                return error(400, "invalid_solo_experience_request");
            }
            if request != &json!({"gameType":"ShootingRange","module":"Aimbots_spawn"}) {
                return error(400, "unsupported_solo_experience");
            }
            self.start_range();
            return (200, self.template("party_player"));
        }
        if get && let Some(id) = path.strip_prefix("/.a.pvp.net/parties/v1/players/") {
            return if id == player_id {
                (200, self.template("party_player"))
            } else {
                error(404, "party_player_not_found")
            };
        }
        if post
            && path
                == format!(
                    "/.a.pvp.net/parties/v1/parties/{party_id}/members/{player_id}/refreshPlayerIdentity"
                )
        {
            return (200, self.party_snapshot());
        }
        if get && let Some(id) = path.strip_prefix("/.a.pvp.net/parties/v1/parties/") {
            return if id == party_id {
                (200, self.party_snapshot())
            } else {
                error(404, "party_not_found")
            };
        }
        if get && path.starts_with("/.a.pvp.net/parties/v2/parties/") && path.ends_with("/muctoken")
        {
            return if path == format!("/.a.pvp.net/parties/v2/parties/{party_id}/muctoken") {
                (
                    200,
                    json!({"Token":"offline-local-muc-token","Room":format!("offline-local-party@offline.na.pvp.net/{party_id}")}),
                )
            } else {
                error(404, "party_not_found")
            };
        }
        if get && let Some(tail) = path.strip_prefix("/.a.pvp.net/bloomline/v0/players/") {
            let parts: Vec<_> = tail.split('/').collect();
            let id = parts[0].to_lowercase();
            if id != player_id && id != player_id.replace('-', "") {
                return error(404, "bloomline_player_not_found");
            }
            if parts.len() != 1 {
                if parts.len() != 3 || parts[1] != "bonsai" {
                    return error(404, "bloomline_resource_not_found");
                }
                if !uuid::Uuid::parse_str(parts[2]).is_ok_and(|id| {
                    [id.to_string(), id.simple().to_string()].contains(&parts[2].to_lowercase())
                }) {
                    return error(400, "invalid_bonsai_id");
                }
            }
            return (
                200,
                json!({"Subject":player_id,"Bonsais":[],"LongGroupVersion":0}),
            );
        }
        if post && path == "/.a.pvp.net/latency/v1/ingestMulti" {
            return (200, json!({}));
        }
        if post && path == "/player-affinity/product/v2/token" {
            if request["product"] != "valorant" {
                return error(400, "unsupported_product");
            }
            return (
                200,
                json!({"affinities":{"live":"na"},"expiry":4102444800000u64,"issuedAt":0,"product":"valorant","puuid":player_id,"token":local_jwt("player-affinity",json!({"affinities":{"live":"na"},"product":"valorant"}))}),
            );
        }
        if get && path.starts_with("/riot-messaging-service/v1/messages/") {
            let state = self.session.lock().unwrap();
            let version = state.version;
            let player_version = state.player_version;
            let party_version = state.party_version;
            let in_game = !state.match_id.is_empty();
            let starting = state.party_state == "SOLO_EXPERIENCE_STARTING";
            drop(state);
            if path
                == format!(
                    "/riot-messaging-service/v1/messages/ares-session/v1/sessions/{player_id}"
                )
            {
                return (
                    200,
                    json!([self.rms_message(
                        "ares-session",
                        &format!("ares-session/v1/sessions/{player_id}"),
                        version,
                        self.session_snapshot()
                    )]),
                );
            }
            if path
                == format!(
                    "/riot-messaging-service/v1/messages/ares-core-game/core-game/v1/players/{player_id}"
                )
            {
                return (
                    200,
                    if in_game {
                        json!([self.rms_message(
                            "ares-core-game",
                            &format!("core-game/v1/players/{player_id}"),
                            player_version,
                            json!({"method":"GET"})
                        )])
                    } else {
                        json!([])
                    },
                );
            }
            if path.starts_with("/riot-messaging-service/v1/messages/ares-parties/") {
                return (
                    200,
                    if starting
                        && path
                            == format!(
                                "/riot-messaging-service/v1/messages/ares-parties/parties/v1/parties/{party_id}"
                            )
                    {
                        json!([self.rms_message(
                            "ares-parties",
                            &format!("parties/v1/parties/{party_id}"),
                            party_version,
                            json!({"method":"GET"})
                        )])
                    } else {
                        json!([])
                    },
                );
            }
            if path.starts_with("/riot-messaging-service/v1/messages/ares-daily-ticket/") || ["/riot-messaging-service/v1/messages/ares-core-game/core-game/v1/provisionfailures","/riot-messaging-service/v1/messages/ares-core-game/core-game/v1/provisionversionfailures"].contains(&path) {return (200,json!([]));}
        }
        if post && path == "/rso-auth/v2/authorizations" {
            if parsed.is_err() || !request.is_object() {
                return error(400, "invalid_json");
            }
            return (
                200,
                self.authorization(
                    string(request.get("clientId").unwrap_or(&json!("valorant-client"))),
                    request.get("scope").cloned().unwrap_or(json!([])),
                ),
            );
        }
        if get && let Some(id) = path.strip_prefix("/rso-auth/v2/authorizations/") {
            return if id.is_empty() || id.contains('/') {
                error(404, "authorization_not_found")
            } else {
                (
                    200,
                    self.authorization(id, json!(["openid", "link", "ban", "account", "lol"])),
                )
            };
        }
        if (get || post)
            && let Some(id) = path.strip_prefix("/rso-auth/v1/userinfo/")
        {
            return if id.is_empty() || id.contains('/') {
                error(404, "userinfo_not_found")
            } else {
                (
                    200,
                    json!({"userInfo":local_jwt("valorant-client",json!({"country":"USA","ban":{"restrictions":[]},"acct":{"type":0,"state":"ENABLED","adm":false,"game_name":"OfflinePlayer","tag_line":"LOCAL"}}))}),
                )
            };
        }
        if post && path == "/product-integration/v1/settings-token" {
            let missing: Vec<_> = ["productSessionId", "productId", "patchlineId"]
                .into_iter()
                .filter(|key| request.get(*key).is_none())
                .collect();
            return if !missing.is_empty() {
                (400, json!({"error":"missing_fields","fields":missing}))
            } else {
                (
                    200,
                    json!({"token":STANDARD.encode(serde_json::to_vec(&self.settings()).unwrap())}),
                )
            };
        }
        (
            404,
            json!({"error":"unhandled_route","method":method,"path":path}),
        )
    }
    fn authorization(&self, client_id: &str, scopes: Value) -> Value {
        let token = local_jwt(client_id, Value::Null);
        json!({"type":"authorized","authorization":{"accessToken":{"token":token,"scopes":scopes,"expiry":4102444800u64,"clientId":client_id},"idToken":{"token":token,"expiry":4102444800u64,"clientId":client_id,"nonce":"offline"}},"country":"US"})
    }
    pub fn admin_route(&self, method: &str, path: &str, body: &[u8]) -> Reply {
        if method == "GET" && path == "/health" {
            return (
                200,
                json!({"service":"aries-platform","pid":std::process::id(),"port":self.config.port,"ready":true}),
            );
        }
        if method == "POST" && path == "/dev/range/enter" {
            return (200, json!({"match_id":self.start_range()}));
        }
        let mut inventory = self.inventory.lock().unwrap();
        if method == "GET" && path == "/dev/skins" {
            return (200, inventory.snapshot());
        }
        if method == "POST" && path == "/dev/skins/grant-all" {
            return match inventory.grant(None) {
                Ok(v) => (200, v),
                Err(e) => error(400, &e),
            };
        }
        if [("POST", "/dev/skins/grant"), ("PUT", "/dev/skins/equip")].contains(&(method, path)) {
            if body.len() > 4096 {
                return error(400, "invalid_content_length");
            }
            let Ok(request) = serde_json::from_slice::<Value>(body) else {
                return error(400, "invalid_json");
            };
            if !request.is_object() {
                return error(400, "invalid_json_object");
            }
            let result = if method == "POST" {
                request["skin_id"]
                    .as_str()
                    .ok_or("unknown_skin_id".to_string())
                    .and_then(|id| inventory.grant(Some(id)))
            } else {
                match (request["weapon_id"].as_str(), request["skin_id"].as_str()) {
                    (Some(w), Some(s)) => inventory.equip(w, s),
                    _ => Err("invalid_skin_selection".into()),
                }
            };
            return match result {
                Ok(v) => (200, v),
                Err(e) => error(400, &e),
            };
        }
        error(404, "unknown_admin_route")
    }
}
