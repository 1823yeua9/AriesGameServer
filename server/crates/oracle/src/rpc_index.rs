//! class net cache walk to resolve rpc field indices (spec 09 R15, resolve_rpc_index.py).
//! the wire writes SerializeInt(field_index, class_max_index + 1).

use crate::objects::ENGINE_INDEX;
use crate::world::{HeaderRule, at, object_array};
use crate::{Block, Memory, OracleError};

/// ClientSetHUD's native ExecFunction thunk (H:3784).
pub const CLIENT_SET_HUD_RVA: u64 = 0x4B6_B340;
/// netdriver +0x198 FClassNetCacheMgr (RRI:20)
const NETDRIVER_CACHE_MGR: u64 = 0x198;
/// UFunction +0xE0 ExecFunction (RRI:21)
const UFUNCTION_EXEC: u64 = 0xE0;
/// FClassNetCache: +0x8 parent, +0x20 fields, +0x28 count; 24-byte field entries (RRI:22-25)
const CACHE_PARENT: u64 = 0x8;
const CACHE_ENTRIES: u64 = 0x20;
const CACHE_COUNT: u64 = 0x28;
const CACHE_STRIDE: u64 = 24;
/// TSet entries of the cache manager are 24 bytes: key (low 32 bits class index), cache (RRI:112-116)
const SET_STRIDE: u64 = 24;
/// the parent walk is bounded (RRI:91)
const MAX_DEPTH: u32 = 16;
/// TSet slots read at a time
const SET_PAGE: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcActor {
    Controller,
    Pawn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RpcIndex {
    pub field_index: u32,
    /// max(base + count) over the cache chain (GetMaxIndex)
    pub class_max_index: u64,
    pub pawn: u64,
    pub controller: u64,
}

/// RRI:58 `readable(p)`: false for null, otherwise an 8-byte read that raises on failure.
fn readable(memory: &impl Memory, pointer: u64) -> Result<bool, OracleError> {
    if pointer == 0 {
        return Ok(false);
    }
    memory.read::<8>(pointer)?;
    Ok(true)
}

/// RRI:86-107 scan_chain: a later (parent) match overwrites an earlier one.
fn scan_chain(memory: &impl Memory, cache: u64, thunk: u64) -> Result<(Option<u32>, u64), OracleError> {
    let mut field_index = None;
    let mut class_max = 0u64;
    let mut current = cache;
    let mut depth = 0;
    // python order: null check, readable(c) (reads), then the depth bound
    while current != 0 && readable(memory, current)? && depth < MAX_DEPTH {
        let base = memory.u32(current)?;
        let count = memory.u32(at(current, CACHE_COUNT))?;
        let entries = memory.u64(at(current, CACHE_ENTRIES))?;
        if (1..=4096).contains(&count) {
            class_max = class_max.max(base as u64 + count as u64);
            if readable(memory, entries)? {
                // every field entry in one read
                let table = Block::read(memory, entries, 0, CACHE_STRIDE * count as u64);
                for j in 0..count as u64 {
                    let tag = table.u64(memory, CACHE_STRIDE * j)?;
                    if tag & 1 != 0 {
                        let function = tag & !1;
                        // readable(function) and its ExecFunction in one read
                        let fields = Block::read(memory, function, 0, UFUNCTION_EXEC + 8);
                        let readable = function != 0 && fields.get::<8>(memory, 0).map(|_| true)?;
                        if readable && fields.u64(memory, UFUNCTION_EXEC)? == thunk {
                            field_index = Some(table.u32(memory, CACHE_STRIDE * j + 8)?);
                        }
                    }
                }
            }
        }
        current = memory.u64(at(current, CACHE_PARENT))?;
        depth += 1;
    }
    Ok((field_index, class_max))
}

/// resolve (RRI:48-126). Ok(None) wherever the python returns None.
pub fn resolve_rpc_index(memory: &impl Memory, thunk: u64, actor_kind: RpcActor) -> Result<Option<RpcIndex>, OracleError> {
    let Some(objects) = object_array(memory, HeaderRule::Resolve)? else {
        return Ok(None);
    };
    // read order of RRI:67-73: engine, world, netdriver, game instance, player, controller, pawn
    let engine = objects.object(memory, ENGINE_INDEX)?;
    let world = memory.u64(at(memory.u64(at(engine, 0xBD0))?, 0x80))?;
    let net_driver = memory.u64(at(world, 0x40))?;
    let controller = crate::world::controller_of(memory, world)?;
    let pawn = memory.u64(at(controller, 0x4B0))?;
    let actor = match actor_kind {
        RpcActor::Controller => controller,
        RpcActor::Pawn => pawn,
    };
    if actor == 0 {
        return Ok(None);
    }
    let actor_class = memory.u64(at(actor, 0x10))?;
    // the actor's own object index, read and unused (RRI:78)
    memory.u32(at(actor, 0xC))?;
    let cache_mgr = memory.u64(at(net_driver, NETDRIVER_CACHE_MGR))?;
    let set_base = memory.u64(at(cache_mgr, 0x8))?;
    let set_count = memory.u32(at(cache_mgr, 0x10))?;
    if set_base == 0 || !(1..=0x10000).contains(&set_count) {
        return Ok(None);
    }
    let class_index = memory.u32(at(actor_class, 0xC))?;
    // no TSet hole check: slots are read until the first key match (RRI:111-117),
    // a page of slots per read
    let mut chosen = None;
    let mut page = None;
    for i in 0..set_count as u64 {
        if i % SET_PAGE == 0 {
            page = Some(Block::read(memory, set_base, SET_STRIDE * i, SET_STRIDE * (i + SET_PAGE).min(set_count as u64)));
        }
        let page = page.as_ref().expect("read at i = 0");
        if page.u64(memory, SET_STRIDE * i)? & 0xFFFF_FFFF == class_index as u64 {
            chosen = Some(page.u64(memory, SET_STRIDE * i + 8)?);
            break;
        }
    }
    let Some(cache) = chosen else {
        return Ok(None);
    };
    let (field_index, class_max_index) = scan_chain(memory, cache, thunk)?;
    Ok(field_index.map(|field_index| RpcIndex { field_index, class_max_index, pawn, controller }))
}
