//! the move-ack block (python 4412-4535), run on every session-ack tick after
//! the stage chain: the first ack is a ClientMoveResponsePacked placement
//! correction, every later one ClientAckInputAndMovement(0xffffffff, move).
//! unreliable on ch8. docs/port/04_bringup_b.md section 9.

use aries_bits::Bits;
use aries_net::control::BunchInfo;
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

/// the input-ack trial (python 1496-1563), every authenticated datagram whose
/// first bunch is on the pawn channel: ack only event ids parsed off this
/// packet's wire, never the client's generated cursor. the generated bound only
/// guards against a stray 0xA3 marker inside another rpc's payload.
pub fn run_input_ack(reply: &mut Reply, config: &RangeConfig, live: &mut dyn LiveClient, decoded: &Bits, first_bunch: Option<&BunchInfo>) {
    if !config.features.input_ack || !reply.conn.bringup.sent(Stage::ClientRestart) {
        return;
    }
    let Some(bunch) = first_bunch.filter(|b| b.channel_index == 8) else {
        return;
    };
    // the first bunch's bounds are not checked against the payload size, so read
    // the bytes as python holds them (trailer bits restored)
    let packet = aries_rep::decode::with_client_trailer(decoded);
    let streams = aries_rep::decode::decode_received_actor_inputs(&packet, bunch.body_start_bit, bunch.end_bit, None).unwrap_or_default();
    let [stream] = streams.as_slice() else {
        return;
    };
    let latest = stream.last_event_id;
    match live.latest_generated_input_event() {
        Ok(Some(bound)) if latest <= bound => {}
        _ => {
            reply.event.set("input_ack_rejected_event_id", latest);
            return;
        }
    }
    let activations: Vec<_> = stream.events.iter().filter(|e| e.kind == 1).map(|e| json!({"event_id": e.event_id, "slot": e.slot, "state": e.state, "action": e.action})).collect();
    if !activations.is_empty() {
        reply.event.set("received_activation_events", activations);
    }
    if reply.conn.moves.last_input_event.is_some_and(|last| latest <= last) {
        return;
    }
    let result = aries_rep::setup::input_ack_body(latest).map_err(|e| e.to_string()).and_then(|body| reply.send_unreliable(8, &body).map_err(|e| e.to_string()));
    match result {
        Ok(length) => {
            reply.event.set("input_ack_received_event_id", latest);
            reply.event.set("input_ack_received_frame_length", length);
            reply.conn.moves.last_input_event = Some(latest);
        }
        Err(error) => reply.event.set("input_ack_error", error),
    }
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
    if let Some([x, y, z]) = moves.newest_location {
        reply.conn.player.ingest(reply.now, reply.now, aries_interp::value::Vec3 { x, y, z });
    }
    let state = &reply.conn.moves;
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
        // python logs the rpc's own bits, the size its content block declares
        let mut header = aries_bits::BitReader::from_bits(&body);
        header.read_bool();
        header.read_bool();
        reply.event.set("move_correction_rpc_bits", header.read_packed_uint());
        reply.conn.moves.correction_sent = true;
        // the correction teleports the pawn: a discontinuity for the position track
        let [x, y, z] = config.spawn_location;
        reply.conn.player.var.reset(reply.now, aries_interp::value::Vec3 { x, y, z });
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
            // python logs these even when they are None
            reply.event.set("move_ack_timestamp", moves.timestamp.map(f64::from));
            reply.event.set("move_ack_oldest", moves.oldest);
            reply.event.set("move_ack_last_acked", moves.last_acked);
        }
        Err(error) => reply.event.set("move_ack_error", error.to_string()),
    }
}
