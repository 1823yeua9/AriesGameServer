//! GObjects (obfuscated array pointer) and the encrypted FName pool.
//! mirrors range_authority_probe/read_world_state.py and read_fname_pool.py.

use std::collections::HashMap;

use aries_net::transform::reverse_wide_bits;

use crate::process::valid_address;
use crate::{Block, Memory, OracleError};

/// rva of the obfuscated GObjects owner block.
pub const OWNER_RVA: u64 = 0xD3E_7140;
/// rva of the FName pool.
pub const NAME_POOL_RVA: u64 = 0xD54_8540;
/// GObjects index of AresGameEngine, the anchor for the name key.
pub const ENGINE_INDEX: u32 = 0x107E2;
const ENGINE_NAME: &[u8] = b"AresGameEngine";
const MULTIPLIER: u64 = 0x2545_F491_4F6C_DD1D;
/// a pool block holds 0x10000 entry slots of 4 bytes; no entry crosses its end
const POOL_BLOCK_BYTES: u64 = 0x10000 * 4;
/// the first read of an entry takes its header and this many name bytes; a
/// longer name takes a second read for the rest
const NAME_GUESS: u64 = 96;

fn swap_adjacent_bits(value: u64) -> u64 {
    ((value >> 1) & 0x5555_5555_5555_5555) | ((value << 1) & 0xAAAA_AAAA_AAAA_AAAA)
}

/// python decode_array_pointer over the 0x80-byte owner block.
pub fn decode_array_pointer(owner: &[u8; 0x80]) -> u64 {
    let seed = u32::from_le_bytes(owner[0x78..0x7C].try_into().unwrap());
    let mixed_seed = seed ^ ((seed ^ (seed >> 15)) >> 12) ^ (seed << 25);
    let mixed = MULTIPLIER.wrapping_mul(mixed_seed as u64);
    let slot = (mixed % 7) as usize;
    let high = mixed >> 32;
    let encoded = u64::from_le_bytes(owner[0x40 + 8 * slot..0x48 + 8 * slot].try_into().unwrap());
    let first_shift = ((high + 2 * slot as u64) % 63 + 1) as u32;
    let second_shift = ((high + slot as u64) % 63 + 1) as u32;
    let low32 = |value: u64| value & 0xFFFF_FFFF;
    let decoded = match slot {
        0 => reverse_wide_bits(encoded ^ !low32(high.wrapping_sub(1))),
        1 => swap_adjacent_bits(encoded.wrapping_sub(low32(high + 2))),
        2 => encoded.rotate_right(first_shift).rotate_right(second_shift),
        3 => encoded.rotate_right(first_shift).rotate_left(second_shift),
        4 => encoded ^ low32(high + 8),
        5 => swap_adjacent_bits(encoded) ^ !low32(high + 5),
        _ => reverse_wide_bits(encoded ^ !low32(high + 12)),
    };
    decoded ^ seed as u64
}

/// the object array: chunk pointers (0x10000 items of 24 bytes each).
pub struct GObjects {
    pub chunks: Vec<u64>,
    pub num_elements: u32,
}

impl GObjects {
    /// the walk with the name-key header rule (FNP:48), the strictest one.
    pub fn read(process: &impl Memory) -> Result<Self, OracleError> {
        crate::world::require_object_array(process, crate::world::HeaderRule::NameKey)
    }

    pub fn object(&self, process: &impl Memory, index: u32) -> Result<u64, OracleError> {
        let chunk = *self.chunks.get((index / 0x10000) as usize).ok_or(OracleError::Invalid("object index outside populated chunks"))?;
        process.u64(chunk.saturating_add(24 * (index % 0x10000) as u64))
    }

    /// GEngine (AresGameEngine), checked by its object index.
    pub fn engine(&self, process: &impl Memory) -> Result<u64, OracleError> {
        let engine = self.object(process, ENGINE_INDEX)?;
        if process.u32(engine.saturating_add(0xC))? != ENGINE_INDEX {
            return Err(OracleError::Invalid("AresGameEngine object index changed"));
        }
        Ok(engine)
    }
}

/// decrypted FName lookups with the per-run 4-byte key, cached by index.
pub struct NamePool {
    key: [u8; 4],
    cache: HashMap<u32, String>,
    /// pool block pointers by block index: allocated once, never moved or freed
    blocks: HashMap<u32, u64>,
}

fn check_index(index: u32) -> Result<(), OracleError> {
    if index >= 0x1000_0000 {
        return Err(OracleError::Invalid("FName comparison index outside bounded range"));
    }
    Ok(())
}

fn read_pool_block(process: &impl Memory, index: u32) -> Result<u64, OracleError> {
    // saturating: python ints never wrap, an overflowing address fails the bounds check
    let block = process.u64(process.image_base().saturating_add(NAME_POOL_RVA + 0x10 + (index >> 16) as u64 * 8))?;
    if !valid_address(block) {
        return Err(OracleError::Invalid("invalid FName pool block pointer"));
    }
    Ok(block)
}

/// the entry's header and its bytes, in one read for names up to NAME_GUESS bytes.
fn read_name_entry(process: &impl Memory, block: u64, index: u32) -> Result<(Vec<u8>, usize, bool), OracleError> {
    let entry = block + (index & 0xFFFF) as u64 * 4;
    let room = (block + POOL_BLOCK_BYTES).saturating_sub(entry);
    let fields = Block::read(process, entry, 4, (6 + NAME_GUESS).min(room).max(6));
    let header = fields.u16(process, 4)?;
    let length = (header >> 1) as usize;
    let wide = header & 1 != 0;
    if !(1..=1023).contains(&length) {
        return Err(OracleError::Invalid("invalid FName entry length"));
    }
    let encrypted = fields.bytes(process, 6, length * if wide { 2 } else { 1 })?;
    Ok((encrypted, length, wide))
}

impl NamePool {
    /// derives and validates the key from the AresGameEngine name.
    pub fn new(process: &impl Memory, objects: &GObjects) -> Result<Self, OracleError> {
        Self::from_engine(process, objects.engine(process)?)
    }

    /// the key from an engine object whose index was already checked (FNP:55-64).
    pub fn from_engine(process: &impl Memory, engine: u64) -> Result<Self, OracleError> {
        let name_index = process.u32(engine.saturating_add(0x18))?;
        check_index(name_index)?;
        let (encrypted, length, wide) = read_name_entry(process, read_pool_block(process, name_index)?, name_index)?;
        if wide || length != ENGINE_NAME.len() {
            return Err(OracleError::Invalid("AresGameEngine anchor has an unexpected name layout"));
        }
        let key: [u8; 4] = std::array::from_fn(|i| encrypted[i] ^ length as u8 ^ ENGINE_NAME[i]);
        let decoded: Vec<u8> = encrypted.iter().enumerate().map(|(i, b)| b ^ length as u8 ^ key[i & 3]).collect();
        if decoded != ENGINE_NAME {
            return Err(OracleError::Invalid("FName key failed full-name validation"));
        }
        Ok(Self { key, cache: HashMap::new(), blocks: HashMap::new() })
    }

    /// the derived per-run xor key (derive_name_key's return value).
    pub fn key(&self) -> [u8; 4] {
        self.key
    }

    pub fn name(&mut self, process: &impl Memory, index: u32) -> Result<String, OracleError> {
        if let Some(name) = self.cache.get(&index) {
            return Ok(name.clone());
        }
        check_index(index)?;
        let block = match self.blocks.get(&(index >> 16)) {
            Some(&block) => block,
            None => {
                let block = read_pool_block(process, index)?;
                self.blocks.insert(index >> 16, block);
                block
            }
        };
        let (encrypted, length, wide) = read_name_entry(process, block, index)?;
        // python xors the full length into each byte; from 256 up bytes() raises (FNP:69-70)
        if length >= 256 {
            return Err(OracleError::Invalid("FName entry too long to decode"));
        }
        let decoded: Vec<u8> = encrypted.iter().enumerate().map(|(i, b)| b ^ length as u8 ^ self.key[i & 3]).collect();
        let name = if wide {
            let units: Vec<u16> = decoded.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            String::from_utf16(&units).map_err(|_| OracleError::Invalid("bad wide FName"))?
        } else {
            String::from_utf8(decoded).map_err(|_| OracleError::Invalid("bad FName"))?
        };
        self.cache.insert(index, name.clone());
        Ok(name)
    }

    /// an object's own name (UObject +0x18 comparison index).
    pub fn object_name(&mut self, process: &impl Memory, object: u64) -> Result<String, OracleError> {
        let index = process.u32(object.saturating_add(0x18))?;
        self.name(process, index)
    }

    /// an object's class name (UObject +0x10 class, its +0x18 name). the class
    /// pointer is not null-checked: a bad one fails the name read (RES:149-151).
    pub fn class_name(&mut self, process: &impl Memory, object: u64) -> Result<String, OracleError> {
        let class = process.u64(object.saturating_add(0x10))?;
        self.object_name(process, class)
    }
}
