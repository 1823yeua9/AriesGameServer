//! the oracle: everything the server learns from the client process instead of
//! the protocol. read-only (ReadProcessMemory); never writes, never injects.

pub mod client;
pub mod equippable;
pub mod input;
pub mod keys;
pub mod memory;
pub mod moves;
pub mod objects;
pub mod probes;
pub mod process;
pub mod rpc_index;
pub mod tags;
pub mod world;

pub use memory::{Block, Counted, FakeMemory, Memory};
pub use objects::{GObjects, NamePool, decode_array_pointer};
pub use process::{FoundProcess, Process, find_processes};

#[derive(Debug, Clone, thiserror::Error)]
pub enum OracleError {
    #[error("OpenProcess({0}) failed")]
    Open(u32),
    #[error("invalid bounded read at {0:#x}")]
    BadAddress(u64),
    #[error("ReadProcessMemory failed at {0:#x} ({1} bytes)")]
    Read(u64, usize),
    #[error("null pointer read from {0:#x}")]
    Null(u64),
    #[error("{0}")]
    Invalid(&'static str),
}
