//! the ares packet transform applied before gcm (server -> client) and undone
//! after it (client -> server). mirrors range_udp_transform.py.

use crate::sbox::{BYTE_SBOX, BYTE_SBOX_INVERSE, WIDE_SBOX, WIDE_SBOX_INVERSE, WORD_SBOX, WORD_SBOX_INVERSE};

const MULTIPLIER: u64 = 0x2545_F491_4F6C_DD1D;

/// the client's 64-bit bit permutation (same in both directions).
pub fn reverse_wide_bits(value: u64) -> u64 {
    let first = (value >> 1) ^ (((value >> 1) ^ (value << 1)) & 0xAAAA_AAAA_AAAA_AAAA);
    let second = (first >> 2) ^ (((first >> 2) ^ (first << 2)) & 0xCCCC_CCCC_CCCC_CCCC);
    let third = (second >> 4) ^ (((second >> 4) ^ (second << 4)) & 0xF0F0_F0F0_F0F0_F0F0);
    let fourth = (third >> 8) ^ (((third >> 8) ^ (third << 8)) & 0xFF00_FF00_FF00_FF00);
    fourth.rotate_right(32)
}

fn substitute_wide(value: u64, table: &[u8; 256]) -> u64 {
    u64::from_le_bytes(value.to_le_bytes().map(|b| table[b as usize]))
}

fn substitute_word(value: u32, table: &[u8; 256]) -> u32 {
    u32::from_le_bytes(value.to_le_bytes().map(|b| table[b as usize]))
}

/// (state_first, state_second, selector) seeded from the packet counter.
fn initial_state(counter: u32) -> (u64, u64, u32) {
    let base_first = counter.wrapping_add(38_722_752);
    let mut mix_first = base_first ^ ((base_first ^ (base_first >> 15)) >> 12);
    mix_first ^= counter.wrapping_sub(64) << 25;
    let mut mix_second = counter ^ (counter << 25);
    mix_second ^= (counter ^ (counter >> 15)) >> 12;
    (MULTIPLIER.wrapping_mul(mix_first as u64), MULTIPLIER.wrapping_mul(mix_second as u64), counter)
}

fn advance_state(first: u64, second: u64) -> (u64, u64, u32) {
    let combined = first.wrapping_add(second);
    let mixed = first ^ second;
    let next_first = mixed ^ (mixed << 14) ^ first.rotate_right(9);
    let next_second = mixed.rotate_right(28);
    (next_first, next_second, (combined >> 32) as u32)
}

fn check(len: usize, bit_count: usize) -> bool {
    bit_count > 0 && bit_count <= 8192 && bit_count <= len * 8
}

/// outgoing transform, in place over the first bit_count bits.
pub fn transform_short_packet(data: &mut [u8], bit_count: usize, counter: u32) -> bool {
    if !check(data.len(), bit_count) {
        return false;
    }
    let (mut first, mut second, mut selector) = initial_state(counter);
    let mut offset = 0usize;
    for _ in 0..bit_count / 64 {
        let mut value = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
        value = value.wrapping_add(selector.rotate_right(1) as u64);
        value = substitute_wide(value, &WIDE_SBOX);
        value = value.wrapping_add(selector.rotate_right(3) as u64);
        value = substitute_wide(value, &WIDE_SBOX);
        value = reverse_wide_bits(value.wrapping_sub(selector.rotate_right(5) as u64));
        value = substitute_wide(value, &WIDE_SBOX);
        data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        (first, second, selector) = advance_state(first, second);
        offset += 8;
    }
    let remainder = bit_count % 64;
    for _ in 0..remainder / 32 {
        let mut value = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
        value = value.wrapping_add(selector.rotate_left(1));
        value = substitute_word(value, &WORD_SBOX);
        value = value.wrapping_add(selector.rotate_left(3));
        value = substitute_word(value, &WORD_SBOX);
        value = value.wrapping_sub(selector.rotate_left(5)).reverse_bits();
        value = substitute_word(value, &WORD_SBOX);
        data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        (first, second, selector) = advance_state(first, second);
        offset += 4;
    }
    let remainder = remainder % 32;
    for _ in 0..remainder / 8 {
        let mut value = data[offset] as u32;
        value = BYTE_SBOX[(value.wrapping_add(selector.wrapping_mul(11)) & 0xFF) as usize] as u32;
        value = BYTE_SBOX[(value.wrapping_add(selector.wrapping_mul(51)) & 0xFF) as usize] as u32;
        value = value.wrapping_sub(selector.wrapping_mul(27)) & 0xFF;
        data[offset] = BYTE_SBOX[(value as u8).reverse_bits() as usize];
        (first, second, selector) = advance_state(first, second);
        offset += 1;
    }
    let remaining_bits = remainder % 8;
    if remaining_bits != 0 {
        data[offset] ^= (((1u32 << remaining_bits) - 1) & (selector ^ 0xC0)) as u8;
    }
    true
}

/// incoming inverse transform, in place over the first bit_count bits.
pub fn inverse_short_packet(data: &mut [u8], bit_count: usize, counter: u32) -> bool {
    if !check(data.len(), bit_count) {
        return false;
    }
    let (mut first, mut second, mut selector) = initial_state(counter);
    let mut offset = 0usize;
    for _ in 0..bit_count / 64 {
        let mut value = u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap());
        value = substitute_wide(value, &WIDE_SBOX_INVERSE);
        value = reverse_wide_bits(value).wrapping_add(selector.rotate_right(5) as u64);
        value = substitute_wide(value, &WIDE_SBOX_INVERSE);
        value = value.wrapping_sub(selector.rotate_right(3) as u64);
        value = substitute_wide(value, &WIDE_SBOX_INVERSE);
        value = value.wrapping_sub(selector.rotate_right(1) as u64);
        data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        (first, second, selector) = advance_state(first, second);
        offset += 8;
    }
    let remainder = bit_count % 64;
    for _ in 0..remainder / 32 {
        let mut value = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
        value = substitute_word(value, &WORD_SBOX_INVERSE);
        value = value.reverse_bits().wrapping_add(selector.rotate_left(5));
        value = substitute_word(value, &WORD_SBOX_INVERSE);
        value = value.wrapping_sub(selector.rotate_left(3));
        value = substitute_word(value, &WORD_SBOX_INVERSE);
        value = value.wrapping_sub(selector.rotate_left(1));
        data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        (first, second, selector) = advance_state(first, second);
        offset += 4;
    }
    let remainder = remainder % 32;
    for _ in 0..remainder / 8 {
        let mut value = BYTE_SBOX_INVERSE[data[offset] as usize] as u32;
        value = (value as u8).reverse_bits() as u32;
        value = value.wrapping_add(selector.wrapping_mul(27)) & 0xFF;
        value = BYTE_SBOX_INVERSE[value as usize] as u32;
        value = value.wrapping_sub(selector.wrapping_mul(51)) & 0xFF;
        value = BYTE_SBOX_INVERSE[value as usize] as u32;
        data[offset] = (value.wrapping_sub(selector.wrapping_mul(11)) & 0xFF) as u8;
        (first, second, selector) = advance_state(first, second);
        offset += 1;
    }
    let remaining_bits = remainder % 8;
    if remaining_bits != 0 {
        data[offset] ^= (((1u32 << remaining_bits) - 1) & (selector ^ 0xC0)) as u8;
    }
    true
}
