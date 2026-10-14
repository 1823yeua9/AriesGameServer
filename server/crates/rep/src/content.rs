//! the client's content-block transform (FContentBlockReader::SerializeBits,
//! 0x7FF738F94320), inverted so our plain rep/rpc payloads read back clean.
//! mirrors range_server_bunch.encode_short_actor_rep_payload.

use aries_bits::Bits;
use aries_net::sbox::{BYTE_SBOX_INVERSE, WIDE_SBOX, WORD_SBOX};
use aries_net::transform::reverse_wide_bits;

const MULTIPLIER: u64 = 0x2545_F491_4F6C_DD1D;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContentError {
    #[error("unsupported short content transform ({bits} bits at offset {offset})")]
    Unsupported { bits: usize, offset: u32 },
    #[error("content transform has no unique inverse")]
    NoInverse,
}

fn swap_adjacent(value: u32) -> u32 {
    (value & 0xD555_5555).wrapping_mul(2) | ((value >> 1) & 0x5555_5555)
}

/// the client's 32-bit content-reader permutation before its two s-boxes.
fn scramble_content_word(wire_word: u32, seed: u32) -> u32 {
    let mut mixed = swap_adjacent(wire_word) ^ seed.rotate_left(7);
    mixed = swap_adjacent(mixed);
    let high = ((mixed >> 2) & 0x3303_0303) | ((mixed & 0xF333_3333).wrapping_mul(4) & 0xFF0F_0F0F);
    let low = ((((mixed >> 2) & 0x3333_3333) as u64 | ((mixed & 0xF333_3333) as u64 * 4)) >> 4) as u32;
    let folded = high.wrapping_mul(16) | (low & 0x0F0F_0F0F);
    swap_adjacent(folded.swap_bytes())
}

/// solves the affine gf(2) map exactly like python invert_content_word_scramble.
fn invert_content_word_scramble(target: u32, seed: u32) -> Result<u32, ContentError> {
    let origin = scramble_content_word(0, seed);
    let basis: [u32; 32] = std::array::from_fn(|i| scramble_content_word(1 << i, seed) ^ origin);
    let mut rows: [u32; 32] = std::array::from_fn(|bit| {
        basis.iter().enumerate().fold(0u32, |acc, (index, column)| acc | (((column >> bit) & 1) << index))
    });
    let mut right = target ^ origin;
    for pivot in 0..32 {
        let row_index = (pivot..32).find(|&i| rows[i] & (1 << pivot) != 0).ok_or(ContentError::NoInverse)?;
        rows.swap(pivot, row_index);
        let pivot_bit = (right >> row_index) & 1;
        let other_bit = (right >> pivot) & 1;
        if pivot_bit != other_bit {
            right ^= (1 << pivot) | (1 << row_index);
        }
        for i in 0..32 {
            if i != pivot && rows[i] & (1 << pivot) != 0 {
                rows[i] ^= rows[pivot];
                if right & (1 << pivot) != 0 {
                    right ^= 1 << i;
                }
            }
        }
    }
    Ok(right)
}

fn substitute_word(value: u32) -> u32 {
    u32::from_le_bytes(value.to_le_bytes().map(|b| WORD_SBOX[b as usize]))
}

fn substitute_wide(value: u64) -> u64 {
    u64::from_le_bytes(value.to_le_bytes().map(|b| WIDE_SBOX[b as usize]))
}

fn encode_content_word(plain_word: u32, seed: u32) -> Result<u32, ContentError> {
    let rotated_seed = seed.rotate_left(2);
    let before_subtraction = rotated_seed.wrapping_sub(1).wrapping_sub(substitute_word(plain_word));
    invert_content_word_scramble(substitute_word(before_subtraction), seed)
}

fn swap_adjacent_64(value: u64) -> u64 {
    ((value >> 1) & 0x5555_5555_5555_5555) | ((value << 1) & 0xAAAA_AAAA_AAAA_AAAA)
}

fn encode_content_wide(plain_wide: u64, seed: u32) -> u64 {
    let before_subtraction = (seed.rotate_right(2) as u64).wrapping_sub(1).wrapping_sub(substitute_wide(plain_wide));
    // one extra adjacent-bit swap after reverse_wide_bits, unlike the packet transform
    let mixed = swap_adjacent_64(reverse_wide_bits(substitute_wide(before_subtraction)));
    let xor_mask = !(seed.rotate_right(7) as u64);
    swap_adjacent_64(mixed ^ xor_mask)
}

fn mix_seed(value: u32, shifted_source: u32) -> u64 {
    let term = value ^ (shifted_source << 25) ^ ((value ^ (value >> 15)) >> 12);
    MULTIPLIER.wrapping_mul(term as u64)
}

fn transform_byte(wire_byte: u8, byte_seed: u32) -> u8 {
    let wire = wire_byte as u32;
    let first_stage = (0u32.wrapping_sub(61).wrapping_mul(byte_seed) & 0xFF) ^ (((wire >> 1) & 0x55) | ((wire & 0xD5) * 2 & 0xFF));
    let spread = ((first_stage as u64 * 32800) & 0x88440) | ((first_stage as u64 * 2050) & 0x22110);
    let shuffled = ((65793 * spread) >> 16) as u32 & 0xFF;
    let inner_index = ((shuffled >> 1) & 0x55) | ((shuffled & 0xD5) * 2 & 0xFF);
    let inner = BYTE_SBOX_INVERSE[inner_index as usize] as u32;
    let outer_index = byte_seed.wrapping_mul(121).wrapping_add(!inner) & 0xFF;
    BYTE_SBOX_INVERSE[outer_index as usize]
}

/// python encode_short_actor_rep_payload. actor_map_value is the channel
/// actor's net guid (or the controller guid 4 for its rpcs).
pub fn encode_short_actor_rep_payload(payload: &Bits, source_bit_offset: u32, actor_map_value: u32) -> Result<Vec<u8>, ContentError> {
    let bits = payload.bit_count;
    if bits < 1 || source_bit_offset > 6 || (source_bit_offset == 0 && bits < 9) || (source_bit_offset != 0 && bits != 9) {
        return Err(ContentError::Unsupported { bits, offset: source_bit_offset });
    }
    let seed = actor_map_value ^ bits as u32;
    let first = mix_seed(seed, seed);
    let second = mix_seed(seed.wrapping_sub(378_250_948), seed.wrapping_add(60));
    let data = &payload.bytes;
    if source_bit_offset != 0 {
        let next_seed = (first.wrapping_add(second) >> 32) as u32;
        let first_mask = ((seed ^ 0x3C) & (0xFF << source_bit_offset)) >> source_bit_offset;
        let remaining_bits = bits as u32 - (8 - source_bit_offset);
        let last_mask = ((next_seed ^ 0x3C) & ((1 << remaining_bits) - 1)) << (8 - source_bit_offset);
        let value = u16::from_le_bytes([data[0], data[1]]) as u32;
        let encoded = (value ^ first_mask ^ last_mask) & 0x1FF;
        return Ok((encoded as u16).to_le_bytes().to_vec());
    }

    let (mut current_seed, mut current_first, mut current_second) = (seed, first, second);
    let advance = |seed: &mut u32, first: &mut u64, second: &mut u64| {
        let next_seed = (first.wrapping_add(*second) >> 32) as u32;
        let combined = *second ^ *first;
        let next_second = combined ^ (combined << 14) ^ second.rotate_right(9);
        let next_first = combined.rotate_right(28);
        (*seed, *first, *second) = (next_seed, next_first, next_second);
    };
    let mut out = Vec::with_capacity(data.len());
    let mut offset = 0usize;
    for _ in 0..bits / 64 {
        let plain = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
        out.extend_from_slice(&encode_content_wide(plain, current_seed).to_le_bytes());
        offset += 8;
        advance(&mut current_seed, &mut current_first, &mut current_second);
    }
    if bits - 8 * offset >= 32 {
        let plain = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
        out.extend_from_slice(&encode_content_word(plain, current_seed)?.to_le_bytes());
        offset += 4;
        advance(&mut current_seed, &mut current_first, &mut current_second);
    }
    for &desired in &data[offset..bits / 8] {
        let mut found = None;
        for wire in 0..=255u8 {
            if transform_byte(wire, current_seed) == desired {
                if found.is_some() {
                    return Err(ContentError::NoInverse);
                }
                found = Some(wire);
            }
        }
        out.push(found.ok_or(ContentError::NoInverse)?);
        advance(&mut current_seed, &mut current_first, &mut current_second);
    }
    if bits % 8 != 0 {
        let mask = ((1u32 << (bits % 8)) - 1) as u8;
        let final_bits = data[data.len() - 1] & mask;
        out.push(final_bits ^ ((current_seed ^ 0x3C) as u8 & mask));
    }
    Ok(out)
}

/// encodes a plain payload for a content block (offset 0), keeping its bit count.
pub fn encode_block_payload(plain: &Bits, actor_map_value: u32) -> Result<Bits, ContentError> {
    let bytes = encode_short_actor_rep_payload(plain, 0, actor_map_value)?;
    Ok(Bits { bytes, bit_count: plain.bit_count })
}
