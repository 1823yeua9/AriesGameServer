//! what the range needs from the running client. the oracle crate's
//! LiveClient (ReadProcessMemory now, the internal dll later) is adapted to
//! this; tests use scripted values. every read may fail: the python treated a
//! failed read as "no data" and sent nothing for that step.

/// read_saved_moves (read_move_number.py:45): only newest and count drive sends.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SavedMoves {
    /// the newest non-null saved move's number among the first min(count, 256) slots
    pub newest: u32,
    /// raw saved-move count (the placement-correction gate uses count > 0)
    pub count: u32,
    pub oldest: Option<u32>,
    pub last_acked: Option<u32>,
    pub timestamp: Option<f32>,
}

/// read_input_replication (read_equippable_state.py:247-268).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InputReplication {
    /// input component +0x12C: replication stopped (>256 unacked or a stale frame)
    pub stopped: bool,
    /// input component +0x130: the correction index it waits for
    pub requested_correction: u32,
    /// inventory +0x2F0
    pub inventory_correction: u32,
    /// class of the desired (+0x260) or current (+0x278) equippable
    pub current_equippable_class: Option<String>,
}

/// the bottom hud widget and load-in cinematic flag (hud refresh, python 1999-2004).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HudWidget {
    pub collapsed: bool,
    pub cinematic: bool,
}

pub trait LiveClient {
    /// none when the pawn or its prediction data is not there yet
    fn saved_moves(&mut self) -> Result<Option<SavedMoves>, String>;
    /// read_pending_input_bounds: (cursor + count) of the pawn input component
    fn latest_generated_input_event(&mut self) -> Result<Option<u32>, String>;
    fn input_replication(&mut self) -> Result<Option<InputReplication>, String> {
        Ok(None)
    }
    /// i32 [pawn + 0x14F4]
    fn ultimate_points_requirement(&mut self) -> Result<Option<i32>, String> {
        Ok(None)
    }
    fn hud_widget(&mut self) -> Result<Option<HudWidget>, String> {
        Ok(None)
    }
}

/// no client to read (tests, or the oracle failed to attach): every read is "no data".
pub struct NoLiveClient;

impl LiveClient for NoLiveClient {
    fn saved_moves(&mut self) -> Result<Option<SavedMoves>, String> {
        Ok(None)
    }

    fn latest_generated_input_event(&mut self) -> Result<Option<u32>, String> {
        Ok(None)
    }
}
