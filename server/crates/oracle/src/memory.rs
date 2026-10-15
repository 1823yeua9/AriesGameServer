//! the memory-read interface every oracle reader uses. Process implements it
//! over ReadProcessMemory; FakeMemory lets tests lay out synthetic objects.

use std::collections::BTreeMap;

use crate::OracleError;
use crate::process::valid_address;

pub trait Memory {
    /// bounded read; fails on short reads like the python read_bytes.
    fn read_into(&self, address: u64, buffer: &mut [u8]) -> Result<(), OracleError>;

    fn image_base(&self) -> u64;

    fn read_bytes(&self, address: u64, size: usize) -> Result<Vec<u8>, OracleError> {
        let mut buffer = vec![0u8; size];
        self.read_into(address, &mut buffer)?;
        Ok(buffer)
    }

    fn read<const N: usize>(&self, address: u64) -> Result<[u8; N], OracleError> {
        let mut buffer = [0u8; N];
        self.read_into(address, &mut buffer)?;
        Ok(buffer)
    }

    fn u64(&self, address: u64) -> Result<u64, OracleError> {
        Ok(u64::from_le_bytes(self.read(address)?))
    }

    fn u32(&self, address: u64) -> Result<u32, OracleError> {
        Ok(u32::from_le_bytes(self.read(address)?))
    }

    fn i32(&self, address: u64) -> Result<i32, OracleError> {
        Ok(i32::from_le_bytes(self.read(address)?))
    }

    fn u16(&self, address: u64) -> Result<u16, OracleError> {
        Ok(u16::from_le_bytes(self.read(address)?))
    }

    fn u8(&self, address: u64) -> Result<u8, OracleError> {
        Ok(self.read::<1>(address)?[0])
    }

    fn f32(&self, address: u64) -> Result<f32, OracleError> {
        Ok(f32::from_le_bytes(self.read(address)?))
    }

    fn f64(&self, address: u64) -> Result<f64, OracleError> {
        Ok(f64::from_le_bytes(self.read(address)?))
    }

    fn vec3(&self, address: u64) -> Result<[f64; 3], OracleError> {
        let raw: [u8; 24] = self.read(address)?;
        Ok(std::array::from_fn(|i| f64::from_le_bytes(raw[i * 8..i * 8 + 8].try_into().unwrap())))
    }

    /// reads a pointer; a null or non-canonical value is an error.
    fn ptr(&self, address: u64) -> Result<u64, OracleError> {
        let value = self.u64(address)?;
        if !valid_address(value) {
            return Err(OracleError::Null(address));
        }
        Ok(value)
    }

    /// pointer that may be null (returns 0).
    fn ptr_or_null(&self, address: u64) -> Result<u64, OracleError> {
        let value = self.u64(address)?;
        Ok(if valid_address(value) { value } else { 0 })
    }

}

/// the largest block; Process refuses bigger reads too
pub const MAX_BLOCK: u64 = 0x10_0000;

/// one ReadProcessMemory of an object's fields [base + start, base + end), parsed
/// locally. when the block read fails (a page in it is not mapped) each field
/// falls back to its own read, so a field fails exactly when the python's single
/// read of it would; a block never changes a result, only the read count.
#[derive(Debug, Clone)]
pub struct Block {
    base: u64,
    start: u64,
    bytes: Option<Vec<u8>>,
}

impl Block {
    pub fn read(memory: &(impl Memory + ?Sized), base: u64, start: u64, end: u64) -> Self {
        let address = base.saturating_add(start);
        let size = end.saturating_sub(start);
        // a null or junk base, or a junk count from client memory, falls back to
        // the fields' own reads instead of a wasted read or a huge allocation
        let bytes = if valid_address(address) && size <= MAX_BLOCK { memory.read_bytes(address, size as usize).ok() } else { None };
        Self { base, start, bytes }
    }

    /// `len` bytes at `offset`, from the block when it covers them.
    pub fn bytes(&self, memory: &(impl Memory + ?Sized), offset: u64, len: usize) -> Result<Vec<u8>, OracleError> {
        if let (Some(bytes), Some(from)) = (&self.bytes, offset.checked_sub(self.start))
            && let Some(slice) = bytes.get(from as usize..from as usize + len)
        {
            return Ok(slice.to_vec());
        }
        memory.read_bytes(self.base.saturating_add(offset), len)
    }

    /// true when the one read covered it (the fallback is per field)
    pub fn loaded(&self) -> bool {
        self.bytes.is_some()
    }

    pub fn get<const N: usize>(&self, memory: &(impl Memory + ?Sized), offset: u64) -> Result<[u8; N], OracleError> {
        if let (Some(bytes), Some(from)) = (&self.bytes, offset.checked_sub(self.start))
            && let Some(slice) = bytes.get(from as usize..from as usize + N)
        {
            return Ok(slice.try_into().expect("N bytes"));
        }
        memory.read(self.base.saturating_add(offset))
    }

    pub fn u64(&self, memory: &(impl Memory + ?Sized), offset: u64) -> Result<u64, OracleError> {
        Ok(u64::from_le_bytes(self.get(memory, offset)?))
    }

    pub fn u32(&self, memory: &(impl Memory + ?Sized), offset: u64) -> Result<u32, OracleError> {
        Ok(u32::from_le_bytes(self.get(memory, offset)?))
    }

    pub fn i32(&self, memory: &(impl Memory + ?Sized), offset: u64) -> Result<i32, OracleError> {
        Ok(i32::from_le_bytes(self.get(memory, offset)?))
    }

    pub fn u16(&self, memory: &(impl Memory + ?Sized), offset: u64) -> Result<u16, OracleError> {
        Ok(u16::from_le_bytes(self.get(memory, offset)?))
    }

    pub fn u8(&self, memory: &(impl Memory + ?Sized), offset: u64) -> Result<u8, OracleError> {
        Ok(self.get::<1>(memory, offset)?[0])
    }

    pub fn vec3(&self, memory: &(impl Memory + ?Sized), offset: u64) -> Result<[f64; 3], OracleError> {
        let raw: [u8; 24] = self.get(memory, offset)?;
        Ok(std::array::from_fn(|i| f64::from_le_bytes(raw[i * 8..i * 8 + 8].try_into().unwrap())))
    }
}

/// counts every read that reaches the wrapped memory (tests and the read budget).
pub struct Counted<M> {
    pub inner: M,
    pub reads: std::cell::Cell<u64>,
}

impl<M> Counted<M> {
    pub fn new(inner: M) -> Self {
        Self { inner, reads: std::cell::Cell::new(0) }
    }

    pub fn take(&self) -> u64 {
        self.reads.replace(0)
    }
}

impl<M: Memory> Memory for Counted<M> {
    fn read_into(&self, address: u64, buffer: &mut [u8]) -> Result<(), OracleError> {
        self.reads.set(self.reads.get() + 1);
        self.inner.read_into(address, buffer)
    }

    fn image_base(&self) -> u64 {
        self.inner.image_base()
    }
}

/// sparse fake address space for tests: write bytes at addresses, read them back.
#[derive(Default, Clone)]
pub struct FakeMemory {
    pub base: u64,
    regions: BTreeMap<u64, Vec<u8>>,
}

impl FakeMemory {
    pub fn new(base: u64) -> Self {
        Self { base, regions: BTreeMap::new() }
    }

    pub fn write(&mut self, address: u64, bytes: &[u8]) {
        for (i, &b) in bytes.iter().enumerate() {
            let a = address + i as u64;
            let page = a & !0xFFF;
            let region = self.regions.entry(page).or_insert_with(|| vec![0u8; 0x1000]);
            region[(a - page) as usize] = b;
        }
    }

    pub fn write_u64(&mut self, address: u64, value: u64) {
        self.write(address, &value.to_le_bytes());
    }

    pub fn write_u32(&mut self, address: u64, value: u32) {
        self.write(address, &value.to_le_bytes());
    }

    pub fn write_f64(&mut self, address: u64, value: f64) {
        self.write(address, &value.to_le_bytes());
    }
}

impl Memory for FakeMemory {
    fn read_into(&self, address: u64, buffer: &mut [u8]) -> Result<(), OracleError> {
        if !valid_address(address) {
            return Err(OracleError::BadAddress(address));
        }
        for (i, slot) in buffer.iter_mut().enumerate() {
            let a = address + i as u64;
            let page = a & !0xFFF;
            let region = self.regions.get(&page).ok_or(OracleError::Read(address, i))?;
            *slot = region[(a - page) as usize];
        }
        Ok(())
    }

    fn image_base(&self) -> u64 {
        self.base
    }
}
