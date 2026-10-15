//! Zero-wait reader of the internal module's process-local geometry snapshot.
//! No game writes and no disk I/O on the shot path.

#[cfg(windows)]
mod platform {
    use aries_world::collision::Snapshot;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_ABANDONED};
    use windows_sys::Win32::System::Memory::{OpenFileMappingW, MapViewOfFile, UnmapViewOfFile, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS};
    use windows_sys::Win32::System::Threading::{OpenMutexW, ReleaseMutex, WaitForSingleObject, SYNCHRONIZATION_SYNCHRONIZE, MUTEX_MODIFY_STATE};

    pub struct Reader {
        mapping: HANDLE,
        mutex: HANDLE,
        view: MEMORY_MAPPED_VIEW_ADDRESS,
    }

    impl Reader {
        pub fn open(pid: u32) -> Result<Self, String> {
            let name = format!("Local\\AriesCollision_{pid}");
            let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            unsafe {
                let mapping = OpenFileMappingW(FILE_MAP_READ, 0, wide(&name).as_ptr());
                if mapping.is_null() { return Err("Internal collision feed not available".into()); }
                let mutex = OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE | MUTEX_MODIFY_STATE, 0, wide(&(name + "_lock")).as_ptr());
                if mutex.is_null() {
                    CloseHandle(mapping);
                    return Err("Internal collision feed lock not available".into());
                }
                let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, size_of::<Snapshot>());
                if view.Value.is_null() {
                    CloseHandle(mutex);
                    CloseHandle(mapping);
                    return Err("Internal collision feed layout unavailable".into());
                }
                Ok(Self { mapping, mutex, view })
            }
        }

        pub fn snapshot(&self) -> Result<Box<Snapshot>, String> {
            // Allocate before acquiring the mutex. The producer is never blocked
            // by allocation, validation, engine reads, or intersection tests.
            let mut copy = Box::<Snapshot>::new_uninit();
            unsafe {
                let wait = WaitForSingleObject(self.mutex, 0);
                if wait == WAIT_ABANDONED {
                    ReleaseMutex(self.mutex);
                    return Err("Collision producer abandoned an incomplete sample".into());
                }
                if wait != WAIT_OBJECT_0 { return Err("Collision snapshot busy".into()); }
                std::ptr::copy_nonoverlapping(self.view.Value.cast::<Snapshot>(), copy.as_mut_ptr(), 1);
                ReleaseMutex(self.mutex);
                let copy = copy.assume_init();
                if copy.version != 1 || copy.count as usize > aries_world::collision::MAX_TARGETS {
                    return Err("Collision snapshot version/count invalid".into());
                }
                Ok(copy)
            }
        }
    }

    impl Drop for Reader {
        fn drop(&mut self) {
            unsafe {
                UnmapViewOfFile(self.view);
                CloseHandle(self.mapping);
                CloseHandle(self.mutex);
            }
        }
    }

    pub fn now_ms() -> u64 {
        unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() }
    }
}

#[cfg(not(windows))]
mod platform {
    pub struct Reader;
    impl Reader {
        pub fn open(_: u32) -> Result<Self, String> { Err("Collision feed requires Windows".into()) }
        pub fn snapshot(&self) -> Result<Box<aries_world::collision::Snapshot>, String> { Err("Collision feed requires Windows".into()) }
    }
    pub fn now_ms() -> u64 { 0 }
}

pub use platform::{Reader, now_ms};
