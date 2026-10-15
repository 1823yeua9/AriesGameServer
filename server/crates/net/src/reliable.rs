//! reliable delivery tracking (new; the python never retransmitted).
//!
//! every datagram we send gets the 14-bit packet sequence; the client acks the
//! newest sequence it received plus a 32-bit history. ue 5.3 FNetPacketNotify:
//! the receiver shifts each status in at bit 0 (AddDeliveryStatus), so bit k is
//! AckedSeq - k (bit 0 = the acked packet itself) and the sender reads
//! IsDelivered(k). verified in the 13.06 client: sub_7FF7393D9360
//! (ProcessReceivedAcks) walks OutAckSeq+1..AckedSeq reading history bit
//! AckCount-1 down to 0. (the python's own server-side mask is shifted by
//! one: bit 0 = current-1; kept for parity there, but we read the client the
//! ue way.) the client supports up to 8 history words (header low 4 bits);
//! like the python we only read the first, which is all it sends in practice.
//! a reliable bunch in a packet the client reports missing is sent again in a
//! new packet with its ORIGINAL channel sequence and body, exactly like ue's
//! UChannel resend; the client drops any copy it already has by sequence.

use std::collections::BTreeMap;

use aries_bits::Bits;

use crate::frame::BunchHeader;

/// sequences are 14-bit on the wire.
const SEQUENCE_MASK: u32 = 0x3FFF;
const HALF: u32 = 0x2000;
/// give up on a packet the client never mentioned after this long (seconds).
pub const ACK_TIMEOUT: f64 = 1.0;

#[derive(Debug, Clone, PartialEq)]
pub struct ReliableBunch {
    pub header: BunchHeader,
    pub body: Bits,
}

#[derive(Debug, Clone)]
struct InFlight {
    sent_at: f64,
    bunches: Vec<ReliableBunch>,
}

/// a - b in sequence space, signed (positive when a is newer).
pub fn sequence_delta(a: u32, b: u32) -> i32 {
    let difference = a.wrapping_sub(b) & SEQUENCE_MASK;
    if difference >= HALF { difference as i32 - (SEQUENCE_MASK as i32 + 1) } else { difference as i32 }
}

#[derive(Debug, Default)]
pub struct ReliableTracker {
    /// keyed by an unwrapped sequence so ordering survives the 14-bit wrap
    in_flight: BTreeMap<u64, InFlight>,
    /// last 14-bit sequence seen and its unwrapped value
    last_sequence: Option<(u32, u64)>,
    newest_acked: Option<u64>,
    pub resent: u64,
    pub acked: u64,
}

impl ReliableTracker {
    fn unwrap(&mut self, sequence: u32) -> u64 {
        match self.last_sequence {
            None => {
                self.last_sequence = Some((sequence, sequence as u64 + (1 << 20)));
                sequence as u64 + (1 << 20)
            }
            Some((last, last_unwrapped)) => {
                let unwrapped = (last_unwrapped as i64 + sequence_delta(sequence, last) as i64) as u64;
                if sequence_delta(sequence, last) > 0 {
                    self.last_sequence = Some((sequence, unwrapped));
                }
                unwrapped
            }
        }
    }

    /// remembers the reliable bunches of a packet we just sent (none for ack-only packets).
    pub fn sent(&mut self, sequence: u32, now: f64, bunches: Vec<ReliableBunch>) {
        let unwrapped = self.unwrap(sequence);
        if !bunches.is_empty() {
            self.in_flight.insert(unwrapped, InFlight { sent_at: now, bunches });
        }
    }

    /// applies a client ack (newest received + 32-bit history of the 32 before it).
    /// returns the bunches to resend, oldest first.
    pub fn on_ack(&mut self, acked: u32, history: u32, now: f64) -> Vec<ReliableBunch> {
        let Some((last, last_unwrapped)) = self.last_sequence else {
            return Vec::new();
        };
        // an ack for something we never sent (e.g. before the challenge) is ignored
        let ahead = sequence_delta(acked, last);
        if ahead > 0 {
            return Vec::new();
        }
        let acked_unwrapped = (last_unwrapped as i64 + ahead as i64) as u64;
        if self.newest_acked.is_some_and(|newest| acked_unwrapped < newest) {
            return Vec::new(); // stale, reordered ack
        }
        self.newest_acked = Some(acked_unwrapped);
        let mut resend = Vec::new();
        let keys: Vec<u64> = self.in_flight.range(..=acked_unwrapped).map(|(k, _)| *k).collect();
        for key in keys {
            let distance = acked_unwrapped - key;
            let received = distance == 0 || (distance < 32 && history & (1 << distance) != 0);
            let flight = self.in_flight.remove(&key).expect("key from range");
            if received {
                self.acked += 1;
            } else if distance < 32 || now - flight.sent_at >= ACK_TIMEOUT {
                // inside the window and not received, or fell out of the window unacked
                self.resent += flight.bunches.len() as u64;
                resend.extend(flight.bunches);
            } else {
                self.in_flight.insert(key, flight);
            }
        }
        resend
    }

    /// bunches unacked for too long with no ack covering them (client went quiet).
    pub fn expired(&mut self, now: f64) -> Vec<ReliableBunch> {
        let keys: Vec<u64> = self.in_flight.iter().filter(|(_, f)| now - f.sent_at >= ACK_TIMEOUT * 2.0).map(|(k, _)| *k).collect();
        let mut resend = Vec::new();
        for key in keys {
            let flight = self.in_flight.remove(&key).expect("key");
            self.resent += flight.bunches.len() as u64;
            resend.extend(flight.bunches);
        }
        resend
    }

    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    /// true while any reliable bunch on this channel is unacked.
    pub fn channel_pending(&self, channel: u32) -> bool {
        self.in_flight.values().flat_map(|flight| &flight.bunches).any(|bunch| bunch.header.channel_index == channel)
    }

    /// true while the client has not acked this reliable bunch. a resend keeps
    /// its channel sequence, so a lost bunch stays pending until a copy lands.
    pub fn pending(&self, channel: u32, channel_sequence: u32) -> bool {
        self.in_flight.values().flat_map(|flight| &flight.bunches).any(|bunch| bunch.header.channel_index == channel && bunch.header.channel_sequence == channel_sequence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bunch(sequence: u32) -> ReliableBunch {
        ReliableBunch { header: BunchHeader::reliable(8, sequence, false), body: Bits::empty() }
    }

    #[test]
    fn acked_packets_are_forgotten_missing_ones_resent() {
        let mut t = ReliableTracker::default();
        for s in 100..110u32 {
            t.sent(s, 0.0, vec![bunch(s)]);
        }
        // client got 109 (bit 0), 108 (bit 1), not 107 (bit 2), 106 and older
        let history = 0b11 | 0xFFFF_FFF8;
        let resend = t.on_ack(109, history, 0.1);
        assert_eq!(resend, vec![bunch(107)]);
        assert_eq!(t.in_flight(), 0);
    }

    #[test]
    fn wraps_around_the_14_bit_sequence() {
        let mut t = ReliableTracker::default();
        for s in [0x3FFE, 0x3FFF, 0, 1] {
            t.sent(s, 0.0, vec![bunch(s)]);
        }
        // got 1 (bit 0); bit k = distance k: 0 (k1) yes, 0x3fff (k2) no, 0x3ffe (k3) yes
        let resend = t.on_ack(1, 0b1011, 0.0);
        assert_eq!(resend, vec![bunch(0x3FFF)]);
        assert_eq!(sequence_delta(1, 0x3FFF), 2);
        assert_eq!(sequence_delta(0x3FFF, 1), -2);
    }

    #[test]
    fn ack_only_packets_cost_nothing_and_stale_acks_are_ignored() {
        let mut t = ReliableTracker::default();
        t.sent(10, 0.0, vec![]);
        t.sent(11, 0.0, vec![bunch(11)]);
        assert!(t.on_ack(11, 0b1, 0.0).is_empty());
        assert!(t.on_ack(10, 0, 0.0).is_empty());
        assert_eq!(t.acked, 1);
    }

    #[test]
    fn a_bunch_stays_pending_through_its_resend() {
        let mut t = ReliableTracker::default();
        t.sent(20, 0.0, vec![bunch(7)]);
        t.sent(21, 0.0, vec![bunch(8)]);
        assert!(t.pending(8, 7) && t.pending(8, 8));
        // 21 arrived, 20 lost: 7 goes out again in 22 and is still pending
        let resend = t.on_ack(21, 0b1, 0.1);
        assert_eq!(resend, vec![bunch(7)]);
        assert!(!t.pending(8, 8));
        t.sent(22, 0.1, resend);
        assert!(t.pending(8, 7));
        t.on_ack(22, 0b11, 0.2);
        assert!(!t.pending(8, 7));
    }

    #[test]
    fn quiet_client_expiry_resends() {
        let mut t = ReliableTracker::default();
        t.sent(5, 0.0, vec![bunch(5)]);
        assert!(t.expired(1.0).is_empty());
        assert_eq!(t.expired(2.5), vec![bunch(5)]);
    }
}
