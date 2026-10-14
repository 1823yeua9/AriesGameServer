//! the client -> server content-block transform, decoded the way the server's
//! FContentBlockReader does (range_client_input.decode_content_payload), and
//! the actor block header around it (read_actor_content_block).
//! crate::content holds the inverse (our encoder); its helpers are private, so
//! the shared word scramble and byte stage are copied here.

use aries_bits::Bits;
use aries_net::sbox::{BYTE_SBOX_INVERSE, WIDE_SBOX_INVERSE, WORD_SBOX_INVERSE};
use aries_net::transform::reverse_wide_bits;

use super::reader::{read_bits, read_packed_uint};
use super::{DecodeError, DecodeResult};

/// the controller's net guid, the actor map value of every ch5 rpc block (range_client_rpc.py:24).
pub const CONTROLLER_ACTOR_MAP_VALUE: u32 = 4;
/// the pawn's net guid, the default actor map value for ch8 (range_client_input.py:151).
pub const PAWN_ACTOR_MAP_VALUE: u32 = 10;
/// largest content block either side accepts (range_client_input.py:95, 162).
const MAX_CONTENT_BITS: usize = 8192;
const MULTIPLIER: u64 = 0x2545_F491_4F6C_DD1D;

fn swap_adjacent(value: u32) -> u32 {
    (value & 0xD555_5555).wrapping_mul(2) | ((value >> 1) & 0x5555_5555)
}

/// range_server_bunch.scramble_content_word (:1030-1047).
fn scramble_content_word(wire_word: u32, seed: u32) -> u32 {
    let mut mixed = swap_adjacent(wire_word) ^ seed.rotate_left(7);
    mixed = swap_adjacent(mixed);
    let high = ((mixed >> 2) & 0x3303_0303) | ((mixed & 0xF333_3333).wrapping_mul(4) & 0xFF0F_0F0F);
    // python keeps the bits shifted past 32 until the >> 4, so widen first
    let low = ((((mixed >> 2) & 0x3333_3333) as u64 | ((mixed & 0xF333_3333) as u64 * 4)) >> 4) as u32;
    let folded = high.wrapping_mul(16) | (low & 0x0F0F_0F0F);
    swap_adjacent(folded.swap_bytes())
}

fn substitute_wide(value: u64) -> u64 {
    u64::from_le_bytes(value.to_le_bytes().map(|b| WIDE_SBOX_INVERSE[b as usize]))
}

fn substitute_word(value: u32) -> u32 {
    u32::from_le_bytes(value.to_le_bytes().map(|b| WORD_SBOX_INVERSE[b as usize]))
}

fn swap_adjacent_64(value: u64) -> u64 {
    ((value >> 1) & 0x5555_5555_5555_5555) | ((value << 1) & 0xAAAA_AAAA_AAAA_AAAA)
}

/// range_client_input._decode_content_wide (:65-71).
fn decode_content_wide(wire_wide: u64, seed: u32) -> u64 {
    // python's ~ on the u32 rotation sets all 32 upper bits once masked to 64
    let xor_mask = !(seed.rotate_right(7) as u64);
    let mixed = swap_adjacent_64(wire_wide) ^ xor_mask;
    let before_subtraction = substitute_wide(reverse_wide_bits(swap_adjacent_64(mixed)));
    let before_last_sbox = (seed.rotate_right(2) as u64).wrapping_sub(1).wrapping_sub(before_subtraction);
    substitute_wide(before_last_sbox)
}

/// range_client_input._decode_content_word (:74-79).
fn decode_content_word(wire_word: u32, seed: u32) -> u32 {
    let before_subtraction = substitute_word(scramble_content_word(wire_word, seed));
    substitute_word(seed.rotate_left(2).wrapping_sub(1).wrapping_sub(before_subtraction))
}

/// range_client_input._decode_content_byte (:82-90).
fn decode_content_byte(wire_byte: u8, seed: u32) -> u8 {
    let wire = wire_byte as u32;
    // (-61 * seed) & 0xff: the u32 wrap keeps the low byte
    let first_stage = (0u32.wrapping_sub(61).wrapping_mul(seed) & 0xFF) ^ (((wire >> 1) & 0x55) | (((wire & 0xD5) * 2) & 0xFF));
    let spread = ((first_stage as u64 * 32800) & 0x88440) | ((first_stage as u64 * 2050) & 0x22110);
    let shuffled = ((65793 * spread) >> 16) as u32 & 0xFF;
    let inner_index = ((shuffled >> 1) & 0x55) | (((shuffled & 0xD5) * 2) & 0xFF);
    let inner = BYTE_SBOX_INVERSE[inner_index as usize] as u32;
    // python (121 * seed + ~inner) & 0xff
    BYTE_SBOX_INVERSE[(seed.wrapping_mul(121).wrapping_add(!inner) & 0xFF) as usize]
}

fn mix_seed(value: u32, shifted_source: u32) -> u64 {
    MULTIPLIER.wrapping_mul((value ^ (shifted_source << 25) ^ ((value ^ (value >> 15)) >> 12)) as u64)
}

/// range_client_input.decode_content_payload (:93-138): the matched
/// FContentBlockReader transform at bit offset zero. `actor_map_value` is the
/// channel actor's net guid (4 on ch5, 10 on ch8). the output keeps the bit
/// count; unused high bits of the last byte are zero.
pub fn decode_content_payload(wire: &Bits, actor_map_value: u32) -> DecodeResult<Bits> {
    let bit_count = wire.bit_count;
    if !(1..=MAX_CONTENT_BITS).contains(&bit_count) || wire.bytes.len() != bit_count.div_ceil(8) {
        return Err(DecodeError::ContentLength);
    }
    let data = &wire.bytes;
    let mut seed = actor_map_value ^ bit_count as u32;
    let mut first = mix_seed(seed, seed);
    let mut second = mix_seed(seed.wrapping_sub(378_250_948), seed.wrapping_add(60));
    let mut advance = |seed: &mut u32| {
        let next_seed = (first.wrapping_add(second) >> 32) as u32;
        let combined = second ^ first;
        let next_second = combined ^ (combined << 14) ^ second.rotate_right(9);
        first = combined.rotate_right(28);
        second = next_second;
        *seed = next_seed;
    };
    let mut output = Vec::with_capacity(data.len());
    let mut offset = 0usize;
    for _ in 0..bit_count / 64 {
        let encoded = u64::from_le_bytes(data[offset..offset + 8].try_into().expect("8 bytes"));
        output.extend_from_slice(&decode_content_wide(encoded, seed).to_le_bytes());
        offset += 8;
        advance(&mut seed);
    }
    if bit_count - 8 * offset >= 32 {
        let encoded = u32::from_le_bytes(data[offset..offset + 4].try_into().expect("4 bytes"));
        output.extend_from_slice(&decode_content_word(encoded, seed).to_le_bytes());
        offset += 4;
        advance(&mut seed);
    }
    while offset < bit_count / 8 {
        output.push(decode_content_byte(data[offset], seed));
        offset += 1;
        advance(&mut seed);
    }
    let tail = bit_count % 8;
    if tail != 0 {
        let mask = ((1u32 << tail) - 1) as u8;
        output.push((data[data.len() - 1] ^ (seed ^ 0x3C) as u8) & mask);
    }
    Ok(Bits { bytes: output, bit_count })
}

/// the first content block of a bunch, decoded (range_client_input.ActorContentBlock).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorContentBlock {
    /// packet bit where the block's wire payload starts
    pub bit_offset: usize,
    pub bit_count: usize,
    pub has_rep_layout: bool,
    /// always true: subobject blocks are rejected
    pub is_actor: bool,
    /// plain payload after decode_content_payload
    pub payload: Bits,
}

/// range_client_input.read_actor_content_block (:150-167). reads only the
/// bunch's first block: [has_rep_layout][is_actor][packed bit count][wire].
/// subobject blocks need their own net guid context and are rejected.
pub fn read_actor_content_block(packet: &Bits, body_start_bit: usize, body_end_bit: usize, actor_map_value: u32) -> DecodeResult<ActorContentBlock> {
    let data = &packet.bytes;
    if body_end_bit > data.len() * 8 || body_start_bit >= body_end_bit {
        return Err(DecodeError::ActorBounds);
    }
    if body_start_bit + 2 > body_end_bit {
        return Err(DecodeError::ActorHeaderTruncated);
    }
    let has_rep_layout = read_bits(data, body_start_bit, 1)? != 0;
    if read_bits(data, body_start_bit + 1, 1)? == 0 {
        return Err(DecodeError::Subobject);
    }
    let (bit_count, bit_offset) = read_packed_uint(data, body_start_bit + 2, body_end_bit)?;
    if bit_count > MAX_CONTENT_BITS as u64 || bit_offset + bit_count as usize > body_end_bit {
        return Err(DecodeError::ContentExceedsBunch);
    }
    let bit_count = bit_count as usize;
    let wire = (0..bit_count.div_ceil(8))
        .map(|index| read_bits(data, bit_offset + index * 8, (bit_count - index * 8).min(8)).map(|byte| byte as u8))
        .collect::<DecodeResult<Vec<u8>>>()?;
    let payload = decode_content_payload(&Bits { bytes: wire, bit_count }, actor_map_value)?;
    Ok(ActorContentBlock { bit_offset, bit_count, has_rep_layout, is_actor: true, payload })
}
