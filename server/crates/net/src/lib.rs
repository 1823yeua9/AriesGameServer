//! ares transport for the pinned 13.06 client: the packet transform, gcm with
//! 4-byte tags, framing, and (soon) the stateless handshake and channels.

pub mod client;
pub mod control;
pub mod frame;
pub mod gcm;
pub mod handshake;
pub mod reliable;
pub mod keys;
pub mod sbox;
pub mod transform;

pub use frame::{BunchHeader, ClientPacket, FrameError, PacketInfo, build_packet_bits, build_server_frame, decode_client_packet, seal_packet};
pub use gcm::AresGcm;
