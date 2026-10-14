//! per-connection state (python keys everything by the client's (ip, port)).

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;

use aries_bits::Bits;
use aries_net::reliable::ReliableTracker;

use crate::authority::AuthorityState;
use crate::bringup::BringupState;
use crate::moves::MoveAckState;

/// how many recent client packet sequences feed the ack history (python deque maxlen 64).
pub const RECENT_SEQUENCES: usize = 64;

/// first dynamic reliable offset per fixed channel = how many fixed bring-up
/// bunches that channel carries (python channel_first_dynamic_offset, production values).
pub const FIRST_DYNAMIC_OFFSETS: [(u32, u32); 12] =
    [(4, 8), (5, 6), (6, 11), (8, 51), (9, 5), (12, 2), (13, 5), (14, 2), (15, 2), (16, 2), (17, 2), (18, 2)];

pub struct Connection {
    pub source: SocketAddr,
    /// stateless handshake done: (challenge timestamp, cookie). gates every session branch.
    pub pending: Option<(f64, [u8; 20])>,
    /// latest 232-bit authenticated packet (the nmt hello), template for our control replies
    pub hello: Option<Bits>,
    pub challenge_sent: bool,
    pub welcome_sent: bool,
    pub join_ack_sent: bool,
    /// next 14-bit server packet sequence
    pub next_server_sequence: u32,
    /// reliable base B: every channel's first reliable bunch is (B + 0) & 0xfff
    pub base_sequence: u32,
    pub recent_client_sequences: VecDeque<u32>,
    pub last_session_ack: Option<f64>,
    /// next dynamic reliable offset per channel (python channel_next_offset)
    pub channel_next_offset: HashMap<u32, u32>,
    /// first dynamic offset per channel; dynamic channels register 0 before their open
    pub channel_first_offset: HashMap<u32, u32>,
    pub bringup: BringupState,
    /// python auth_correct_rep_index: bring-up sends 1..5, purchases and the input-correction responder continue it
    pub correction_index: u32,
    /// reliable bunches in flight, for retransmission (new; python never resent)
    pub reliable: ReliableTracker,
    /// move and input acks
    pub moves: MoveAckState,
    /// the per-packet authority features
    pub authority: AuthorityState,
}

impl Connection {
    pub fn new(source: SocketAddr) -> Self {
        Self {
            source,
            pending: None,
            hello: None,
            challenge_sent: false,
            welcome_sent: false,
            join_ack_sent: false,
            next_server_sequence: 0,
            base_sequence: 0,
            recent_client_sequences: VecDeque::with_capacity(RECENT_SEQUENCES),
            last_session_ack: None,
            channel_next_offset: HashMap::new(),
            channel_first_offset: FIRST_DYNAMIC_OFFSETS.into_iter().collect(),
            bringup: BringupState::default(),
            correction_index: 0,
            reliable: ReliableTracker::default(),
            moves: MoveAckState::default(),
            authority: AuthorityState::default(),
        }
    }

    /// python update_received_packet_history: mask over the remembered sequences, then remember this one.
    pub fn record_client_sequence(&mut self, sequence: u32) -> u32 {
        let mask = aries_net::control::received_history_mask(self.recent_client_sequences.iter().copied(), sequence);
        if self.recent_client_sequences.len() == RECENT_SEQUENCES {
            self.recent_client_sequences.pop_front();
        }
        self.recent_client_sequences.push_back(sequence);
        mask
    }

    /// python take_channel_sequence. none when the channel was never registered (python KeyError).
    pub fn take_channel_sequence(&mut self, channel: u32) -> Option<u32> {
        let offset = match self.channel_next_offset.get(&channel) {
            Some(&offset) => offset,
            None => *self.channel_first_offset.get(&channel)?,
        };
        self.channel_next_offset.insert(channel, offset + 1);
        Some((self.base_sequence + offset) & 0xFFF)
    }

    /// a fresh dynamic channel (purchases, bots, projectiles): first reliable offset 0.
    pub fn register_dynamic_channel(&mut self, channel: u32) {
        self.channel_first_offset.insert(channel, 0);
    }

    /// (B + offset) & 0xfff for the fixed bring-up stages.
    pub fn fixed_sequence(&self, offset: u32) -> u32 {
        (self.base_sequence + offset) & 0xFFF
    }

    pub fn session_tick_due(&self, now: f64) -> bool {
        self.join_ack_sent && self.pending.is_some() && now - self.last_session_ack.unwrap_or(0.0) >= 0.1
    }
}
