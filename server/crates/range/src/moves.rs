//! the move-ack block (python 4412-4535), run on every session-ack tick after
//! the stage chain: the first ack is a ClientMoveResponsePacked placement
//! correction, every later one ClientAckInputAndMovement(0xffffffff, move).
//! unreliable on ch8. docs/port/04_bringup_b.md section 9.

use serde_json::json;

use crate::bringup::Stage;
use crate::config::RangeConfig;
use crate::live::LiveClient;
use crate::send::Reply;

/// python move_ack gate (last_move_ack_times, >= 0.05 s).
pub const MOVE_ACK_INTERVAL: f64 = 0.05;

#[derive(Debug, Default, Clone)]
pub struct MoveAckState {
    pub last_time: Option<f64>,
    /// python last_move_ack_numbers (default -1)
    pub last_number: Option<u32>,
    /// python move_correction_sent_sources
    pub correction_sent: bool,
    /// python last_input_ack_numbers (default -1)
    pub last_input_event: Option<u32>,
}

pub fn run_move_ack(reply: &mut Reply, config: &RangeConfig, live: &mut dyn LiveClient) {
    if !config.features.move_ack || !reply.conn.bringup.sent(Stage::ClientRestart) {
        return;
    }
    let state = &reply.conn.moves;
    if state.last_time.is_some_and(|t| reply.now - t < MOVE_ACK_INTERVAL) {
        return;
    }
    let moves = match live.saved_moves() {
        Ok(Some(moves)) => moves,
        Ok(None) => return,
        Err(error) => {
            reply.event.set("move_ack_read_error", error);
            return;
        }
    };
    if state.last_number.is_some_and(|last| moves.newest <= last) {
        return;
    }
    let initial = !state.correction_sent && moves.count > 0;
    let body = if initial {
        aries_rep::setup::placement_correction_body(moves.newest, config.spawn_location)
    } else {
        aries_rep::setup::move_ack_body(moves.newest)
    };
    reply.conn.moves.last_number = Some(moves.newest);
    let body = match body {
        Ok(body) => body,
        Err(error) => {
            reply.event.set("move_ack_error", error.to_string());
            return;
        }
    };
    if initial {
        reply.event.set("movement_ack_path", "placement_correction");
        reply.event.set("move_correction_location", json!(config.spawn_location));
        reply.event.set("move_correction_rpc_bits", body.bit_count);
        reply.conn.moves.correction_sent = true;
    } else {
        reply.event.set("movement_ack_path", "input_and_movement");
    }
    match reply.send_unreliable(8, &body) {
        Ok(length) => {
            reply.conn.moves.last_time = Some(reply.now);
            reply.event.set("sent_move_ack_length", length);
            reply.event.set("move_ack_value_source", "number");
            reply.event.set("move_ack_value", moves.newest);
            reply.event.set("move_ack_newest_number", moves.newest);
            reply.event.set("move_ack_count", moves.count);
            if let Some(oldest) = moves.oldest {
                reply.event.set("move_ack_oldest", oldest);
            }
            if let Some(last) = moves.last_acked {
                reply.event.set("move_ack_last_acked", last);
            }
            if let Some(timestamp) = moves.timestamp {
                reply.event.set("move_ack_timestamp", timestamp as f64);
            }
        }
        Err(error) => reply.event.set("move_ack_error", error.to_string()),
    }
}
