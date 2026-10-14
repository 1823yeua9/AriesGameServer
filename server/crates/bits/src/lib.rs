//! ue 5.3 bit archive, matched bit for bit to the python responder's BitWriter.
//!
//! bits are lsb-first inside each byte. writers and readers keep a sticky
//! error (like ue's ArIsError) so call sites stay flat; check it once at the end.

mod quant;
mod reader;
mod writer;

pub use quant::{append_quantized_vector, append_quantized_vector10, append_rotator_bytes, append_rotator_shorts, compress_byte, compress_short, quantize_component};
pub use reader::BitReader;
pub use writer::BitWriter;

/// a finished bit string: python's (body, bit_count) tuple.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Bits {
    pub bytes: Vec<u8>,
    pub bit_count: usize,
}

impl Bits {
    pub fn new(bytes: Vec<u8>, bit_count: usize) -> Result<Self, BitError> {
        if bytes.len() != bit_count.div_ceil(8) {
            return Err(BitError::LengthMismatch { bytes: bytes.len(), bits: bit_count });
        }
        Ok(Self { bytes, bit_count })
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn hex(&self) -> String {
        self.bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn from_hex(hex: &str, bit_count: usize) -> Result<Self, BitError> {
        if !hex.len().is_multiple_of(2) || !hex.is_ascii() {
            return Err(BitError::BadHex);
        }
        let bytes = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| BitError::BadHex))
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(bytes, bit_count)
    }
}

impl std::fmt::Debug for Bits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Bits({} bits, {})", self.bit_count, self.hex())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BitError {
    #[error("value {value:#x} does not fit {width} bits")]
    ValueTooWide { value: u64, width: u32 },
    #[error("packed field outside uint32 range")]
    PackedOutOfRange,
    #[error("serialize int {value} outside [0, {max})")]
    SerializeIntRange { value: u64, max: u64 },
    #[error("payload is {bytes} bytes for {bits} bits")]
    LengthMismatch { bytes: usize, bits: usize },
    #[error("fstring outside bounded ascii range")]
    BadString,
    #[error("non-finite or out-of-range vector component")]
    BadVector,
    #[error("read past the end ({wanted} bits at {position} of {total})")]
    Overflow { wanted: usize, position: usize, total: usize },
    #[error("malformed value: {0}")]
    Malformed(&'static str),
    #[error("bad hex")]
    BadHex,
}
