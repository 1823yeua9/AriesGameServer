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
    /// lag compensation for shots (docs/port/10 section 4.11)
    pub lagcomp: LagCompSettings,
    /// smoothing of values read from the client (the player position stream)
    pub interp: InterpSettings,
    /// how fast bring-up, purchases and the range setup go
    pub pacing: PacingSettings,
}

/// [range.pacing]. python spaced every bring-up stage and purchase step by fixed
/// delays (0.2-4 s, ~75 s to a playable range, ~2.5 s per gun). they are empirical
/// (03 section 12 Q1): the only real dependency is that the client has the step
/// before. fast waits for exactly that, the client's ack, plus a small gap.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PacingSettings {
    pub mode: PacingMode,
    /// fast: least time between two steps of one chain (seconds)
    pub stage_gap: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PacingMode {
    /// the python delays, bit for bit (shadow diffs, golden timing)
    #[default]
    Python,
    /// ack-driven
    Fast,
}

impl Default for PacingSettings {
    fn default() -> Self {
        Self { mode: PacingMode::Python, stage_gap: 0.05 }
    }
}

impl PacingSettings {
    pub fn fast() -> Self {
        Self { mode: PacingMode::Fast, ..Self::default() }
    }

    pub fn is_fast(&self) -> bool {
        self.mode == PacingMode::Fast
    }

    /// a python delay in this mode: as is, or the fast gap (acks do the real gating).
    pub fn delay(&self, python: f64) -> f64 {
        if self.is_fast() { self.stage_gap.min(python) } else { python }
    }

    /// python retried every `python_retry` s up to `python_tries` times; fast retries
    /// every gap but keeps the same total wait.
    pub fn tries(&self, python_retry: f64, python_tries: u32) -> u32 {
        let retry = self.delay(python_retry);
        if retry <= 0.0 { python_tries } else { (python_retry * python_tries as f64 / retry).ceil() as u32 }
    }
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
            lagcomp: LagCompSettings::default(),
            interp: InterpSettings::default(),
            pacing: PacingSettings::default(),
        }
    }
}

/// [range.lagcomp] (source sdk defaults; view_interp is measured, 0 until then).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LagCompSettings {
    pub enabled: bool,
    /// sv_maxunlag
    pub max_unlag: f64,
    /// sv_lagcompensation_teleport_dist, 2d per record
    pub teleport_dist: f64,
    pub claim_tolerance: f64,
    /// the client's view delay for remote entities (docs/port/10 section 4.13)
    pub view_interp: f64,
}

impl Default for LagCompSettings {
    fn default() -> Self {
        Self { enabled: true, max_unlag: 1.0, teleport_dist: 64.0, claim_tolerance: 0.2, view_interp: 0.0 }
    }
}

/// [range.interp] (source sdk defaults, docs/port/10 section 4.11).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct InterpSettings {
    /// cl_interp_ratio
    pub oracle_interp_ratio: f64,
    /// cl_extrapolate_amount (seconds)
    pub max_extrapolate: f64,
    /// a stream silent this long is reported held (seconds)
    pub stale_after: f64,
    /// EXTRA_INTERPOLATION_HISTORY_STORED (seconds)
    pub history_extra: f64,
}

impl Default for InterpSettings {
    fn default() -> Self {
        let source = aries_interp::stream::StreamConfig::default();
        Self { oracle_interp_ratio: source.interp_ratio, max_extrapolate: source.max_extrapolate, stale_after: source.stale_after, history_extra: source.history_extra }
    }
}

impl InterpSettings {
    pub fn stream(&self) -> aries_interp::stream::StreamConfig {
        aries_interp::stream::StreamConfig { interp_ratio: self.oracle_interp_ratio, max_extrapolate: self.max_extrapolate, stale_after: self.stale_after, history_extra: self.history_extra }
    }
}
