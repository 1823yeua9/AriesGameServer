//! decoding what the client sends (spec 06): the walk over every bunch of a
//! decoded client packet, actor content blocks and their transform, the
//! controller rpcs we act on (ServerPurchase, range settings) and the pawn's
//! input-event streams. owner: I4.
//!
//! every function is pure over the decoded packet (`Bits`). reads follow the
//! python readers exactly, including where they stop and what they return on
//! odd input; errors carry python's ValueError text so the logs diff cleanly.
//! data bounds use `bytes.len() * 8` (python `len(data) * 8`), limits use
//! `bit_count`, so a `Bits` whose bytes run past `bit_count` reads like python.

mod bunch;
mod content;
mod input;
mod reader;
mod rpc;

pub use bunch::{BunchView, iter_bunches, with_client_trailer};
pub use content::{ActorContentBlock, CONTROLLER_ACTOR_MAP_VALUE, PAWN_ACTOR_MAP_VALUE, decode_content_payload, read_actor_content_block};
pub use input::{
    INPUT_STREAM_MARKER, InputEvent, InputStream, MAX_INPUT_STREAM_EVENTS, decode_received_actor_inputs, input_event_payload_bits, parse_input_stream_at,
    scan_received_input_streams,
};
pub use reader::{BoundedReader, read_bits, read_packed_uint};
pub use rpc::{
    CLIENT_UPDATE_RANGE_SETTINGS_FIELD, CONTROLLER_CHANNEL, CONTROLLER_FIELD_MAX, PAWN_FIELD_MAX, PurchaseRequest, RANGE_SETTINGS_FIELDS, RangeSettingsRequest,
    RpcHeader, SERVER_END_MISSION_MODULE_FIELD, SERVER_LOAD_AND_SWAP_CHARACTER_FIELD, SERVER_PURCHASE_FIELD, SERVER_REMOVE_FIELD, SERVER_RESET_BOTS_FIELD,
    SERVER_SELL_FIELD, SERVER_TOGGLE_FREE_SHOOTING_FIELD, SERVER_UPDATE_SHOOTING_RANGE_SETTINGS_FIELD, SERVER_UPDATE_TARGET_PRACTICE_SETTINGS_FIELD,
    decode_controller_rpc, decode_purchase_requests, decode_purchase_requests_in, decode_range_settings_requests, decode_range_settings_requests_in,
    decode_range_settings_rpc, read_object_reference, read_rpc_header, take_new_purchases, take_new_range_settings,
};

/// why a decode stopped. the display text is python's exception text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// range_client_rpc._Reader.take past its limit
    #[error("read past end")]
    ReadPastEnd,
    /// range_client_input.read_bits past the end of the bytes
    #[error("bit range outside data")]
    OutsideData,
    #[error("packed integer truncated")]
    PackedTruncated,
    #[error("packed integer exceeds five bytes")]
    PackedTooLong,
    /// the bunch-header packed reader (_packed_header_field) has its own text
    #[error("packed channel field exceeds five bytes")]
    HeaderPackedTooLong,
    #[error("bunch exceeds packet")]
    BunchExceedsPacket,
    #[error("implausible FString length")]
    FStringLength,
    /// python raises UnicodeDecodeError (a ValueError) for non-ascii bytes or bad utf-16
    #[error("fstring is not valid ascii or utf-16")]
    BadString,
    #[error("object outer chain too deep")]
    OuterTooDeep,
    #[error("invalid content payload length")]
    ContentLength,
    #[error("invalid actor bunch bounds")]
    ActorBounds,
    #[error("actor block header truncated")]
    ActorHeaderTruncated,
    #[error("subobject block needs its own NetGUID context")]
    Subobject,
    #[error("actor content length exceeds bunch")]
    ContentExceedsBunch,
    #[error("invalid content bit count")]
    ContentBitCount,
    #[error("parameterless settings RPC carried parameters")]
    ParameterlessSettings,
    #[error("settings RPC {0} parameters did not end at the payload size")]
    SettingsEnd(u64),
}

pub type DecodeResult<T> = Result<T, DecodeError>;

impl From<DecodeError> for crate::RepError {
    fn from(error: DecodeError) -> Self {
        crate::RepError::Invalid(error.to_string())
    }
}
