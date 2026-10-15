//! saved moves / move numbers for the move ack (spec 09 R17): read_move_number.read_saved_moves
//! on its production path (imported, so the `__main__` dumps at RMN:84-113 never run).
//!
//! every read the python makes is kept, the unused diagnostics included: any of them
//! failing fails the call (and so skips that move ack), exactly like the python (Q7).
//! they come out of a few object blocks (pawn, movement, prediction data, the
//! saved-move array, the newest move), so the diagnostics cost no extra reads.

use crate::objects::ENGINE_INDEX;
use crate::world::{HeaderRule, at, object_array};
use crate::{Block, Memory, OracleError};

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
    moves_of(memory, controller, pawn)
}

/// RMN:66-179 from a resolved controller and pawn.
pub fn moves_of(memory: &impl Memory, controller: u64, pawn: u64) -> Result<Option<SavedMoves>, OracleError> {
    if pawn == 0 {
        return Ok(None);
    }
    // read but unused on the production path (RMN:69-79): role, remote role, gates
    let fields = Block::read(memory, pawn, 0x128, 0x520);
    for offset in [0x224, 0x128] {
        fields.u8(memory, offset)?;
    }
    for offset in [0x4A0, 0x510, 0x518] {
        fields.u64(memory, offset)?;
    }
    let gates = Block::read(memory, controller, 1296, 1312);
    gates.u64(memory, 1304)?;
    gates.u64(memory, 1296)?;
    let movement = fields.u64(memory, CHARACTER_MOVEMENT)?;
    // vtable, flags, prediction data and the dropping flag: one read of +0 .. +0x13D9
    let component = Block::read(memory, movement, 0, 0x13D9);
    let movement_vtable = component.u64(memory, 0)?;
    let slots = Block::read(memory, movement_vtable, 2920, 2952);
    slots.u64(memory, 2944)?;
    slots.u64(memory, 2920)?;
    let move_handler_flags = component.u8(memory, 0xC1)?;
    let dropping_moves = component.u8(memory, 0x13D8)? != 0;
    let prediction = component.u64(memory, MOVEMENT_PREDICTION_DATA)?;
    if prediction == 0 {
        return Ok(None);
    }
    let data = Block::read(memory, prediction, PD_SAVED_ARRAY, 0x6C);
    let count = data.u32(memory, PD_SAVED_COUNT)?;
    // overflow comparison value, unused (RMN:120)
    data.u32(memory, 0x68)?;
    let base = data.u64(memory, PD_SAVED_ARRAY)?;
    let last_acked_move = data.u64(memory, PD_LAST_ACKED)?;
    let last_acked = if last_acked_move == 0 { None } else { Some(memory.u32(at(last_acked_move, SAVED_MOVE_NUMBER))?) };
    if count > 4096 || base == 0 {
        return Ok(None);
    }
    let walked = count.min(MAX_WALKED) as u64;
    let array = Block::read(memory, base, 0, SAVED_STRIDE * walked);
    let mut numbers = Vec::new();
    let mut newest_timestamp_bits = None;
    let mut newest_saved_location = None;
    let mut saw_move = false;
    for i in 0..walked {
        let saved_move = array.u64(memory, SAVED_STRIDE * i)?;
        if saved_move == 0 {
            continue;
        }
        // every move is its own allocation: one read each, the first from its
        // vtable, the newest out to its timestamp, direction and location
        let last = i == walked - 1;
        let start = if saw_move { SAVED_MOVE_NUMBER } else { 0 };
        let end = if last { 1080 + 24 } else { SAVED_MOVE_NUMBER + 4 };
        let fields = Block::read(memory, saved_move, start, end);
        if !saw_move {
            // vtable and its prepare slot, logged as rvas only (RMN:136-139)
            let vtable = fields.u64(memory, 0)?;
            memory.u64(at(vtable, 16))?;
            saw_move = true;
        }
        numbers.push(fields.u32(memory, SAVED_MOVE_NUMBER)?);
        if last {
            newest_timestamp_bits = Some(fields.u32(memory, SAVED_MOVE_TIMESTAMP)?);
            // move direction and saved location: follow/track modes only (RMN:143-146)
            fields.get::<24>(memory, 1080)?;
            newest_saved_location = Some(fields.vec3(memory, 592)?);
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
