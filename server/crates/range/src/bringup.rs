//! the bring-up stage chain (python elif chain 2185-4407): per session-ack tick
//! the FIRST ready stage sends, in chain order. specs: docs/port/03_bringup_a.md,
//! 04_bringup_b.md. conditions are copied from range_udp_handshake.py; times
//! the python reads with .get(source, 0) are Option here, and None counts as
//! "long ago" (python monotonic uptime minus 0 always passes the gate).

use crate::authority::Env;
use crate::send::Reply;

/// every production stage, in chain (elif) order. dead probes are left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    // part A (python 2185-3352)
    WorldSettingsOpen,     // A01 2185 ch3 +0
    GameStateOpen,         // A02 2218 ch4 +0
    ControllerOpen,        // A03 2286 ch5 +0
    PlayerStateOpen,       // A04 2308 ch6 +0
    PlayerInfoOpen,        // A05 2330 ch7 +0
    PawnOpen,              // A06 2352 ch8 +0
    GameStateBool,         // A07 2380 ch4 +1
    ControllerPlayerState, // A08 2409 ch5 +1
    ControllerPawn,        // A09 2436 ch5 +2
    PlayerStatePlayerInfo, // A10 2462 ch6 +1
    PlayerStateCharacter,  // A11 2492 ch6 +2,+3 (two frames)
    PlayerStateDesiredClass, // A12 2529 ch6 +4
    PlayerInfoController,  // A13 2570 ch7 +1
    PawnPlayerState,       // A14 2607 ch8 +1
    PawnController,        // A15 2635 ch8 +2
    PawnRole,              // A16 2661 ch8 +3
    InventoryOpen,         // A17 2689 ch8 +4
    ItemSlotCreate,        // A18 2748 ch8 +5
    ItemSlotLink,          // A19 2826 ch8 +6
    ItemSlotScale,         // A20 2873 ch8 +7.. (pairs, slots 1..15)
    PistolOpen,            // A21 2969 ch9 +0
    PistolLink,            // A22 3048 ch8 +37
    PistolOwner,           // A23 3094 ch9 +1
    PistolRole,            // A24 3131 ch9 +2
    PistolStateMachine,    // A25 3174 ch9 +3
    PistolInactiveState,   // A26 3227 ch9 +4
    InventoryCorrection,   // A27 3290 ch8 +38.. (x5)
    // part B (python 3353-4407)
    ClientRestart,         // B1 3353 ch5 +3
    GameStatePhase,        // B2 3381 ch4 +2
    TeamStateOpen,         // B3 3409 ch10/11 +0 (x2)
    TeamStateArray,        // B4 3440 ch4 +3
    TeamStateLink,         // B5 3471 ch10/11 +1 (x2)
    TeamStatePlayer,       // B6 3503 ch6 +5
    BombTeamOpen,          // B7 3532 ch6 +6
    BombTeamNumber,        // B8 3587 ch6 +7
    AccessibleCharacters,  // B9 3635 ch6 +8
    PlayerUniqueId,        // B10 3670 ch6 +9
    RangeMoney,            // B11 3707 ch6 +10
    PawnOwner,             // B12 3735 ch8 +43
    ClientSetHud,          // B13 3769 ch5 +5
    MatchState,            // B14 3861 ch4 +4
    GameplayStart,         // B15 3898 ch4 +5
    PlayerArray,           // B16 3933 ch4 +6
    RangeShop,             // B17 3968 ch4 +7
    StartingEquippable,    // B18 4003 ch12+i +0,+1, ch8 +44+i (x7)
    MeleeRole,             // B19 4102 ch13 +2
    MeleeStateMachine,     // B20 4133 ch13 +3
    MeleeInactiveState,    // B21 4163 ch13 +4
    ViewTarget,            // B22 4265 ch5 +4
}

pub const STAGE_COUNT: usize = Stage::ViewTarget as usize + 1;
/// item slots 0..15 (python item_slot_scale_index < 16)
pub const ITEM_SLOT_COUNT: u32 = 16;
pub const INVENTORY_CORRECTIONS: u32 = 5;
pub const TEAM_STATES: u32 = 2;
/// python len(wushu_starting_equippables)
pub const STARTING_EQUIPPABLES: u32 = 7;

/// when each stage last sent, plus the multi-step counters.
#[derive(Debug, Clone)]
pub struct BringupState {
    sent: [Option<f64>; STAGE_COUNT],
    /// python item_slot_scale_index (A19 sets 1, A20 increments to 16)
    pub item_slot_scale_index: u32,
    /// python team_state_open_index / team_state_link_index (0..2)
    pub team_state_open_index: u32,
    pub team_state_link_index: u32,
    /// python starting_equippable_index (0..7)
    pub starting_equippable_index: u32,
    /// live-resolved ClientSetHUD (field, class max) for the range hud rebuild
    pub hud_rpc: Option<(u32, u32)>,
}

impl Default for BringupState {
    fn default() -> Self {
        Self { sent: [None; STAGE_COUNT], item_slot_scale_index: 0, team_state_open_index: 0, team_state_link_index: 0, starting_equippable_index: 0, hud_rpc: None }
    }
}

/// a gate on an optional time: none = long ago, so it passes.
fn elapsed(now: f64, at: Option<f64>, gate: f64) -> bool {
    at.is_none_or(|t| now - t >= gate)
}

fn latest(times: &[Option<f64>]) -> Option<f64> {
    times.iter().flatten().copied().reduce(f64::max)
}

impl BringupState {
    pub fn sent(&self, stage: Stage) -> bool {
        self.sent[stage as usize].is_some()
    }

    pub fn time(&self, stage: Stage) -> Option<f64> {
        self.sent[stage as usize]
    }

    /// stamps a stage as sent at `now` (python *_sent_sources.add + *_sent_times[source] = now).
    pub fn record(&mut self, stage: Stage, now: f64) {
        self.sent[stage as usize] = Some(now);
    }

    /// python controller_link_stage: 1 after A08, 2 after A09.
    pub fn controller_link_stage(&self) -> u8 {
        if self.sent(Stage::ControllerPawn) { 2 } else if self.sent(Stage::ControllerPlayerState) { 1 } else { 0 }
    }

    /// python pawn_link_stage: 1 after A14, 2 after A15.
    pub fn pawn_link_stage(&self) -> u8 {
        if self.sent(Stage::PawnController) { 2 } else if self.sent(Stage::PawnPlayerState) { 1 } else { 0 }
    }

    /// python controller_link_sent_times: the latest of A08/A09.
    fn controller_link_time(&self) -> Option<f64> {
        latest(&[self.time(Stage::ControllerPlayerState), self.time(Stage::ControllerPawn)])
    }

    /// python pawn_link_sent_times: the latest of A14/A15.
    fn pawn_link_time(&self) -> Option<f64> {
        latest(&[self.time(Stage::PawnPlayerState), self.time(Stage::PawnController)])
    }

    /// python item_slot_scale_last_time: set by A19, then every A20 pair.
    fn item_slot_scale_last_time(&self) -> Option<f64> {
        latest(&[self.time(Stage::ItemSlotLink), self.time(Stage::ItemSlotScale)])
    }

    /// the first ready stage in chain order. `corrections` is the shared
    /// python auth_correct_rep_index (bring-up sends 1..5; purchases and the
    /// input-correction responder raise it later).
    pub fn next_stage(&self, now: f64, corrections: u32) -> Option<Stage> {
        use Stage::*;
        let t = |s: Stage| self.time(s);
        let sent = |s: Stage| self.sent(s);
        let after = |s: Stage, gate: f64| sent(s) && elapsed(now, t(s), gate);

        // A01-A06 actor opens, 2 s apart
        if !sent(WorldSettingsOpen) {
            return Some(WorldSettingsOpen);
        }
        if !sent(GameStateOpen) && after(WorldSettingsOpen, 2.0) {
            return Some(GameStateOpen);
        }
        if !sent(ControllerOpen) && after(GameStateOpen, 2.0) {
            return Some(ControllerOpen);
        }
        if !sent(PlayerStateOpen) && after(ControllerOpen, 2.0) {
            return Some(PlayerStateOpen);
        }
        if !sent(PlayerInfoOpen) && after(PlayerStateOpen, 2.0) {
            return Some(PlayerInfoOpen);
        }
        if !sent(PawnOpen) && after(PlayerInfoOpen, 2.0) {
            return Some(PawnOpen);
        }
        if !sent(GameStateBool) && after(PawnOpen, 3.0) {
            return Some(GameStateBool);
        }
        // A08/A09 controller links
        if self.controller_link_stage() == 0 && after(GameStateBool, 3.0) {
            return Some(ControllerPlayerState);
        }
        if self.controller_link_stage() == 1 && elapsed(now, self.controller_link_time(), 3.0) {
            return Some(ControllerPawn);
        }
        // A10-A13 player state links
        if self.controller_link_stage() == 2 && !sent(PlayerStatePlayerInfo) && elapsed(now, self.controller_link_time(), 3.0) {
            return Some(PlayerStatePlayerInfo);
        }
        if !sent(PlayerStateCharacter) && after(PlayerStatePlayerInfo, 0.5) {
            return Some(PlayerStateCharacter);
        }
        if !sent(PlayerStateDesiredClass) && after(PlayerStateCharacter, 0.5) {
            return Some(PlayerStateDesiredClass);
        }
        if sent(PlayerStateDesiredClass) && sent(PlayerInfoOpen) && !sent(PlayerInfoController) && elapsed(now, t(PlayerStateDesiredClass), 0.5) {
            return Some(PlayerInfoController);
        }
        // A14-A16 pawn links and role
        if self.controller_link_stage() == 2 && sent(PlayerStateDesiredClass) && self.pawn_link_stage() == 0 && elapsed(now, t(PlayerStateDesiredClass), 1.0) {
            return Some(PawnPlayerState);
        }
        if self.pawn_link_stage() == 1 && elapsed(now, self.pawn_link_time(), 3.0) {
            return Some(PawnController);
        }
        if self.pawn_link_stage() == 2 && !sent(PawnRole) && elapsed(now, self.pawn_link_time(), 1.0) {
            return Some(PawnRole);
        }
        // A17-A20 inventory and item slots
        if !sent(InventoryOpen) && after(PawnRole, 1.5) {
            return Some(InventoryOpen);
        }
        if !sent(ItemSlotCreate) && after(InventoryOpen, 2.0) {
            return Some(ItemSlotCreate);
        }
        if !sent(ItemSlotLink) && after(ItemSlotCreate, 1.5) {
            return Some(ItemSlotLink);
        }
        if sent(ItemSlotLink) && self.item_slot_scale_index < ITEM_SLOT_COUNT && elapsed(now, self.item_slot_scale_last_time(), 1.0) {
            return Some(ItemSlotScale);
        }
        // A21-A26 base pistol
        if sent(ItemSlotLink) && self.item_slot_scale_index >= ITEM_SLOT_COUNT && !sent(PistolOpen) && elapsed(now, self.item_slot_scale_last_time(), 2.0) {
            return Some(PistolOpen);
        }
        if !sent(PistolLink) && after(PistolOpen, 2.0) {
            return Some(PistolLink);
        }
        if !sent(PistolOwner) && after(PistolLink, 1.0) {
            return Some(PistolOwner);
        }
        if !sent(PistolRole) && after(PistolOwner, 1.0) {
            return Some(PistolRole);
        }
        if !sent(PistolStateMachine) && after(PistolRole, 1.5) {
            return Some(PistolStateMachine);
        }
        if !sent(PistolInactiveState) && after(PistolStateMachine, 1.0) {
            return Some(PistolInactiveState);
        }
        // A27 inventory corrections 1..5, 2 s after the latest pistol stage or correction
        if sent(PistolInactiveState)
            && corrections < INVENTORY_CORRECTIONS
            && elapsed(now, latest(&[t(PistolInactiveState), t(PistolStateMachine), t(PistolRole), t(PistolOwner), t(InventoryCorrection)]), 2.0)
        {
            return Some(InventoryCorrection);
        }

        // B1 ClientRestart, 3 s after the pawn role
        if self.pawn_link_stage() == 2 && sent(PawnRole) && !sent(ClientRestart) && elapsed(now, t(PawnRole), 3.0) {
            return Some(ClientRestart);
        }
        if !sent(GameStatePhase) && after(ClientRestart, 3.0) {
            return Some(GameStatePhase);
        }
        // B3-B6 team states
        if sent(GameStatePhase) && sent(PistolOpen) && self.team_state_open_index < TEAM_STATES && elapsed(now, latest(&[t(PistolOpen), t(TeamStateOpen)]), 1.0) {
            return Some(TeamStateOpen);
        }
        if self.team_state_open_index == TEAM_STATES && !sent(TeamStateArray) && elapsed(now, t(TeamStateOpen), 2.0) {
            return Some(TeamStateArray);
        }
        if sent(TeamStateArray) && self.team_state_link_index < TEAM_STATES && elapsed(now, latest(&[t(TeamStateArray), t(TeamStateLink)]), 1.0) {
            return Some(TeamStateLink);
        }
        if self.team_state_link_index == TEAM_STATES && !sent(TeamStatePlayer) && elapsed(now, t(TeamStateLink), 2.0) {
            return Some(TeamStatePlayer);
        }
        // B7-B11 player state: bombteam, characters, unique id, money
        if sent(TeamStatePlayer) && !sent(BombTeamOpen) && elapsed(now, t(TeamStatePlayer), 1.0) {
            return Some(BombTeamOpen);
        }
        if sent(BombTeamOpen) && !sent(BombTeamNumber) && elapsed(now, t(BombTeamOpen), 1.0) {
            return Some(BombTeamNumber);
        }
        if !sent(AccessibleCharacters) && after(BombTeamNumber, 0.5) {
            return Some(AccessibleCharacters);
        }
        if !sent(PlayerUniqueId) && after(AccessibleCharacters, 0.5) {
            return Some(PlayerUniqueId);
        }
        if sent(PlayerUniqueId) && !sent(RangeMoney) && elapsed(now, t(PlayerUniqueId), 0.5) {
            return Some(RangeMoney);
        }
        // B12 pawn owner: no time gate
        if sent(PlayerUniqueId) && corrections >= INVENTORY_CORRECTIONS && !sent(PawnOwner) {
            return Some(PawnOwner);
        }
        // B13 ClientSetHUD needs the view target (B22) too
        if sent(AccessibleCharacters) && sent(PlayerUniqueId) && sent(ViewTarget) && !sent(ClientSetHud) && elapsed(now, t(AccessibleCharacters), 0.5) {
            return Some(ClientSetHud);
        }
        // B14-B17 game state completion
        if sent(ClientSetHud) && sent(GameStatePhase) && !sent(MatchState) && elapsed(now, t(ClientSetHud), 4.0) {
            return Some(MatchState);
        }
        if !sent(GameplayStart) && after(MatchState, 2.0) {
            return Some(GameplayStart);
        }
        if !sent(PlayerArray) && after(GameplayStart, 2.0) {
            return Some(PlayerArray);
        }
        if !sent(RangeShop) && after(PlayerArray, 1.0) {
            return Some(RangeShop);
        }
        // B18 starting equippables x7, 0.5 s apart
        if sent(PawnOwner) && self.team_state_open_index >= TEAM_STATES && self.starting_equippable_index < STARTING_EQUIPPABLES && elapsed(now, t(StartingEquippable), 0.5) {
            return Some(StartingEquippable);
        }
        // B19-B21 knife
        if self.starting_equippable_index >= STARTING_EQUIPPABLES && !sent(MeleeRole) && elapsed(now, t(StartingEquippable), 0.5) {
            return Some(MeleeRole);
        }
        if !sent(MeleeStateMachine) && after(MeleeRole, 0.5) {
            return Some(MeleeStateMachine);
        }
        if !sent(MeleeInactiveState) && after(MeleeStateMachine, 0.5) {
            return Some(MeleeInactiveState);
        }
        // B22 view target, 1 s after ClientRestart (late in the chain: earlier ready stages win)
        if !sent(ViewTarget) && after(ClientRestart, 1.0) {
            return Some(ViewTarget);
        }
        None
    }

    /// bookkeeping after a stage sent (counters; the stage time is stamped too).
    pub fn advance(&mut self, stage: Stage, now: f64) {
        match stage {
            Stage::ItemSlotLink => self.item_slot_scale_index = 1,
            Stage::ItemSlotScale => self.item_slot_scale_index += 1,
            Stage::TeamStateOpen => self.team_state_open_index += 1,
            Stage::TeamStateLink => self.team_state_link_index += 1,
            Stage::StartingEquippable => self.starting_equippable_index += 1,
            _ => {}
        }
        self.record(stage, now);
    }
}

/// one fixed-offset reliable send of a stage.
struct Frame {
    channel: u32,
    offset: u32,
    open: bool,
    exports: bool,
    body: aries_rep::RepResult<aries_bits::Bits>,
}

fn frame(channel: u32, offset: u32, open: bool, exports: bool, body: aries_rep::RepResult<aries_bits::Bits>) -> Frame {
    Frame { channel, offset, open, exports, body }
}

/// the frames a stage sends and its python event key. counters (slot index,
/// team index, equippable index, correction index) are read here and advanced
/// by advance() after the send.
fn stage_frames(stage: Stage, reply: &mut Reply, env: &mut Env) -> (Vec<Frame>, &'static str) {
    use aries_rep::setup::*;
    let config = env.config;
    let state = &reply.conn.bringup;
    match stage {
        Stage::WorldSettingsOpen => (vec![frame(3, 0, true, true, world_settings_open())], "sent_actor_open_probe_length"),
        Stage::GameStateOpen => (vec![frame(4, 0, true, true, game_state_open())], "sent_game_state_probe_length"),
        Stage::ControllerOpen => (vec![frame(5, 0, true, true, controller_open())], "sent_player_controller_probe_length"),
        Stage::PlayerStateOpen => (vec![frame(6, 0, true, true, player_state_open())], "sent_player_state_probe_length"),
        Stage::PlayerInfoOpen => (vec![frame(7, 0, true, true, player_info_open())], "sent_player_info_probe_length"),
        Stage::PawnOpen => (vec![frame(8, 0, true, true, pawn_open())], "sent_wushu_pawn_probe_length"),
        Stage::GameStateBool => (vec![frame(4, 1, false, false, game_state_bool_h18_body(GAME_STATE_BOOL_VALUE))], "sent_game_state_bool_probe_length"),
        Stage::ControllerPlayerState => (vec![frame(5, 1, false, false, controller_player_state_link())], "sent_controller_player_state_link_length"),
        Stage::ControllerPawn => (vec![frame(5, 2, false, false, controller_pawn_link())], "sent_controller_pawn_link_length"),
        Stage::PlayerStatePlayerInfo => (vec![frame(6, 1, false, false, player_state_player_info_link())], "sent_player_state_info_link_length"),
        Stage::PlayerStateCharacter => (
            vec![frame(6, 2, false, false, player_state_spawned_character_link()), frame(6, 3, false, false, player_state_possessed_character_link())],
            "sent_player_state_spawned_character_link_length",
        ),
        Stage::PlayerStateDesiredClass => (vec![frame(6, 4, false, false, player_state_desired_class_body(config.desired_class_guid))], "sent_player_state_desired_class_link_length"),
        Stage::PlayerInfoController => (vec![frame(7, 1, false, false, player_info_controller_link())], "sent_player_info_controller_link_length"),
        Stage::PawnPlayerState => (vec![frame(8, 1, false, false, pawn_player_state_link())], "sent_pawn_player_state_link_length"),
        Stage::PawnController => (vec![frame(8, 2, false, false, pawn_controller_link())], "sent_pawn_controller_link_length"),
        Stage::PawnRole => (vec![frame(8, 3, false, false, pawn_remote_role_body())], "sent_pawn_autonomous_role_length"),
        Stage::InventoryOpen => (vec![frame(8, 4, false, true, inventory_open_body())], "sent_inventory_open_length"),
        Stage::ItemSlotCreate => (vec![frame(8, 5, false, true, item_slot_create_body(0))], "sent_item_slot_create_length"),
        Stage::ItemSlotLink => (vec![frame(8, 6, false, false, item_slot_link_body(0))], "sent_item_slot_link_length"),
        Stage::ItemSlotScale => {
            let slot = state.item_slot_scale_index;
            let offset = 7 + 2 * (slot - 1);
            (vec![frame(8, offset, false, true, item_slot_create_body(slot)), frame(8, offset + 1, false, false, item_slot_link_body(slot))], "sent_item_slot_scale_pair_length")
        }
        Stage::PistolOpen => (vec![frame(9, 0, true, true, crate::hooks::pistol_open_body(env, &mut reply.event))], "sent_base_pistol_open_length"),
        Stage::PistolLink => (vec![frame(8, 37, false, false, pistol_slot_contents_body())], "sent_base_pistol_link_length"),
        Stage::PistolOwner => (vec![frame(9, 1, false, false, pistol_owner_body())], "sent_base_pistol_owner_length"),
        Stage::PistolRole => (vec![frame(9, 2, false, false, pistol_remote_role_body())], "sent_base_pistol_role_length"),
        Stage::PistolStateMachine => (vec![frame(9, 3, false, true, pistol_state_machine_open_body())], "sent_equippable_sm_open_length"),
        Stage::PistolInactiveState => (vec![frame(9, 4, false, true, pistol_inactive_state_body())], "sent_inactive_state_link_length"),
        Stage::InventoryCorrection => {
            let index = reply.conn.correction_index + 1;
            (vec![frame(8, 37 + index, false, false, pistol_correction_body(index))], "sent_auth_correct_rep_length")
        }
        Stage::ClientRestart => (vec![frame(5, 3, false, false, client_restart_body())], "sent_client_restart_rpc_length"),
        Stage::GameStatePhase => (vec![frame(4, 2, false, false, game_state_phase_body(config.game_state_phase))], "sent_game_state_phase_probe_length"),
        Stage::TeamStateOpen => {
            let team = state.team_state_open_index as usize;
            (vec![frame(10 + team as u32, 0, true, true, team_state_open_body(team))], "sent_team_state_open_length")
        }
        Stage::TeamStateArray => (vec![frame(4, 3, false, false, team_state_array_body())], "sent_team_state_array_length"),
        Stage::TeamStateLink => {
            let team = state.team_state_link_index as usize;
            (vec![frame(10 + team as u32, 1, false, true, team_state_link_body(team))], "sent_team_state_link_length")
        }
        Stage::TeamStatePlayer => (vec![frame(6, 5, false, false, player_state_team_body())], "sent_team_state_player_length"),
        Stage::BombTeamOpen => (vec![frame(6, 6, false, true, bomb_team_open_body())], "sent_bomb_team_open_length"),
        Stage::BombTeamNumber => (vec![frame(6, 7, false, false, bomb_team_number_body(config.player_team as i64, BOMB_TEAM_NUMBER_BITS))], "sent_bomb_team_number_length"),
        Stage::AccessibleCharacters => (vec![frame(6, 8, false, false, accessible_character_list_body(config.desired_class_guid))], "sent_accessible_character_length"),
        Stage::PlayerUniqueId => (vec![frame(6, 9, false, false, player_unique_id_body(&config.player_id))], "sent_player_unique_id_length"),
        Stage::RangeMoney => (vec![frame(6, 10, false, true, range_money_body())], "sent_range_money_length"),
        Stage::PawnOwner => (vec![frame(8, 43, false, false, pawn_owner_body())], "sent_pawn_owner_length"),
        Stage::ClientSetHud => {
            // resolved by run_stage_chain just before (python hud_rpc_cache)
            let (field, max) = state.hud_rpc.expect("resolved before the frames");
            (vec![frame(5, 5, false, true, hud_set_body(field, max))], "sent_hud_set_length")
        }
        Stage::MatchState => (vec![frame(4, 4, false, false, match_state_body())], "sent_match_state_length"),
        Stage::GameplayStart => (vec![frame(4, 5, false, false, gameplay_start_body())], "sent_gameplay_start_length"),
        Stage::PlayerArray => (vec![frame(4, 6, false, false, player_array_body())], "sent_player_array_length"),
        Stage::RangeShop => (vec![frame(4, 7, false, false, range_shop_body())], "sent_range_shop_settings_length"),
        Stage::StartingEquippable => {
            let index = state.starting_equippable_index as usize;
            let channel = aries_rep::actors::STARTING_EQUIPPABLE_FIRST_CHANNEL + index as u32;
            (
                vec![
                    frame(channel, 0, true, true, crate::hooks::starting_equippable_open_body(index, env, &mut reply.event)),
                    frame(channel, 1, false, false, starting_equippable_owner_body(index)),
                    frame(8, 44 + index as u32, false, false, starting_equippable_contents_body(index)),
                ],
                "sent_starting_equippable_length",
            )
        }
        Stage::MeleeRole => (vec![frame(13, 2, false, false, knife_remote_role_body())], "sent_melee_autonomous_role_length"),
        Stage::MeleeStateMachine => (vec![frame(13, 3, false, true, knife_state_machine_open_body())], "sent_melee_state_machine_open_length"),
        Stage::MeleeInactiveState => (vec![frame(13, 4, false, true, knife_inactive_state_body())], "sent_melee_inactive_state_length"),
        Stage::ViewTarget => (vec![frame(5, 4, false, true, viewtarget_body())], "sent_viewtarget_length"),
    }
}

/// runs at most one stage (the first ready one in chain order). returns the
/// stage when it sent. a stage whose body fails to build is logged and skipped
/// this tick (the python would crash the responder; we keep the session).
pub fn run_stage_chain(reply: &mut Reply, env: &mut Env) -> Option<Stage> {
    let stage = reply.conn.bringup.next_stage(reply.now, reply.conn.correction_index)?;
    if stage == Stage::ClientSetHud {
        reply.conn.bringup.hud_rpc = Some(crate::hooks::client_set_hud_index(env, &mut reply.event));
    }
    let (frames, key) = stage_frames(stage, reply, env);
    let mut total = 0usize;
    for frame in frames {
        let body = match frame.body {
            Ok(body) => body,
            Err(error) => {
                reply.event.set("stage_error", format!("{stage:?}: {error}"));
                return None;
            }
        };
        match reply.send_fixed(frame.channel, frame.offset, &body, frame.exports, frame.open) {
            Ok(length) => total += length,
            Err(error) => {
                reply.event.set("stage_error", format!("{stage:?}: {error}"));
                return None;
            }
        }
    }
    reply.event.set(key, total);
    if stage == Stage::InventoryCorrection {
        reply.conn.correction_index += 1;
        reply.event.set("auth_correct_rep_index", reply.conn.correction_index);
    }
    let now = reply.now;
    if stage == Stage::PawnOpen {
        // the pawn opens at the spawn point: start the position track there
        let [x, y, z] = env.config.spawn_location;
        reply.conn.player.var.reset(now, aries_interp::value::Vec3 { x, y, z });
    }
    reply.conn.bringup.advance(stage, now);
    log::info!(target: "bringup", "{stage:?} ({key} {total})");
    Some(stage)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// drive the chain with session-ack ticks every `tick` seconds; returns (stage, time) in firing order.
    fn simulate(tick: f64, until: f64) -> Vec<(Stage, f64)> {
        let mut state = BringupState::default();
        let mut corrections = 0u32;
        let mut fired = Vec::new();
        let mut now = 0.0;
        while now <= until {
            if let Some(stage) = state.next_stage(now, corrections) {
                if stage == Stage::InventoryCorrection {
                    corrections += 1;
                }
                state.advance(stage, now);
                fired.push((stage, now));
            }
            now = ((now + tick) * 1e6).round() / 1e6;
        }
        fired
    }

    #[test]
    fn part_a_fires_in_order_at_nominal_times() {
        let fired = simulate(0.05, 200.0);
        let first = |s: Stage| fired.iter().find(|(x, _)| *x == s).map(|(_, t)| *t).unwrap();
        // nominal times from 03_bringup_a.md section 6 (A01 at 0)
        let nominal = [
            (Stage::GameStateOpen, 2.0), (Stage::ControllerOpen, 4.0), (Stage::PlayerStateOpen, 6.0), (Stage::PlayerInfoOpen, 8.0),
            (Stage::PawnOpen, 10.0), (Stage::GameStateBool, 13.0), (Stage::ControllerPlayerState, 16.0), (Stage::ControllerPawn, 19.0),
            (Stage::PlayerStatePlayerInfo, 22.0), (Stage::PlayerStateCharacter, 22.5), (Stage::PlayerStateDesiredClass, 23.0),
            (Stage::PlayerInfoController, 23.5), (Stage::PawnPlayerState, 24.0), (Stage::PawnController, 27.0), (Stage::PawnRole, 28.0),
            (Stage::InventoryOpen, 29.5), (Stage::ItemSlotCreate, 31.5), (Stage::ItemSlotLink, 33.0),
        ];
        for (stage, t) in nominal {
            assert!((first(stage) - t).abs() < 0.11, "{stage:?} at {} (nominal {t})", first(stage));
        }
        // 15 scale pairs, then the pistol chain
        assert_eq!(fired.iter().filter(|(s, _)| *s == Stage::ItemSlotScale).count(), 15);
        assert_eq!(fired.iter().filter(|(s, _)| *s == Stage::InventoryCorrection).count(), 5);
        assert_eq!(fired.iter().filter(|(s, _)| *s == Stage::TeamStateOpen).count(), 2);
        assert_eq!(fired.iter().filter(|(s, _)| *s == Stage::StartingEquippable).count(), 7);
        // client restart 3 s after the pawn role; part B runs in the gaps of part A
        assert!((first(Stage::ClientRestart) - (first(Stage::PawnRole) + 3.0)).abs() < 0.11);
        assert!(first(Stage::ViewTarget) < first(Stage::ClientSetHud));
        assert!(first(Stage::PawnOwner) > fired.iter().filter(|(s, _)| *s == Stage::InventoryCorrection).map(|(_, t)| *t).fold(0.0, f64::max));
        // every stage fires exactly once, except the multi-step ones
        for stage in [Stage::WorldSettingsOpen, Stage::PistolOpen, Stage::ClientSetHud, Stage::MeleeInactiveState, Stage::RangeShop] {
            assert_eq!(fired.iter().filter(|(s, _)| *s == stage).count(), 1, "{stage:?}");
        }
        assert_eq!(fired.last().unwrap().0, Stage::MeleeInactiveState);
    }

    #[test]
    fn one_stage_per_tick_and_earlier_stages_win() {
        // with ticks every 0.125 s, a stage that becomes ready behind another waits a tick
        let fired = simulate(0.125, 200.0);
        let mut times: Vec<f64> = fired.iter().map(|(_, t)| *t).collect();
        times.dedup();
        assert_eq!(times.len(), fired.len(), "never two stages in one tick");
    }
}

#[cfg(test)]
mod observed {
    use super::*;

    /// the live log interleaves part A and B differently from nominal timing
    /// (client hitches pause the session ticks), but every channel's fixed
    /// reliable offsets must still go out in order. check that under jittered
    /// tick cadences from 30 ms to 250 ms.
    #[test]
    fn per_channel_order_holds_under_any_cadence() {
        use Stage::*;
        let channels: [&[Stage]; 6] = [
            &[GameStateOpen, GameStateBool, GameStatePhase, TeamStateArray, MatchState, GameplayStart, PlayerArray, RangeShop],
            &[ControllerOpen, ControllerPlayerState, ControllerPawn, ClientRestart, ViewTarget, ClientSetHud],
            &[PlayerStateOpen, PlayerStatePlayerInfo, PlayerStateCharacter, PlayerStateDesiredClass, TeamStatePlayer, BombTeamOpen, BombTeamNumber, AccessibleCharacters, PlayerUniqueId, RangeMoney],
            &[PawnOpen, PawnPlayerState, PawnController, PawnRole, InventoryOpen, ItemSlotCreate, ItemSlotLink, ItemSlotScale, PistolLink, InventoryCorrection, PawnOwner, StartingEquippable],
            &[PistolOpen, PistolOwner, PistolRole, PistolStateMachine, PistolInactiveState],
            &[StartingEquippable, MeleeRole, MeleeStateMachine, MeleeInactiveState],
        ];
        for (cadence, seed) in [(0.03, 1u64), (0.0625, 2), (0.125, 3), (0.25, 4), (0.11, 5)] {
            let mut rng = aries_world::PyRandom::new(seed);
            let mut state = BringupState::default();
            let mut corrections = 0u32;
            let mut fired: Vec<Stage> = Vec::new();
            let mut now = 0.0f64;
            while now < 300.0 {
                if let Some(stage) = state.next_stage(now, corrections) {
                    if stage == InventoryCorrection {
                        corrections += 1;
                    }
                    state.advance(stage, now);
                    fired.push(stage);
                }
                // jittered arrival plus the occasional 2.5 s hitch
                now += cadence * rng.uniform(0.6, 1.4) + if rng.random() < 0.01 { 2.5 } else { 0.0 };
            }
            assert_eq!(fired.iter().collect::<std::collections::HashSet<_>>().len(), STAGE_COUNT, "cadence {cadence}: every stage fires");
            for order in channels {
                let positions: Vec<usize> = order
                    .iter()
                    .map(|stage| fired.iter().rposition(|s| s == stage).unwrap_or_else(|| panic!("{stage:?} never fired")))
                    .collect();
                // each stage's last firing is before the next stage's first firing
                for pair in order.windows(2) {
                    let last_a = fired.iter().rposition(|s| *s == pair[0]).unwrap();
                    let first_b = fired.iter().position(|s| *s == pair[1]).unwrap();
                    assert!(last_a < first_b || pair[0] == pair[1], "cadence {cadence}: {:?} after {:?}", pair[1], pair[0]);
                }
                assert!(!positions.is_empty());
            }
        }
    }
}
