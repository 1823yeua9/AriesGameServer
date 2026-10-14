//! the reply context for one received datagram. every frame we send in
//! response carries that datagram's client sequence, ack mask and stateless
//! header, and takes the connection's next server packet sequence.

use std::net::SocketAddr;

use aries_bits::Bits;
use aries_net::control::session_ack_packet;
use aries_net::frame::ACTOR_CHANNEL_NAME;
use aries_net::reliable::ReliableBunch;
use aries_net::{AresGcm, BunchHeader, FrameError, PacketInfo, build_server_frame, seal_packet};

use crate::conn::Connection;
use crate::event::Event;

/// datagrams waiting to go out this tick.
#[derive(Default)]
pub struct Outbox {
    pub datagrams: Vec<(SocketAddr, Vec<u8>)>,
}

#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error("channel {0} has no reliable sequence registered")]
    UnregisteredChannel(u32),
}

pub struct Reply<'a> {
    pub conn: &'a mut Connection,
    pub gcm: &'a AresGcm,
    pub out: &'a mut Outbox,
    pub event: &'a mut Event,
    pub now: f64,
    pub client_sequence: u32,
    pub ack_mask: u32,
    pub stateless_header: u32,
}

impl Reply<'_> {
    fn info(&self) -> PacketInfo {
        PacketInfo {
            server_sequence: self.conn.next_server_sequence,
            acked_client_sequence: self.client_sequence,
            ack_history_mask: self.ack_mask,
            stateless_header: self.stateless_header,
        }
    }

    fn push(&mut self, datagram: Vec<u8>) -> usize {
        let length = datagram.len();
        self.out.datagrams.push((self.conn.source, datagram));
        self.conn.next_server_sequence = (self.conn.next_server_sequence + 1) & 0x3FFF;
        length
    }

    /// one bunch, one datagram. returns the datagram length. reliable bunches are
    /// remembered until the client acks the packet that carried them.
    pub fn send_bunch(&mut self, header: &BunchHeader, body: &Bits) -> Result<usize, SendError> {
        let sequence = self.conn.next_server_sequence;
        let (datagram, _) = build_server_frame(body, header, &self.info(), self.gcm)?;
        let tracked = if header.is_reliable { vec![ReliableBunch { header: header.clone(), body: body.clone() }] } else { Vec::new() };
        self.conn.reliable.sent(sequence, self.now, tracked);
        Ok(self.push(datagram))
    }

    /// applies the client's ack of our packets and resends reliable bunches it
    /// reports lost (original channel sequence and body, in a new packet).
    pub fn resend_lost(&mut self, acked: u32, history: u32) {
        let lost = self.conn.reliable.on_ack(acked, history, self.now);
        for bunch in lost {
            match self.send_bunch(&bunch.header, &bunch.body) {
                Ok(_) => self.event.push("resent_reliable", serde_json::json!([bunch.header.channel_index, bunch.header.channel_sequence])),
                Err(error) => self.event.set("resend_error", error.to_string()),
            }
        }
    }

    /// reliable bunch at a fixed bring-up offset: sequence (B + offset) & 0xfff.
    pub fn send_fixed(&mut self, channel: u32, offset: u32, body: &Bits, has_exports: bool, is_open: bool) -> Result<usize, SendError> {
        let header = BunchHeader {
            channel_index: channel,
            channel_name_index: ACTOR_CHANNEL_NAME,
            channel_sequence: self.conn.fixed_sequence(offset),
            is_open,
            is_reliable: true,
            has_exports,
            ..Default::default()
        };
        self.send_bunch(&header, body)
    }

    /// python send_reliable: next dynamic sequence on the channel.
    pub fn send_reliable(&mut self, channel: u32, body: &Bits, has_exports: bool, is_open: bool, is_close: bool) -> Result<usize, SendError> {
        let sequence = self.conn.take_channel_sequence(channel).ok_or(SendError::UnregisteredChannel(channel))?;
        let header = BunchHeader {
            channel_index: channel,
            channel_name_index: ACTOR_CHANNEL_NAME,
            channel_sequence: sequence,
            is_open,
            is_close,
            is_reliable: true,
            has_exports,
            ..Default::default()
        };
        self.send_bunch(&header, body)
    }

    /// closes a channel (reason 0, destroyed) with an empty reliable bunch.
    pub fn send_close(&mut self, channel: u32) -> Result<usize, SendError> {
        self.send_reliable(channel, &Bits::empty(), false, false, true)
    }

    /// unreliable bunch on an open channel (move and input acks).
    pub fn send_unreliable(&mut self, channel: u32, body: &Bits) -> Result<usize, SendError> {
        self.send_bunch(&BunchHeader::unreliable(channel), body)
    }

    /// the 77-bit ack-only packet with this datagram's history mask.
    pub fn send_session_ack(&mut self, history_mask: u32) -> Result<usize, SendError> {
        let sequence = self.conn.next_server_sequence;
        let packet = session_ack_packet(sequence, self.client_sequence, history_mask);
        let datagram = seal_packet(&packet, self.stateless_header, self.gcm)?;
        self.conn.reliable.sent(sequence, self.now, Vec::new());
        Ok(self.push(datagram))
    }
}
