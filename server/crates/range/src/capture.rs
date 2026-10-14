//! movement packet capture (python 2086-2106, on in production): after the
//! pawn's role is sent, log the decoded bytes of up to 300 pawn packets (at
//! least 0.25 s apart) and of 16 distinct controller packets. no sends.

use std::collections::HashSet;

use aries_bits::Bits;
use aries_net::control::BunchInfo;

use crate::send::Reply;

pub const MOVEMENT_LIMIT: u32 = 300;
pub const MOVEMENT_SPACING: f64 = 0.25;
pub const CONTROLLER_LIMIT: usize = 16;

#[derive(Debug, Default, Clone)]
pub struct CaptureState {
    pub movement_count: u32,
    /// python movement_capture_last_times (default -1.0), on event elapsed seconds
    pub movement_last: Option<f64>,
    /// (reliable, channel sequence) or (unreliable, whole second)
    pub controller_keys: HashSet<(bool, i64)>,
}

/// python writes decoded_packet[0].hex(): the bytes with the client's trailer bits.
fn packet_hex(decoded: &Bits) -> String {
    aries_rep::decode::with_client_trailer(decoded).bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn run_capture(reply: &mut Reply, decoded: &Bits, first_bunch: Option<&BunchInfo>, elapsed: f64) {
    let Some(bunch) = first_bunch else {
        return;
    };
    let state = &mut reply.conn.capture;
    if bunch.channel_index == 8 && state.movement_count < MOVEMENT_LIMIT && elapsed - state.movement_last.unwrap_or(-1.0) >= MOVEMENT_SPACING {
        state.movement_count += 1;
        state.movement_last = Some(elapsed);
        reply.event.set("decoded_packet_hex", packet_hex(decoded));
    }
    let state = &mut reply.conn.capture;
    if bunch.channel_index == 5 {
        let key = match bunch.channel_sequence {
            Some(sequence) => (true, sequence as i64),
            None => (false, elapsed as i64),
        };
        if !state.controller_keys.contains(&key) && state.controller_keys.len() < CONTROLLER_LIMIT {
            state.controller_keys.insert(key);
            reply.event.set("decoded_packet_hex", packet_hex(decoded));
        }
    }
}
