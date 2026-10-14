//! session keys: scan the client's private memory for the ares packet-handler
//! object (its first qword is the handler vtable) and read both aes keys from
//! it. mirrors check_live_packet_keys.read_current_handler_keys (read-only).
//! docs/port/01_transport.md section 7.

use std::time::{Duration, Instant};

use crate::OracleError;

/// rva of the ares packet-handler vtable in .rdata.
pub const HANDLER_VTABLE_RVA: u64 = 0xA5F_3BB0;
/// client -> server key (decrypts what the client sends)
pub const CLIENT_SEND_KEY_OFFSET: usize = 0x28;
/// server -> client key (python receive_key_override)
pub const SERVER_SEND_KEY_OFFSET: usize = 0x48;
const HANDLER_READ: usize = 112;
const SCAN_CHUNK: usize = 1 << 20;
const SCAN_LIMIT: u64 = 4096 << 20;
const SCAN_END: u64 = 0x7FFF_FFFF_0000;

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

/// finds the handler and reads both keys. first valid hit (ascending address) wins.
#[cfg(windows)]
pub fn read_handler_keys(pid: u32, image_base: u64, max_seconds: f64) -> Result<HandlerKeys, OracleError> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_PRIVATE, MEMORY_BASIC_INFORMATION, VirtualQueryEx};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

    let needle = (image_base + HANDLER_VTABLE_RVA).to_le_bytes();
    let handle = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid) };
    if handle.is_null() {
        return Err(OracleError::Open(pid));
    }
    let read = |address: u64, buffer: &mut [u8]| -> usize {
        let mut count = 0usize;
        let ok = unsafe { ReadProcessMemory(handle, address as *const _, buffer.as_mut_ptr().cast(), buffer.len(), &mut count) };
        if ok == 0 { 0 } else { count }
    };
    let started = Instant::now();
    let deadline = Duration::from_secs_f64(max_seconds);
    let mut scanned = 0u64;
    let mut address = 0x10000u64;
    let mut chunk = vec![0u8; SCAN_CHUNK];
    let mut result = Err(OracleError::Invalid("ares packet handler not found"));
    'regions: while address < SCAN_END && started.elapsed() < deadline && scanned < SCAN_LIMIT {
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let size = unsafe { VirtualQueryEx(handle, address as *const _, &mut info, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) };
        if size == 0 {
            break;
        }
        let base = info.BaseAddress as u64;
        let region_size = info.RegionSize as u64;
        let next = base.saturating_add(region_size).max(address + 0x1000);
        let readable = info.Protect & 0xEE != 0 && info.Protect & 0x101 == 0;
        if info.State == MEM_COMMIT && info.Type == MEM_PRIVATE && readable {
            let mut offset = 0u64;
            while offset < region_size {
                let length = (region_size - offset).min(SCAN_CHUNK as u64) as usize;
                let got = read(base + offset, &mut chunk[..length]);
                scanned += length as u64;
                if got >= 8 {
                    let mut i = 0usize;
                    while i + 8 <= got {
                        // prefilter on the first byte, then compare the full qword
                        match chunk[i..got - 7].iter().position(|&b| b == needle[0]) {
                            Some(p) => i += p,
                            None => break,
                        }
                        if chunk[i..i + 8] == needle {
                            let hit = base + offset + i as u64;
                            let mut handler = [0u8; HANDLER_READ];
                            if read(hit, &mut handler) == HANDLER_READ && handler[..8] == needle {
                                let mut keys = HandlerKeys { handler: hit, client_send: [0; 32], server_send: [0; 32] };
                                keys.client_send.copy_from_slice(&handler[CLIENT_SEND_KEY_OFFSET..CLIENT_SEND_KEY_OFFSET + 32]);
                                keys.server_send.copy_from_slice(&handler[SERVER_SEND_KEY_OFFSET..SERVER_SEND_KEY_OFFSET + 32]);
                                result = Ok(keys);
                                break 'regions;
                            }
                        }
                        i += 1;
                    }
                }
                offset += length as u64;
                if started.elapsed() >= deadline || scanned >= SCAN_LIMIT {
                    break 'regions;
                }
            }
        }
        address = next;
    }
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
