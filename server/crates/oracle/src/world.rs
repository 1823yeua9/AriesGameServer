//! engine -> world -> controller -> pawn chain, camera pov (spec 09 §3.2-3.3, R9).
//! every walk re-decodes GObjects like the python (RES:30-63, RRI:60-73).

use crate::objects::{ENGINE_INDEX, GObjects, OWNER_RVA, decode_array_pointer};
use crate::{Memory, OracleError};

/// base + offset for addresses that came out of client memory. python ints never
/// wrap, so an overflowing address must stay invalid (u64::MAX fails every read).
pub(crate) fn at(base: u64, offset: u64) -> u64 {
    base.saturating_add(offset)
}

/// the decoded GObjects header, `struct "<Q8xIIII"` (RWS:127-129).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArrayHeader {
    pub chunk_table: u64,
    pub max_elements: u32,
    pub num_elements: u32,
    pub max_chunks: u32,
    pub num_chunks: u32,
}

/// the header check a walk applies. each python caller kept its own (spec 09 §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderRule {
    /// read_fname_pool.derive_name_key (FNP:48): 0x107E2 < elements <= 0x400000, 1 <= chunks <= 0x14; raises.
    /// the tag scan (GTN:45-46) has no check of its own but always runs right after
    /// a fresh derive_name_key on the same header (H:4054-4057), so it uses this one.
    NameKey,
    /// resolve_rpc_index.resolve (RRI:64), read_saved_moves (RMN:57): 1 <= chunks <= 0x14,
    /// 0x800 < elements <= 0x400000; returns None.
    Resolve,
    /// find_pawn / find_world (RES:35, 53), find_input_component (CIE:28): 1 <= chunks <= 256; raises.
    Chain,
}

impl HeaderRule {
    pub fn accepts(self, header: &ArrayHeader) -> bool {
        let (elements, chunks) = (header.num_elements, header.num_chunks);
        match self {
            Self::NameKey => ENGINE_INDEX < elements && elements <= 0x40_0000 && (1..=0x14).contains(&chunks),
            Self::Resolve => (1..=0x14).contains(&chunks) && 0x800 < elements && elements <= 0x40_0000,
            Self::Chain => (1..=256).contains(&chunks),
        }
    }
}

/// owner block (image + 0xD3E7140, 0x80 bytes) -> decoded array -> 0x20-byte header.
pub fn read_array_header(memory: &impl Memory) -> Result<ArrayHeader, OracleError> {
    let owner: [u8; 0x80] = memory.read(at(memory.image_base(), OWNER_RVA))?;
    let raw: [u8; 0x20] = memory.read(decode_array_pointer(&owner))?;
    let word = |offset: usize| u32::from_le_bytes(raw[offset..offset + 4].try_into().unwrap());
    Ok(ArrayHeader {
        chunk_table: u64::from_le_bytes(raw[0..8].try_into().unwrap()),
        max_elements: word(0x10),
        num_elements: word(0x14),
        max_chunks: word(0x18),
        num_chunks: word(0x1C),
    })
}

/// walks GObjects; Ok(None) when the header fails `rule`. the chunk table is read
/// one u64 per chunk, like the python list comprehension.
pub fn object_array(memory: &impl Memory, rule: HeaderRule) -> Result<Option<GObjects>, OracleError> {
    let header = read_array_header(memory)?;
    if !rule.accepts(&header) {
        return Ok(None);
    }
    let chunks = (0..header.num_chunks as u64).map(|i| memory.u64(at(header.chunk_table, 8 * i))).collect::<Result<Vec<_>, _>>()?;
    Ok(Some(GObjects { chunks, num_elements: header.num_elements }))
}

/// the raising form (RES:35-36, CIE:28-29, FNP:48-49).
pub fn require_object_array(memory: &impl Memory, rule: HeaderRule) -> Result<GObjects, OracleError> {
    object_array(memory, rule)?.ok_or(OracleError::Invalid("GObjects header failed validation"))
}

/// derive_name_key's own walk (FNP:44-54): the strict header rule, then only the
/// chunk holding the engine. it does not check that chunk against NumChunks the
/// way read_object does, so one chunk is enough, and it never reads the others.
pub fn name_key_engine(memory: &impl Memory) -> Result<u64, OracleError> {
    let header = read_array_header(memory)?;
    if !HeaderRule::NameKey.accepts(&header) {
        return Err(OracleError::Invalid("GObjects header failed validation"));
    }
    let chunk = memory.u64(at(header.chunk_table, 8 * (ENGINE_INDEX / 0x10000) as u64))?;
    let engine = memory.u64(at(chunk, 24 * (ENGINE_INDEX % 0x10000) as u64))?;
    if memory.u32(at(engine, 0xC))? != ENGINE_INDEX {
        return Err(OracleError::Invalid("AresGameEngine object index changed"));
    }
    Ok(engine)
}

/// GEngine -> GameViewport +0xBD0 -> World +0x80 (RES:47-57). no null checks.
pub fn find_world(memory: &impl Memory) -> Result<u64, OracleError> {
    let objects = require_object_array(memory, HeaderRule::Chain)?;
    world_of(memory, &objects)
}

fn world_of(memory: &impl Memory, objects: &GObjects) -> Result<u64, OracleError> {
    let engine = objects.object(memory, ENGINE_INDEX)?;
    let viewport = memory.u64(at(engine, 0xBD0))?;
    memory.u64(at(viewport, 0x80))
}

/// world +0x1D8 game instance -> +0x40 LocalPlayers[0] -> +0x38 controller (RES:60-63).
pub fn controller_of(memory: &impl Memory, world: u64) -> Result<u64, OracleError> {
    let game_instance = memory.u64(at(world, 0x1D8))?;
    let local_player = memory.u64(memory.u64(at(game_instance, 0x40))?)?;
    memory.u64(at(local_player, 0x38))
}

pub fn find_controller(memory: &impl Memory) -> Result<u64, OracleError> {
    let world = find_world(memory)?;
    controller_of(memory, world)
}

/// controller +0x4B0, 0 when the controller is null (RES:30-44).
pub fn find_pawn(memory: &impl Memory) -> Result<u64, OracleError> {
    let controller = find_controller(memory)?;
    if controller == 0 { Ok(0) } else { memory.u64(at(controller, 0x4B0)) }
}

/// PlayerCameraManager (controller +0x528) CameraCachePrivate POV at +0x17C0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub location: [f64; 3],
    /// pitch, yaw, roll in degrees
    pub rotation: [f64; 3],
}

/// read_camera without an explicit controller (RES:66-74); None when there is no manager.
pub fn read_camera(memory: &impl Memory) -> Result<Option<Camera>, OracleError> {
    let controller = find_controller(memory)?;
    let manager = memory.u64(at(controller, 0x528))?;
    if manager == 0 {
        return Ok(None);
    }
    let raw: [u8; 48] = memory.read(at(manager, 0x17C0))?;
    let value = |i: usize| f64::from_le_bytes(raw[8 * i..8 * i + 8].try_into().unwrap());
    Ok(Some(Camera { location: [value(0), value(1), value(2)], rotation: [value(3), value(4), value(5)] }))
}

/// the objects every snapshot starts from (spec 09 §7.2): the world chain plus the
/// pawn's inventory and current equippable, with the python null guards.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorldChain {
    pub engine: u64,
    pub viewport: u64,
    pub world: u64,
    pub net_driver: u64,
    pub game_instance: u64,
    pub local_player: u64,
    pub controller: u64,
    /// controller +0x4B0, 0 when the controller is null (RES:44)
    pub pawn: u64,
    /// pawn +0xBF8, 0 without a pawn (RES:137)
    pub inventory: u64,
    /// inventory +0x278, 0 without an inventory (RES:140)
    pub current_equippable: u64,
}

pub fn read_world_chain(memory: &impl Memory) -> Result<WorldChain, OracleError> {
    let objects = require_object_array(memory, HeaderRule::Chain)?;
    let engine = objects.object(memory, ENGINE_INDEX)?;
    let viewport = memory.u64(at(engine, 0xBD0))?;
    let world = memory.u64(at(viewport, 0x80))?;
    // RRI:69 netdriver = world +0x40
    let net_driver = memory.u64(at(world, 0x40))?;
    let game_instance = memory.u64(at(world, 0x1D8))?;
    let local_player = memory.u64(memory.u64(at(game_instance, 0x40))?)?;
    let controller = memory.u64(at(local_player, 0x38))?;
    let pawn = if controller == 0 { 0 } else { memory.u64(at(controller, 0x4B0))? };
    let inventory = if pawn == 0 { 0 } else { memory.u64(at(pawn, 0xBF8))? };
    let current_equippable = if inventory == 0 { 0 } else { memory.u64(at(inventory, 0x278))? };
    Ok(WorldChain { engine, viewport, world, net_driver, game_instance, local_player, controller, pawn, inventory, current_equippable })
}
