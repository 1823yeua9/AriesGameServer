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

pub trait LiveClient {
    /// none when the pawn or its prediction data is not there yet
    fn saved_moves(&mut self) -> Result<Option<SavedMoves>, String>;
    /// read_pending_input_bounds: (cursor + count) of the pawn input component
    fn latest_generated_input_event(&mut self) -> Result<Option<u32>, String>;
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
