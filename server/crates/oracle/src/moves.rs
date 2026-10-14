//! saved moves / move numbers for the move ack (spec 09 R17): read_move_number.read_saved_moves
//! on its production path (imported, so the `__main__` dumps at RMN:84-113 never run).
//!
//! every read the python makes is kept, the unused diagnostics included: any of them
//! failing fails the call (and so skips that move ack), exactly like the python (Q7).

use crate::objects::ENGINE_INDEX;
use crate::world::{HeaderRule, at, object_array};
use crate::{Memory, OracleError};

/// RMN:18-28
const CONTROLLER_PAWN: u64 = 0x4B0;
const CHARACTER_MOVEMENT: u64 = 0x4F8;
const MOVEMENT_PREDICTION_DATA: u64 = 0x7E8;
const PD_SAVED_ARRAY: u64 = 0x20;
const PD_SAVED_COUNT: u64 = 0x28;
const PD_LAST_ACKED: u64 = 0x50;
const SAVED_MOVE_NUMBER: u64 = 0x1C;
const SAVED_MOVE_TIMESTAMP: u64 = 0x3C;
/// saved-move array entries are 16 bytes, FSavedMove* first (RMN:134)
const SAVED_STRIDE: u64 = 16;
/// only the first 256 entries are walked (RMN:133)
const MAX_WALKED: u32 = 256;

#[derive(Debug, Clone, PartialEq)]
pub struct SavedMoves {
    pub count: u32,
    pub oldest: u32,
    /// the last non-null entry among the first min(count, 256) slots (spec 09 §4.17)
    pub newest: u32,
    pub last_acked: Option<u32>,
    pub numbers: Vec<u32>,
    /// f32 bits of slot min(count, 256) - 1, only when that slot is non-null
    pub newest_timestamp_bits: Option<u32>,
    /// movement +0xC1 & 0x20 (diagnostic)
    pub move_handler_gate_open: bool,
    /// movement +0x13D8 (diagnostic)
    pub dropping_moves: bool,
    /// saved move +592 of that same slot (RMN:145-146): the player-position stream's feed
    pub newest_saved_location: Option<[f64; 3]>,
}

impl SavedMoves {
    pub fn newest_timestamp(&self) -> Option<f32> {
        self.newest_timestamp_bits.map(f32::from_bits)
    }
}

/// read_saved_moves (RMN:45-179). Ok(None) wherever the python returns None.
pub fn read_saved_moves(memory: &impl Memory) -> Result<Option<SavedMoves>, OracleError> {
    let Some(objects) = object_array(memory, HeaderRule::Resolve)? else {
        return Ok(None);
    };
    let engine = objects.object(memory, ENGINE_INDEX)?;
    let viewport = memory.u64(at(engine, 0xBD0))?;
    let world = memory.u64(at(viewport, 0x80))?;
    let controller = crate::world::controller_of(memory, world)?;
    let pawn = memory.u64(at(controller, CONTROLLER_PAWN))?;
    if pawn == 0 {
        return Ok(None);
    }
    // read but unused on the production path (RMN:69-79): role, remote role, gates
    for offset in [0x224, 0x128] {
        memory.u8(at(pawn, offset))?;
    }
    for offset in [0x4A0, 0x510, 0x518] {
        memory.u64(at(pawn, offset))?;
    }
    memory.u64(at(controller, 1304))?;
    memory.u64(at(controller, 1296))?;
    let movement = memory.u64(at(pawn, CHARACTER_MOVEMENT))?;
    let movement_vtable = memory.u64(movement)?;
    memory.u64(at(movement_vtable, 2944))?;
    memory.u64(at(movement_vtable, 2920))?;
    let move_handler_flags = memory.u8(at(movement, 0xC1))?;
    let dropping_moves = memory.u8(at(movement, 0x13D8))? != 0;
    let prediction = memory.u64(at(movement, MOVEMENT_PREDICTION_DATA))?;
    if prediction == 0 {
        return Ok(None);
    }
    let count = memory.u32(at(prediction, PD_SAVED_COUNT))?;
    // overflow comparison value, unused (RMN:120)
    memory.u32(at(prediction, 0x68))?;
    let base = memory.u64(at(prediction, PD_SAVED_ARRAY))?;
    let last_acked_move = memory.u64(at(prediction, PD_LAST_ACKED))?;
    let last_acked = if last_acked_move == 0 { None } else { Some(memory.u32(at(last_acked_move, SAVED_MOVE_NUMBER))?) };
    if count > 4096 || base == 0 {
        return Ok(None);
    }
    let walked = count.min(MAX_WALKED) as u64;
    let mut numbers = Vec::new();
    let mut newest_timestamp_bits = None;
    let mut newest_saved_location = None;
    let mut saw_move = false;
    for i in 0..walked {
        let saved_move = memory.u64(at(base, SAVED_STRIDE * i))?;
        if saved_move == 0 {
            continue;
        }
        if !saw_move {
            // vtable and its prepare slot, logged as rvas only (RMN:136-139)
            let vtable = memory.u64(saved_move)?;
            memory.u64(at(vtable, 16))?;
            saw_move = true;
        }
        numbers.push(memory.u32(at(saved_move, SAVED_MOVE_NUMBER))?);
        if i == walked - 1 {
            newest_timestamp_bits = Some(memory.u32(at(saved_move, SAVED_MOVE_TIMESTAMP))?);
            // move direction and saved location: follow/track modes only (RMN:143-146)
            memory.read::<24>(at(saved_move, 1080))?;
            let location = memory.read::<24>(at(saved_move, 592))?;
            newest_saved_location = Some([0, 8, 16].map(|o| f64::from_le_bytes(location[o..o + 8].try_into().expect("8 bytes"))));
        }
    }
    let (Some(&oldest), Some(&newest)) = (numbers.first(), numbers.last()) else {
        return Ok(None);
    };
    Ok(Some(SavedMoves {
        count,
        oldest,
        newest,
        last_acked,
        numbers,
        newest_timestamp_bits,
        move_handler_gate_open: move_handler_flags & 0x20 != 0,
        dropping_moves,
        newest_saved_location,
    }))
}
