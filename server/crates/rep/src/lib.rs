//! replication encoding for the range: the content transform, content blocks,
//! netguid exports, and the builders that make every bunch body we send.

pub mod block;
pub mod content;

pub use block::{Spawn, Target};
pub use content::{ContentError, encode_block_payload, encode_short_actor_rep_payload};

/// errors from any builder.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RepError {
    #[error(transparent)]
    Bits(#[from] aries_bits::BitError),
    #[error(transparent)]
    Content(#[from] ContentError),
    #[error("{0}")]
    Invalid(String),
}

pub type RepResult<T> = Result<T, RepError>;
