//! range settings. defaults are the python production configuration
//! (launch.ps1 + launch_range.py); every feature can be turned off for a/b tests.

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RangeConfig {
    /// pawn spawn location (A06) and the one-shot placement correction (move ack)
    pub spawn_location: [f64; 3],
    /// wushu (jett) desired class guid words, PlayerState handles 201..204
    pub desired_class_guid: [u32; 4],
    /// GameState.Phase value (B2)
    pub game_state_phase: u32,
    /// bombteam team number for the player (B8)
    pub player_team: u32,
    pub bot_count: usize,
    /// saved skins and the catalog the backend owns
    pub skin_catalog: PathBuf,
    pub skin_state: PathBuf,
    pub player_id: String,
    /// debug queue directory (python runtime/responder_queue)
    pub command_queue: Option<PathBuf>,
    pub features: Features,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Features {
    pub pistol_initial_cosmetic: bool,
    pub equippable_mirror: bool,
    pub input_correction: bool,
    pub store_purchases: bool,
    pub range_hud: bool,
    pub ability_activation: bool,
    pub streaming_levels: bool,
    pub range_settings: bool,
    pub bots: bool,
    pub hud_refresh: bool,
    pub ability_projectiles: bool,
    pub move_ack: bool,
    pub input_ack: bool,
    /// resend reliable bunches the client reports lost (new; python never resent)
    pub retransmit: bool,
}

impl Default for Features {
    fn default() -> Self {
        Self {
            pistol_initial_cosmetic: true,
            equippable_mirror: true,
            input_correction: true,
            store_purchases: true,
            range_hud: true,
            ability_activation: true,
            streaming_levels: true,
            range_settings: true,
            bots: true,
            hud_refresh: true,
            ability_projectiles: true,
            move_ack: true,
            input_ack: true,
            retransmit: true,
        }
    }
}

impl Default for RangeConfig {
    fn default() -> Self {
        let backend = PathBuf::from("../local_backend");
        Self {
            spawn_location: [3581.634, -12019.322, 1294.153],
            desired_class_guid: [0xADD6_443A, 0x41BD_E414, 0xF6AD_E58D, 0x267F_4E95],
            game_state_phase: 4,
            player_team: 1,
            bot_count: 5,
            skin_catalog: backend.join("skin_catalog.json"),
            skin_state: backend.join("offline_skin_state.json"),
            player_id: "7c9f41a4-65e2-4d81-a8d5-f879c3e5ab61".into(),
            command_queue: Some(backend.join("runtime").join("responder_queue")),
            features: Features::default(),
        }
    }
}
