//! input event cursor reads (spec 09 R11): capture_input_event_cursor.read_pending_input_bounds.
//! the bound must come from a read that starts after the datagram arrived (§4.11, §7.2).

use crate::objects::ENGINE_INDEX;
use crate::world::{HeaderRule, at, require_object_array};
use crate::{Block, Memory, OracleError};

/// pawn +0xB90 input component (CIE:37)
pub const PAWN_INPUT: u64 = 0xB90;
/// input component fields (CIE:53-55)
const INPUT_STOPPED: u64 = 0x12C;
const INPUT_CURSOR: u64 = 0xA84;
const INPUT_PENDING: u64 = 0x1198;
/// the python rejects anything above this as garbage (CIE:56-60)
const MAX_PENDING: u32 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputBounds {
    pub component: u64,
    /// read but unused by the responder
    pub stopped: u8,
    pub cursor: u32,
    pub count: u32,
    /// (cursor + count) & 0xFFFFFFFF, the newest event id the client generated
    pub latest: u32,
}

/// find_input_component (CIE:23-39): world chain, then pawn +0xB90. the controller
/// is not null-checked here (a null one fails the pawn read), the pawn is.
pub fn find_input_component(memory: &impl Memory) -> Result<u64, OracleError> {
    let objects = require_object_array(memory, HeaderRule::Chain)?;
    let engine = objects.object(memory, ENGINE_INDEX)?;
    let viewport = memory.u64(at(engine, 0xBD0))?;
    let world = memory.u64(at(viewport, 0x80))?;
    let controller = crate::world::controller_of(memory, world)?;
    let pawn = memory.u64(at(controller, 0x4B0))?;
    if pawn == 0 { Ok(0) } else { memory.u64(at(pawn, PAWN_INPUT)) }
}

/// read_pending_input_bounds (CIE:42-65); None without an input component.
pub fn read_pending_input_bounds(memory: &impl Memory) -> Result<Option<InputBounds>, OracleError> {
    bounds_of(memory, find_input_component(memory)?)
}

/// the three fields in one read (+0x12C .. +0x119C).
pub fn bounds_of(memory: &impl Memory, component: u64) -> Result<Option<InputBounds>, OracleError> {
    if component == 0 {
        return Ok(None);
    }
    let fields = Block::read(memory, component, INPUT_STOPPED, INPUT_PENDING + 4);
    let stopped = fields.u8(memory, INPUT_STOPPED)?;
    let cursor = fields.u32(memory, INPUT_CURSOR)?;
    let count = fields.u32(memory, INPUT_PENDING)?;
    if count > MAX_PENDING {
        return Err(OracleError::Invalid("input-event count exceeds sane bound"));
    }
    Ok(Some(InputBounds { component, stopped, cursor, count, latest: cursor.wrapping_add(count) }))
}
