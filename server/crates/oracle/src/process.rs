//! read-only access to the client process (ReadProcessMemory only; we never write).

use crate::OracleError;

/// PROCESS_VM_READ | PROCESS_QUERY_LIMITED_INFORMATION, same as the python.
#[cfg(windows)]
const READ_ACCESS: u32 = 0x0010 | 0x1000;
/// user-mode canonical address range the python bounds every read to.
const MIN_ADDRESS: u64 = 0x10000;
const MAX_ADDRESS: u64 = 0x0000_7FFF_FFFF_FFFF;

pub struct Process {
    pub pid: u32,
    pub image_base: u64,
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

// the handle is only used for ReadProcessMemory, which is thread safe
unsafe impl Send for Process {}
unsafe impl Sync for Process {}

impl Process {
    #[cfg(windows)]
    pub fn open(pid: u32, image_base: u64) -> Result<Self, OracleError> {
        use windows_sys::Win32::System::Threading::OpenProcess;
        let handle = unsafe { OpenProcess(READ_ACCESS, 0, pid) };
        if handle.is_null() {
            return Err(OracleError::Open(pid));
        }
        Ok(Self { pid, image_base, handle })
    }

    #[cfg(not(windows))]
    pub fn open(pid: u32, _image_base: u64) -> Result<Self, OracleError> {
        Err(OracleError::Open(pid))
    }

    pub fn valid_address(address: u64) -> bool {
        (MIN_ADDRESS..=MAX_ADDRESS).contains(&address)
    }

    /// bounded read; fails on short reads like the python read_bytes.
    #[cfg(windows)]
    pub fn read_into(&self, address: u64, buffer: &mut [u8]) -> Result<(), OracleError> {
        use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
        if !Self::valid_address(address) || buffer.len() > 0x10_0000 {
            return Err(OracleError::BadAddress(address));
        }
        let mut count = 0usize;
        let ok = unsafe { ReadProcessMemory(self.handle, address as *const _, buffer.as_mut_ptr().cast(), buffer.len(), &mut count) };
        if ok == 0 || count != buffer.len() {
            return Err(OracleError::Read(address, buffer.len()));
        }
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn read_into(&self, address: u64, _buffer: &mut [u8]) -> Result<(), OracleError> {
        Err(OracleError::BadAddress(address))
    }

    pub fn read_bytes(&self, address: u64, size: usize) -> Result<Vec<u8>, OracleError> {
        let mut buffer = vec![0u8; size];
        self.read_into(address, &mut buffer)?;
        Ok(buffer)
    }

    pub fn read<const N: usize>(&self, address: u64) -> Result<[u8; N], OracleError> {
        let mut buffer = [0u8; N];
        self.read_into(address, &mut buffer)?;
        Ok(buffer)
    }

    pub fn u64(&self, address: u64) -> Result<u64, OracleError> {
        Ok(u64::from_le_bytes(self.read(address)?))
    }

    pub fn u32(&self, address: u64) -> Result<u32, OracleError> {
        Ok(u32::from_le_bytes(self.read(address)?))
    }

    pub fn i32(&self, address: u64) -> Result<i32, OracleError> {
        Ok(i32::from_le_bytes(self.read(address)?))
    }

    pub fn u16(&self, address: u64) -> Result<u16, OracleError> {
        Ok(u16::from_le_bytes(self.read(address)?))
    }

    pub fn u8(&self, address: u64) -> Result<u8, OracleError> {
        Ok(self.read::<1>(address)?[0])
    }

    pub fn f32(&self, address: u64) -> Result<f32, OracleError> {
        Ok(f32::from_le_bytes(self.read(address)?))
    }

    pub fn f64(&self, address: u64) -> Result<f64, OracleError> {
        Ok(f64::from_le_bytes(self.read(address)?))
    }

    pub fn vec3(&self, address: u64) -> Result<[f64; 3], OracleError> {
        let raw: [u8; 24] = self.read(address)?;
        Ok(std::array::from_fn(|i| f64::from_le_bytes(raw[i * 8..i * 8 + 8].try_into().unwrap())))
    }

    /// reads a pointer; a null or non-canonical value is an error.
    pub fn ptr(&self, address: u64) -> Result<u64, OracleError> {
        let value = self.u64(address)?;
        if !Self::valid_address(value) {
            return Err(OracleError::Null(address));
        }
        Ok(value)
    }

    /// pointer that may be null (returns 0).
    pub fn ptr_or_null(&self, address: u64) -> Result<u64, OracleError> {
        let value = self.u64(address)?;
        Ok(if Self::valid_address(value) { value } else { 0 })
    }

    #[cfg(windows)]
    pub fn is_alive(&self) -> bool {
        use windows_sys::Win32::System::Threading::GetExitCodeProcess;
        const STILL_ACTIVE: u32 = 259;
        let mut code = 0u32;
        unsafe { GetExitCodeProcess(self.handle, &mut code) != 0 && code == STILL_ACTIVE }
    }

    #[cfg(not(windows))]
    pub fn is_alive(&self) -> bool {
        false
    }
}

#[cfg(windows)]
impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

/// a running process found by exe name.
#[derive(Debug, Clone)]
pub struct FoundProcess {
    pub pid: u32,
    pub image_base: u64,
    pub path: Option<std::path::PathBuf>,
}

/// every process with this exe name (case-insensitive), with image base and path.
#[cfg(windows)]
pub fn find_processes(exe_name: &str) -> Vec<FoundProcess> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
    let mut found = Vec::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return found;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut more = Process32FirstW(snapshot, &mut entry) != 0;
        while more {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if name.eq_ignore_ascii_case(exe_name) {
                let pid = entry.th32ProcessID;
                let (image_base, path) = module_info(pid);
                found.push(FoundProcess { pid, image_base, path });
            }
            more = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
    }
    found
}

#[cfg(not(windows))]
pub fn find_processes(_exe_name: &str) -> Vec<FoundProcess> {
    Vec::new()
}

/// first module base (the exe) and the full image path.
#[cfg(windows)]
fn module_info(pid: u32) -> (u64, Option<std::path::PathBuf>) {
    use windows_sys::Win32::Foundation::{CloseHandle, HMODULE};
    use windows_sys::Win32::System::ProcessStatus::EnumProcessModules;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ, QueryFullProcessImageNameW};
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
        if handle.is_null() {
            return (0, None);
        }
        let mut modules: [HMODULE; 1] = [std::ptr::null_mut()];
        let mut needed = 0u32;
        let base = if EnumProcessModules(handle, modules.as_mut_ptr(), std::mem::size_of_val(&modules) as u32, &mut needed) != 0 {
            modules[0] as u64
        } else {
            0
        };
        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        let path = (QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut size) != 0)
            .then(|| std::path::PathBuf::from(String::from_utf16_lossy(&buffer[..size as usize])));
        CloseHandle(handle);
        (base, path)
    }
}
