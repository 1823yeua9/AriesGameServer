use crate::{BitError, BitWriter};

/// round-half-away-from-zero of value * scale (python floor(v*s+0.5) / ceil(v*s-0.5)).
pub fn quantize_component(value: f64, scale: f64) -> f64 {
    if value >= 0.0 { (value * scale + 0.5).floor() } else { (value * scale - 0.5).ceil() }
}

/// this build's quantized vector (sub_7FF737520200): 7-bit header
/// (width | 0x40), then x, y, z as width-bit two's complement. width >= 1.
pub fn append_quantized_vector(writer: &mut BitWriter, vector: [f64; 3], scale: f64) {
    let mut components = [0i64; 3];
    for (slot, &value) in components.iter_mut().zip(vector.iter()) {
        let quantized = quantize_component(value, scale);
        if !quantized.is_finite() || quantized.abs() >= 9.0e18 {
            writer.set_error(BitError::BadVector);
            return;
        }
        *slot = quantized as i64;
    }
    let width = components
        .iter()
        .map(|&c| (64 - (if c >= 0 { c } else { !c }).leading_zeros()) + 1)
        .max()
        .unwrap_or(1)
        .max(1);
    if width > 63 {
        writer.set_error(BitError::BadVector);
        return;
    }
    writer.append_bits(width as u64 | 0x40, 7);
    for c in components {
        writer.append_bits((c as u64) & ((1u64 << width) - 1), width);
    }
}

/// spawn-location vector at scale 10; python also bounds components to int32.
pub fn append_quantized_vector10(writer: &mut BitWriter, vector: [f64; 3]) {
    for &value in &vector {
        let quantized = quantize_component(value, 10.0);
        if !value.is_finite() || quantized < -(2f64.powi(31)) || quantized >= 2f64.powi(31) {
            writer.set_error(BitError::BadVector);
            return;
        }
    }
    append_quantized_vector(writer, vector, 10.0);
}

/// FRotator SerializeCompressedShort value: python round() (half to even) & 0xffff.
pub fn compress_short(degrees: f64) -> u16 {
    ((degrees * 65536.0 / 360.0).round_ties_even() as i64 as u64 & 0xFFFF) as u16
}

/// FRotator::SerializeCompressed byte: floor(deg * 256 / 360 + 0.5) & 0xff.
pub fn compress_byte(degrees: f64) -> u8 {
    ((degrees * 256.0 / 360.0 + 0.5).floor() as i64 as u64 & 0xFF) as u8
}

/// per axis (pitch, yaw, roll): [nonzero bit][16 bits when nonzero].
pub fn append_rotator_shorts(writer: &mut BitWriter, pitch: f64, yaw: f64, roll: f64) {
    for degrees in [pitch, yaw, roll] {
        let value = compress_short(degrees);
        writer.append_bool(value != 0);
        if value != 0 {
            writer.append_bits(value as u64, 16);
        }
    }
}

/// per axis (pitch, yaw, roll): [nonzero bit][8 bits when nonzero].
pub fn append_rotator_bytes(writer: &mut BitWriter, pitch: f64, yaw: f64, roll: f64) {
    for degrees in [pitch, yaw, roll] {
        let value = compress_byte(degrees);
        writer.append_bool(value != 0);
        if value != 0 {
            writer.append_bits(value as u64, 8);
        }
    }
}
