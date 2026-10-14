//! session keys: scan the client's private memory for the ares packet-handler
//! object (its first qword is the handler vtable) and read both aes keys from
//! it. mirrors check_live_packet_keys.read_current_handler_keys (CLK:42-82,
//! read-only). docs/port/01_transport.md section 7, 09_oracle.md R1.

use std::time::{Duration, Instant};

use crate::OracleError;

/// rva of the ares packet-handler vtable in .rdata.
pub const HANDLER_VTABLE_RVA: u64 = 0xA5F_3BB0;
/// client -> server key (decrypts what the client sends)
pub const CLIENT_SEND_KEY_OFFSET: usize = 0x28;
/// server -> client key (python receive_key_override)
pub const SERVER_SEND_KEY_OFFSET: usize = 0x48;
/// the handler is re-read whole before its keys are trusted (CLK:72)
const HANDLER_READ: usize = 112;
/// CLK:64 1 MiB chunks, no overlap (a needle across a boundary is missed, like python)
const SCAN_CHUNK: usize = 1 << 20;
/// CLK:52 4096 MiB of bytes actually read, checked per region only
const SCAN_LIMIT: u64 = 4096 << 20;
const SCAN_START: u64 = 0x10000;
const SCAN_END: u64 = 0x7FFF_FFFF_0000;
/// MEM_COMMIT, MEM_PRIVATE; PAGE_NOACCESS | PAGE_GUARD excluded, any readable protection (CLK:59-60)
const MEM_COMMIT: u32 = 0x1000;
const MEM_PRIVATE: u32 = 0x20000;
const PROTECT_EXCLUDED: u32 = 0x101;
const PROTECT_READABLE: u32 = 0xEE;

#[derive(Clone)]
pub struct HandlerKeys {
    pub handler: u64,
    pub client_send: [u8; 32],
    pub server_send: [u8; 32],
}

impl std::fmt::Debug for HandlerKeys {
    // never print key bytes
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HandlerKeys {{ handler: {:#x}, .. }}", self.handler)
    }
}

/// the MEMORY_BASIC_INFORMATION fields the scan uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub base: u64,
    pub size: u64,
    pub state: u32,
    pub kind: u32,
    pub protect: u32,
}

/// what the scan needs from a process: VirtualQueryEx and ReadProcessMemory.
pub trait ScanSource {
    /// None when VirtualQueryEx returns 0
    fn query(&self, address: u64) -> Option<Region>;
    /// bytes read, None when ReadProcessMemory fails
    fn read(&self, address: u64, buffer: &mut [u8]) -> Option<usize>;
}

/// read_current_handler_keys' scan (CLK:42-82): ascending regions, 1 MiB chunks,
/// every byte offset, first hit whose 112-byte re-read still starts with the needle.
pub fn scan_handler_keys(source: &impl ScanSource, handler_vtable: u64, max_seconds: f64) -> Result<HandlerKeys, OracleError> {
    let needle = handler_vtable.to_le_bytes();
    let started = Instant::now();
    let deadline = Duration::from_secs_f64(max_seconds);
    let mut scanned = 0u64;
    let mut address = SCAN_START;
    let mut chunk = vec![0u8; SCAN_CHUNK];
    while address < SCAN_END && started.elapsed() < deadline && scanned < SCAN_LIMIT {
        let Some(region) = source.query(address) else {
            break;
        };
        // CLK:56 `(BaseAddress or addr) + RegionSize`
        let start = if region.base != 0 { region.base } else { address };
        let next = start.saturating_add(region.size);
        if next <= address {
            break;
        }
        let readable = region.protect & PROTECT_EXCLUDED == 0 && region.protect & PROTECT_READABLE != 0;
        if region.state == MEM_COMMIT && region.kind == MEM_PRIVATE && readable {
            let mut chunk_address = address;
            while chunk_address < next && started.elapsed() < deadline {
                let size = (next - chunk_address).min(SCAN_CHUNK as u64) as usize;
                if let Some(got) = source.read(chunk_address, &mut chunk[..size]) {
                    let data = &chunk[..got.min(size)];
                    let mut i = 0usize;
                    while i + 8 <= data.len() {
                        // prefilter on the first byte, then compare the full qword
                        match data[i..data.len() - 7].iter().position(|&b| b == needle[0]) {
                            Some(p) => i += p,
                            None => break,
                        }
                        if data[i..i + 8] == needle {
                            let hit = chunk_address + i as u64;
                            let mut handler = [0u8; HANDLER_READ];
                            if source.read(hit, &mut handler) == Some(HANDLER_READ) && handler[..8] == needle {
                                let mut keys = HandlerKeys { handler: hit, client_send: [0; 32], server_send: [0; 32] };
                                keys.client_send.copy_from_slice(&handler[CLIENT_SEND_KEY_OFFSET..CLIENT_SEND_KEY_OFFSET + 32]);
                                keys.server_send.copy_from_slice(&handler[SERVER_SEND_KEY_OFFSET..SERVER_SEND_KEY_OFFSET + 32]);
                                return Ok(keys);
                            }
                        }
                        i += 1;
                    }
                    // only bytes actually read count toward the cap (CLK:77)
                    scanned += got as u64;
                }
                chunk_address += size as u64;
            }
        }
        address = next;
    }
    Err(OracleError::Invalid("current Ares handler key was not found within read-only scan bounds"))
}

#[cfg(windows)]
struct ProcessScan {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl ScanSource for ProcessScan {
    fn query(&self, address: u64) -> Option<Region> {
        use windows_sys::Win32::System::Memory::{MEMORY_BASIC_INFORMATION, VirtualQueryEx};
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let size = unsafe { VirtualQueryEx(self.handle, address as *const _, &mut info, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) };
        (size != 0).then_some(Region { base: info.BaseAddress as u64, size: info.RegionSize as u64, state: info.State, kind: info.Type, protect: info.Protect })
    }

    fn read(&self, address: u64, buffer: &mut [u8]) -> Option<usize> {
        use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
        let mut count = 0usize;
        let ok = unsafe { ReadProcessMemory(self.handle, address as *const _, buffer.as_mut_ptr().cast(), buffer.len(), &mut count) };
        (ok != 0).then_some(count)
    }
}

/// finds the handler and reads both keys. first valid hit (ascending address) wins.
#[cfg(windows)]
pub fn read_handler_keys(pid: u32, image_base: u64, max_seconds: f64) -> Result<HandlerKeys, OracleError> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

    // 0x0410: VirtualQueryEx needs QUERY_INFORMATION (CLK:44)
    let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
    if handle.is_null() {
        return Err(OracleError::Open(pid));
    }
    let result = scan_handler_keys(&ProcessScan { handle }, image_base.wrapping_add(HANDLER_VTABLE_RVA), max_seconds);
    unsafe {
        CloseHandle(handle);
    }
    result
}

#[cfg(not(windows))]
pub fn read_handler_keys(pid: u32, _image_base: u64, _max_seconds: f64) -> Result<HandlerKeys, OracleError> {
    Err(OracleError::Open(pid))
}

/// sha256(key)[:12] hex, the only thing about a key we ever log (python fingerprint).
pub fn fingerprint(key: &[u8; 32]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(key).iter().take(6).map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn fingerprint_matches_python() {
        // python: hashlib.sha256(bytes(range(32))).hexdigest()[:12]
        let key: [u8; 32] = std::array::from_fn(|i| i as u8);
        assert_eq!(super::fingerprint(&key), "630dcd2966c4");
    }
}
