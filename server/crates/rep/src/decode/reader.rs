//! the python readers: range_client_input.read_bits / read_packed_uint and
//! range_client_rpc._Reader. kept separate from aries_bits::BitReader because
//! these fail hard (no sticky zero reads), keep packed values unmasked (35
//! bits) and decode fstrings the python way.

use super::{DecodeError, DecodeResult};

/// range_client_input.py:14-22. lsb-first, bounded by the byte length only.
pub fn read_bits(data: &[u8], offset: usize, width: usize) -> DecodeResult<u64> {
    debug_assert!(width <= 64);
    if offset + width > data.len() * 8 {
        return Err(DecodeError::OutsideData);
    }
    let mut value = 0u64;
    let mut done = 0usize;
    let mut position = offset;
    while done < width {
        let shift = position & 7;
        let take = (width - done).min(8 - shift);
        let part = (data[position >> 3] as u64 >> shift) & ((1u64 << take) - 1);
        value |= part << done;
        done += take;
        position += take;
    }
    Ok(value)
}

/// range_client_input.py:25-35: ue SerializeIntPacked, at most five bytes,
/// value not masked (a fifth byte can carry bits 28..34). returns (value, end).
pub fn read_packed_uint(data: &[u8], offset: usize, limit: usize) -> DecodeResult<(u64, usize)> {
    let mut value = 0u64;
    let mut position = offset;
    for shift in (0..35).step_by(7) {
        if position + 8 > limit {
            return Err(DecodeError::PackedTruncated);
        }
        let byte = read_bits(data, position, 8)?;
        position += 8;
        value |= (byte >> 1) << shift;
        if byte & 1 == 0 {
            return Ok((value, position));
        }
    }
    Err(DecodeError::PackedTooLong)
}

/// range_client_rpc._Reader (:39-71): a cursor with a hard limit. the limit
/// can sit past the data (rpc params trust the sender's size), so a read is
/// checked against the limit first, then against the bytes.
#[derive(Debug, Clone)]
pub struct BoundedReader<'a> {
    data: &'a [u8],
    pub position: usize,
    pub limit: usize,
}

impl<'a> BoundedReader<'a> {
    pub fn new(data: &'a [u8], position: usize, limit: usize) -> Self {
        Self { data, position, limit }
    }

    pub fn take(&mut self, width: usize) -> DecodeResult<u64> {
        if self.position + width > self.limit {
            return Err(DecodeError::ReadPastEnd);
        }
        let value = read_bits(self.data, self.position, width)?;
        self.position += width;
        Ok(value)
    }

    pub fn take_bool(&mut self) -> DecodeResult<bool> {
        Ok(self.take(1)? != 0)
    }

    /// _Reader.packed: read_packed_uint against this reader's limit.
    pub fn packed(&mut self) -> DecodeResult<u64> {
        let (value, position) = read_packed_uint(self.data, self.position, self.limit)?;
        self.position = position;
        Ok(value)
    }

    /// range_client_rpc._packed_header_field (:74-81): the bunch header's packed
    /// reader. same bytes as packed(), but byte reads use take() and the
    /// overlong error has its own text.
    pub fn header_packed(&mut self) -> DecodeResult<u64> {
        let mut value = 0u64;
        for shift in (0..35).step_by(7) {
            let byte = self.take(8)?;
            value |= (byte >> 1) << shift;
            if byte & 1 == 0 {
                return Ok(value);
            }
        }
        Err(DecodeError::HeaderPackedTooLong)
    }

    /// _Reader.serialize_int (:54-61): FBitReader::SerializeInt, lsb-first
    /// while value + mask < maximum. the width depends on the value.
    pub fn serialize_int(&mut self, maximum: u64) -> DecodeResult<u64> {
        // u128 so the mask cannot wrap before python's unbounded int would stop
        let maximum = maximum as u128;
        let (mut value, mut mask) = (0u128, 1u128);
        while value + mask < maximum {
            if self.take_bool()? {
                value |= mask;
            }
            mask <<= 1;
        }
        Ok(value as u64)
    }

    /// _Reader.fstring (:63-71). negative length = utf-16 code units, else
    /// ascii bytes (at most 1024); trailing nuls stripped either way.
    pub fn fstring(&mut self) -> DecodeResult<String> {
        let length = self.take(32)?;
        if length & 0x8000_0000 != 0 {
            let units = (1u64 << 32) - length;
            let mut raw = Vec::new();
            for _ in 0..2 * units {
                raw.push(self.take(8)? as u8);
            }
            let wide: Vec<u16> = raw.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect();
            let text = String::from_utf16(&wide).map_err(|_| DecodeError::BadString)?;
            return Ok(text.trim_end_matches('\0').to_string());
        }
        if length > 1024 {
            return Err(DecodeError::FStringLength);
        }
        let mut raw = Vec::with_capacity(length as usize);
        for _ in 0..length {
            raw.push(self.take(8)? as u8);
        }
        if !raw.is_ascii() {
            return Err(DecodeError::BadString);
        }
        let text = String::from_utf8(raw).map_err(|_| DecodeError::BadString)?;
        Ok(text.trim_end_matches('\0').to_string())
    }
}
