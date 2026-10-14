use crate::{BitError, Bits};

/// lsb-first bit reader over a bounded bit string. reads past the end set a
/// sticky error and return zero, like ue's FBitReader overflow.
#[derive(Clone)]
pub struct BitReader<'a> {
    bytes: &'a [u8],
    bit_count: usize,
    position: usize,
    error: Option<BitError>,
}

impl<'a> BitReader<'a> {
    pub fn new(bytes: &'a [u8], bit_count: usize) -> Self {
        let bit_count = bit_count.min(bytes.len() * 8);
        Self { bytes, bit_count, position: 0, error: None }
    }

    pub fn from_bits(bits: &'a Bits) -> Self {
        Self::new(&bits.bytes, bits.bit_count)
    }

    /// a reader over [start, end) of the same buffer.
    pub fn slice(bytes: &'a [u8], start: usize, end: usize) -> Self {
        let mut reader = Self::new(bytes, end);
        reader.position = start.min(reader.bit_count);
        reader
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn set_position(&mut self, position: usize) {
        self.position = position.min(self.bit_count);
    }

    pub fn bit_count(&self) -> usize {
        self.bit_count
    }

    pub fn remaining(&self) -> usize {
        self.bit_count - self.position
    }

    pub fn at_end(&self) -> bool {
        self.position >= self.bit_count
    }

    pub fn error(&self) -> Option<&BitError> {
        self.error.as_ref()
    }

    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }

    pub fn set_error(&mut self, error: BitError) {
        if self.error.is_none() {
            self.error = Some(error);
        }
    }

    pub fn read_bits(&mut self, width: u32) -> u64 {
        if self.error.is_some() || width == 0 {
            return 0;
        }
        if width > 64 || self.position + width as usize > self.bit_count {
            self.set_error(BitError::Overflow { wanted: width as usize, position: self.position, total: self.bit_count });
            return 0;
        }
        let mut value = 0u64;
        let mut done = 0u32;
        while done < width {
            let byte = self.bytes[self.position >> 3] as u64;
            let offset = (self.position & 7) as u32;
            let take = (width - done).min(8 - offset);
            value |= ((byte >> offset) & ((1u64 << take) - 1)) << done;
            done += take;
            self.position += take as usize;
        }
        value
    }

    pub fn read_bool(&mut self) -> bool {
        self.read_bits(1) != 0
    }

    pub fn read_packed_uint(&mut self) -> u64 {
        let mut value = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = self.read_bits(8);
            if self.error.is_some() {
                return 0;
            }
            if shift < 35 {
                value |= (byte >> 1) << shift;
            }
            shift += 7;
            if byte & 1 == 0 {
                break;
            }
            if shift > 35 {
                self.set_error(BitError::Malformed("packed int longer than 5 bytes"));
                return 0;
            }
        }
        value & 0xFFFF_FFFF
    }

    /// ue FBitReader::SerializeInt mirror of BitWriter::append_serialize_int.
    pub fn read_serialize_int(&mut self, value_max: u64) -> u64 {
        let mut value = 0u64;
        let mut mask = 1u64;
        while value + mask < value_max && mask < 1u64 << 32 {
            if self.read_bool() {
                value |= mask;
            }
            mask <<= 1;
        }
        value
    }

    pub fn read_u32(&mut self) -> u32 {
        self.read_bits(32) as u32
    }

    pub fn read_i32(&mut self) -> i32 {
        self.read_bits(32) as u32 as i32
    }

    pub fn read_f32(&mut self) -> f32 {
        f32::from_bits(self.read_bits(32) as u32)
    }

    pub fn read_f64(&mut self) -> f64 {
        f64::from_bits(self.read_bits(64))
    }

    /// ue FString: positive length = ansi incl. nul, negative = utf-16 incl. nul.
    pub fn read_fstring(&mut self) -> String {
        let length = self.read_i32();
        if self.error.is_some() || length == 0 {
            return String::new();
        }
        if length.unsigned_abs() > 1 << 16 {
            self.set_error(BitError::Malformed("fstring length"));
            return String::new();
        }
        if length > 0 {
            let mut out = Vec::with_capacity(length as usize);
            for _ in 0..length {
                out.push(self.read_bits(8) as u8);
            }
            if out.last() == Some(&0) {
                out.pop();
            }
            String::from_utf8_lossy(&out).into_owned()
        } else {
            let mut units = Vec::with_capacity(length.unsigned_abs() as usize);
            for _ in 0..length.unsigned_abs() {
                units.push(self.read_bits(16) as u16);
            }
            if units.last() == Some(&0) {
                units.pop();
            }
            String::from_utf16_lossy(&units)
        }
    }

    /// copies the next bit_count bits out as their own bit string.
    pub fn read_payload(&mut self, bit_count: usize) -> Bits {
        if self.position + bit_count > self.bit_count {
            self.set_error(BitError::Overflow { wanted: bit_count, position: self.position, total: self.bit_count });
            return Bits::empty();
        }
        let mut bytes = Vec::with_capacity(bit_count.div_ceil(8));
        let mut left = bit_count;
        while left > 0 {
            let take = left.min(8) as u32;
            bytes.push(self.read_bits(take) as u8);
            left -= take as usize;
        }
        Bits { bytes, bit_count }
    }
}
