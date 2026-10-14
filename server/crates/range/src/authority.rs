//! the per-packet authority block (python 1566-2078): after ClientRestart,
//! every authenticated datagram runs these features in this fixed order,
//! before the session-ack tick (so their frames take the lower packet
//! sequences). docs/port/05_authority_loop.md.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use aries_bits::Bits;
use aries_rep::decode::RangeSettingsRequest;
use serde_json::{Map, Value, json};

use crate::bringup::{STARTING_EQUIPPABLES, Stage};
use crate::config::RangeConfig;
use crate::live::LiveClient;
use crate::send::Reply;

/// python input_correction_polled gate and resend spacing.
pub const INPUT_CORRECTION_POLL: f64 = 0.2;
pub const INPUT_CORRECTION_RESEND: f64 = 1.0;
/// python range streaming / settings request delays after the range hud.
pub const STREAMING_DELAY: f64 = 1.0;
pub const SETTINGS_REQUEST_DELAY: f64 = 3.0;
pub const COMMAND_QUEUE_POLL: f64 = 0.2;
pub const HUD_REFRESH_POLL: f64 = 1.0;
pub const HUD_REFRESH_LIMIT: u32 = 6;

/// per-connection authority state (python run_probe dicts keyed by source).
#[derive(Debug, Default, Clone)]
pub struct AuthorityState {
    pub input_correction_polled: Option<f64>,
    pub input_correction_sent: Option<f64>,
    pub range_hud_sent: Option<f64>,
    pub streaming_sent: bool,
    pub ultimate_points_sent: bool,
    pub settings_requested: bool,
    /// python range_settings_sequences_seen (172..177 share it)
    pub settings_sequences_seen: HashSet<u32>,
    /// the last ServerUpdateShootingRangeSettings (python range_settings_state)
    pub settings: Option<RangeSettingsRequest>,
    /// python range_infinite_ammo_sent
    pub infinite_ammo_sent: Option<bool>,
    pub command_queue_polled: Option<f64>,
    pub hud_refresh_polled: Option<f64>,
    pub hud_refresh_count: u32,
    /// python purchased_class_guids: class -> actor guid of bought items
    pub purchased_class_guids: HashMap<String, u32>,
}

fn due(last: Option<f64>, now: f64, interval: f64) -> bool {
    last.is_none_or(|t| now - t >= interval)
}

fn starting_equippables_done(reply: &Reply) -> bool {
    reply.conn.bringup.starting_equippable_index >= STARTING_EQUIPPABLES
}

/// python 1566-2078 in order. the gate (ClientRestart sent) is checked by the caller.
pub fn run_authority(reply: &mut Reply, config: &RangeConfig, live: &mut dyn LiveClient, decoded: &Bits) {
    // 2 equippable mirror (+ firing seed shots, cloudburst hook): world integration
    if config.features.input_correction {
        input_correction(reply, live);
    }
    if config.features.range_hud {
        range_hud(reply);
    }
    if config.features.streaming_levels {
        level_streaming(reply);
    }
    ultimate_points(reply, live);
    // 8 store purchases: world integration
    if config.features.range_settings && reply.conn.authority.streaming_sent {
        range_settings_request(reply);
        range_settings_apply(reply, decoded);
    }
    if let Some(queue) = &config.command_queue {
        command_queue(reply, queue);
    }
    if config.features.hud_refresh {
        hud_refresh(reply, live);
    }
    // 12 cloudburst flights, 13 bots, 14 ability jobs: world integration
}

/// 6.5: the client stopped replicating inputs (>256 unacked or a stale frame);
/// it resumes once the inventory CorrectionIndex passes the one it recorded.
fn input_correction(reply: &mut Reply, live: &mut dyn LiveClient) {
    if reply.conn.correction_index < 5 || !starting_equippables_done(reply) || !due(reply.conn.authority.input_correction_polled, reply.now, INPUT_CORRECTION_POLL) {
        return;
    }
    reply.conn.authority.input_correction_polled = Some(reply.now);
    let replication = match live.input_replication() {
        Ok(Some(replication)) => replication,
        Ok(None) => return,
        Err(error) => {
            reply.event.set("input_correction_error", error);
            return;
        }
    };
    if !replication.stopped || !due(reply.conn.authority.input_correction_sent, reply.now, INPUT_CORRECTION_RESEND) {
        return;
    }
    let index = replication.requested_correction.max(replication.inventory_correction).max(reply.conn.correction_index) + 1;
    let class = replication.current_equippable_class.clone().unwrap_or_default();
    let purchased = reply.conn.authority.purchased_class_guids.get(&class).copied();
    let body = match aries_rep::setup::input_resume_correction_body(index, purchased, &class) {
        Ok(body) => body,
        Err(error) => {
            reply.event.set("input_correction_error", error.to_string());
            return;
        }
    };
    match reply.send_reliable(8, &body, false, false, false) {
        Ok(_) => {
            reply.conn.correction_index = index;
            reply.conn.authority.input_correction_sent = Some(reply.now);
            reply.event.set("sent_input_resume_correction", index);
            reply.event.set("input_resume_equippable", replication.current_equippable_class.map(Value::from).unwrap_or(Value::Null));
        }
        Err(error) => reply.event.set("input_correction_error", error.to_string()),
    }
}

/// 6.6: GameModeClass + HUD config on the GameState, then ClientSetHUD again.
fn range_hud(reply: &mut Reply) {
    let Some((field, max)) = reply.conn.bringup.hud_rpc else {
        return;
    };
    if !reply.conn.bringup.sent(Stage::RangeShop) || reply.conn.authority.range_hud_sent.is_some() {
        return;
    }
    use aries_rep::bodies::{RANGE_GAME_MODE_CLASS, RANGE_HUD_CONFIG_CLASS};
    let sends = [
        (4, aries_rep::setup::range_class_property_body(&RANGE_GAME_MODE_CLASS), true),
        (4, aries_rep::setup::range_class_property_body(&RANGE_HUD_CONFIG_CLASS), true),
        (5, aries_rep::setup::hud_rebuild_body(field, max), false),
    ];
    for (channel, body, exports) in sends {
        let result = body.map_err(|e| e.to_string()).and_then(|body| reply.send_reliable(channel, &body, exports, false, false).map_err(|e| e.to_string()));
        if let Err(error) = result {
            reply.event.set("range_hud_error", error);
            return;
        }
    }
    reply.conn.authority.range_hud_sent = Some(reply.now);
    reply.event.set("sent_range_mode_hud", true);
}

/// 6.7: stream in Range_Only_Gameplay and the LiveStory sublevel, then flush.
fn level_streaming(reply: &mut Reply) {
    let Some(hud_time) = reply.conn.authority.range_hud_sent else {
        return;
    };
    if reply.conn.authority.streaming_sent || reply.now - hud_time < STREAMING_DELAY {
        return;
    }
    for body in [aries_rep::setup::levels_streaming_body(), aries_rep::setup::flush_level_streaming_body()] {
        let result = body.map_err(|e| e.to_string()).and_then(|body| reply.send_reliable(5, &body, false, false, false).map_err(|e| e.to_string()));
        if let Err(error) = result {
            reply.event.set("range_streaming_error", error);
            return;
        }
    }
    reply.conn.authority.streaming_sent = true;
    reply.event.set("sent_range_streaming_levels", json!(aries_rep::rpc::RANGE_GAMEPLAY_SUBLEVELS));
}

/// 6.8: NumUltimatePoints from the pawn, every pass until a value in 1..49 is read.
fn ultimate_points(reply: &mut Reply, live: &mut dyn LiveClient) {
    if !reply.conn.bringup.sent(Stage::RangeMoney) || reply.conn.authority.ultimate_points_sent {
        return;
    }
    let points = match live.ultimate_points_requirement() {
        Ok(Some(points)) => points,
        Ok(None) => return,
        Err(error) => {
            reply.event.set("ultimate_points_error", error);
            return;
        }
    };
    if !(1..50).contains(&points) {
        return;
    }
    let result = aries_rep::setup::ultimate_points_body(points)
        .map_err(|e| e.to_string())
        .and_then(|body| reply.send_reliable(6, &body, false, false, false).map_err(|e| e.to_string()));
    match result {
        Ok(_) => {
            reply.conn.authority.ultimate_points_sent = true;
            reply.event.set("sent_ultimate_points", points);
        }
        Err(error) => reply.event.set("ultimate_points_error", error),
    }
}

/// 6.10 (request half): ask the client for its range settings once, 3 s after the range hud.
fn range_settings_request(reply: &mut Reply) {
    if reply.conn.authority.settings_requested {
        return;
    }
    if reply.conn.authority.range_hud_sent.is_some_and(|t| reply.now - t < SETTINGS_REQUEST_DELAY) {
        return;
    }
    let result = aries_rep::setup::range_settings_request_body()
        .map_err(|e| e.to_string())
        .and_then(|body| reply.send_reliable(5, &body, false, false, false).map_err(|e| e.to_string()));
    match result {
        Ok(_) => {
            reply.conn.authority.settings_requested = true;
            reply.event.set("sent_client_update_range_settings", true);
        }
        Err(error) => reply.event.set("range_settings_error", error),
    }
}

/// 6.10 (apply half): each new 172..177 rpc is logged; 176 carries the settings
/// struct and its infinite-ammo flag goes to GameState GameInfo handle 46,
/// which the client's ammo and reload paths read natively.
fn range_settings_apply(reply: &mut Reply, decoded: &Bits) {
    let requests = aries_rep::decode::decode_range_settings_requests(decoded);
    for request in aries_rep::decode::take_new_range_settings(requests, &mut reply.conn.authority.settings_sequences_seen) {
        reply.event.push("received_range_settings", settings_event(&request));
        if request.field != aries_rep::decode::SERVER_UPDATE_SHOOTING_RANGE_SETTINGS_FIELD {
            continue;
        }
        let infinite_ammo = request.infinite_ammo;
        reply.conn.authority.settings = Some(request);
        if reply.conn.authority.infinite_ammo_sent == Some(infinite_ammo) {
            continue;
        }
        // python marks it sent only after the send returns
        let result = aries_rep::setup::infinite_ammo_body(infinite_ammo)
            .map_err(|e| e.to_string())
            .and_then(|body| reply.send_reliable(4, &body, false, false, false).map_err(|e| e.to_string()));
        match result {
            Ok(_) => {
                reply.conn.authority.infinite_ammo_sent = Some(infinite_ammo);
                reply.event.set("sent_infinite_ammo", infinite_ammo);
            }
            Err(error) => reply.event.set("range_settings_error", error),
        }
    }
}

/// python {key: value for key, value in request.__dict__.items() if value}
fn settings_event(request: &RangeSettingsRequest) -> Value {
    let mut map = Map::new();
    let mut put = |key: &str, value: Value, truthy: bool| {
        if truthy {
            map.insert(key.to_string(), value);
        }
    };
    put("field", json!(request.field), request.field != 0);
    put("channel_sequence", json!(request.channel_sequence), request.channel_sequence != 0);
    put("bot_armor", json!(true), request.bot_armor);
    put("bot_strafe", json!(true), request.bot_strafe);
    put("infinite_ammo", json!(true), request.infinite_ammo);
    put("skill_test", json!(request.skill_test), request.skill_test != 0);
    put("plant_difficulty", json!(request.plant_difficulty), request.plant_difficulty != 0);
    put("defuse_difficulty", json!(request.defuse_difficulty), request.defuse_difficulty != 0);
    put("practice_mode", json!(true), request.practice_mode);
    put("bot_amount_index", json!(request.bot_amount_index), request.bot_amount_index != 0);
    put("distance_index", json!(request.distance_index), request.distance_index != 0);
    put("bot_enabled", json!(true), request.bot_enabled);
    put("is_init_settings_flow", json!(true), request.is_init_settings_flow);
    if let Some(path) = request.object_path.as_deref().filter(|p| !p.is_empty()) {
        put("object_path", json!(path), true);
    }
    Value::Object(map)
}

/// 6.11: a debug hook. json files dropped in the queue dir are sent as
/// reliable bunches ({"channel", "body_hex", "bits", "exports"?, "open"?}).
fn command_queue(reply: &mut Reply, queue: &Path) {
    if !due(reply.conn.authority.command_queue_polled, reply.now, COMMAND_QUEUE_POLL) {
        return;
    }
    reply.conn.authority.command_queue_polled = Some(reply.now);
    let Ok(entries) = std::fs::read_dir(queue) else {
        return;
    };
    let mut files: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    for file in files {
        let text = std::fs::read_to_string(&file);
        let _ = std::fs::remove_file(&file);
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let result = text.map_err(|e| e.to_string()).and_then(|text| serde_json::from_str::<Value>(&text).map_err(|e| e.to_string())).and_then(|command| queued_bunch(reply, &command));
        match result {
            Ok(channel) => reply.event.push("sent_queued_commands", json!([name, channel])),
            Err(error) => reply.event.push("queued_command_errors", json!([name, error])),
        }
    }
}

fn queued_bunch(reply: &mut Reply, command: &Value) -> Result<u32, String> {
    let channel = command["channel"].as_u64().ok_or("missing channel")? as u32;
    let bits = command["bits"].as_u64().ok_or("missing bits")? as usize;
    let body = aries_bits::Bits::from_hex(command["body_hex"].as_str().ok_or("missing body_hex")?, bits).map_err(|e| e.to_string())?;
    let open = command["open"].as_bool().unwrap_or(false);
    if open {
        reply.conn.channel_first_offset.entry(channel).or_insert(0);
    }
    reply.send_reliable(channel, &body, command["exports"].as_bool().unwrap_or(false), open, false).map_err(|e| e.to_string())?;
    Ok(channel)
}

/// 6.12: the bottom hud starts collapsed when load-in cinematic mode is still
/// on; re-sending SpawnedCharacter (null, then the pawn) makes it re-evaluate.
fn hud_refresh(reply: &mut Reply, live: &mut dyn LiveClient) {
    let state = &reply.conn.authority;
    if !starting_equippables_done(reply) || !due(state.hud_refresh_polled, reply.now, HUD_REFRESH_POLL) || state.hud_refresh_count >= HUD_REFRESH_LIMIT {
        return;
    }
    reply.conn.authority.hud_refresh_polled = Some(reply.now);
    let (collapsed, cinematic) = match live.hud_widget() {
        Ok(Some(widget)) => (widget.collapsed, widget.cinematic),
        Ok(None) => return,
        Err(error) => {
            reply.event.set("hud_refresh_error", error);
            (false, true)
        }
    };
    if !collapsed || cinematic {
        return;
    }
    for body in [aries_rep::setup::spawned_character_null_body(), aries_rep::setup::player_state_spawned_character_link()] {
        let result = body.map_err(|e| e.to_string()).and_then(|body| reply.send_reliable(6, &body, false, false, false).map_err(|e| e.to_string()));
        if let Err(error) = result {
            reply.event.set("hud_refresh_error", error);
            return;
        }
    }
    reply.conn.authority.hud_refresh_count += 1;
    reply.event.set("sent_hud_refresh", reply.conn.authority.hud_refresh_count);
}
