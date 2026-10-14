use crate::{BitError, Bits};

/// lsb-first bit writer. the first error sticks and later appends are ignored.
#[derive(Clone, Default)]
pub struct BitWriter {
    bytes: Vec<u8>,
    bit_count: usize,
    error: Option<BitError>,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bit_count(&self) -> usize {
        self.bit_count
    }

    pub fn error(&self) -> Option<&BitError> {
        self.error.as_ref()
    }

    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }

    /// marks the writer bad from outside (a builder found a bad argument).
    pub fn set_error(&mut self, error: BitError) {
        if self.error.is_none() {
            self.error = Some(error);
        }
    }

    pub fn append_bits(&mut self, value: u64, width: u32) {
        if self.error.is_some() {
            return;
        }
        if width > 64 || (width < 64 && value >> width != 0) {
            self.set_error(BitError::ValueTooWide { value, width });
            return;
        }
        let mut remaining = width;
        let mut value = value;
        while remaining > 0 {
            let offset = (self.bit_count & 7) as u32;
            if offset == 0 {
                self.bytes.push(0);
            }
            let take = remaining.min(8 - offset);
            let part = (value & ((1u64 << take) - 1)) as u8;
            *self.bytes.last_mut().expect("byte pushed above") |= part << offset;
            value = if take == 64 { 0 } else { value >> take };
            remaining -= take;
            self.bit_count += take as usize;
        }
    }

    pub fn append_bool(&mut self, value: bool) {
        self.append_bits(value as u64, 1);
    }

    /// ue SerializeIntPacked: 7 bits per byte, low bit = more follows.
    pub fn append_packed_uint(&mut self, value: u64) {
        if value > u32::MAX as u64 {
            self.set_error(BitError::PackedOutOfRange);
            return;
        }
        let mut value = value;
        loop {
            let part = value & 0x7F;
            value >>= 7;
            self.append_bits((part << 1) | (value != 0) as u64, 8);
            if value == 0 {
                break;
            }
        }
    }

    /// ue FBitWriter::SerializeInt: value bits lsb-first until the next mask
    /// would reach value_max. returns the bits written (value- and max-dependent).
    pub fn append_serialize_int(&mut self, value: u64, value_max: u64) -> u32 {
        if value >= value_max || value_max > 1u64 << 32 {
            self.set_error(BitError::SerializeIntRange { value, max: value_max });
            return 0;
        }
        let mut written = 0;
        let mut new_value = 0u64;
        let mut mask = 1u64;
        while new_value + mask < value_max && mask < 1u64 << 32 {
            let bit = value & mask != 0;
            self.append_bool(bit);
            if bit {
                new_value += mask;
            }
            mask <<= 1;
            written += 1;
        }
        written
    }

    /// appends exactly bit_count bits of payload (python append_payload).
    pub fn append_payload(&mut self, payload: &[u8], bit_count: usize) {
        if payload.len() != bit_count.div_ceil(8) {
            self.set_error(BitError::LengthMismatch { bytes: payload.len(), bits: bit_count });
            return;
        }
        let mut left = bit_count;
        for &byte in payload {
            let take = left.min(8) as u32;
            self.append_bits((byte as u64) & ((1u64 << take) - 1), take);
            left -= take as usize;
        }
    }

    pub fn append_bits_value(&mut self, bits: &Bits) {
        self.append_payload(&bits.bytes, bits.bit_count);
    }

    pub fn append_writer(&mut self, other: &BitWriter) {
        if let Some(error) = other.error.clone() {
            self.set_error(error);
            return;
        }
        self.append_payload(&other.bytes, other.bit_count);
    }

    /// ue FString as ansi: int32 length incl. nul, bytes, nul. python bounds it
    /// to non-empty ascii without nul, at most 240 chars.
    pub fn append_fstring_ascii(&mut self, value: &str) {
        let encoded = value.as_bytes();
        if encoded.is_empty() || !value.is_ascii() || encoded.contains(&0) || encoded.len() > 240 {
            self.set_error(BitError::BadString);
            return;
        }
        self.append_bits(encoded.len() as u64 + 1, 32);
        for &byte in encoded {
            self.append_bits(byte as u64, 8);
        }
        self.append_bits(0, 8);
    }

    pub fn append_u32(&mut self, value: u32) {
        self.append_bits(value as u64, 32);
    }

    pub fn append_i32(&mut self, value: i32) {
        self.append_bits(value as u32 as u64, 32);
    }

    pub fn append_f32(&mut self, value: f32) {
        self.append_bits(value.to_bits() as u64, 32);
    }

    pub fn append_f64(&mut self, value: f64) {
        self.append_bits(value.to_bits(), 64);
    }

    /// bytes so far; unused high bits of the last byte are zero.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    pub fn finish(self) -> Result<Bits, BitError> {
        match self.error {
            Some(error) => Err(error),
            None => Ok(Bits { bytes: self.bytes, bit_count: self.bit_count }),
        }
    }

    /// the bits without consuming; errors if the writer failed.
    pub fn bits(&self) -> Result<Bits, BitError> {
        match &self.error {
            Some(error) => Err(error.clone()),
            None => Ok(Bits { bytes: self.bytes.clone(), bit_count: self.bit_count }),
        }
    }
}
