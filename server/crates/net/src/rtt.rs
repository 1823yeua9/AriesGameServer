//! round-trip time from the client's packet acks (new; for lag compensation,
//! docs/port/10 section 4.7). a sample is the time from sending server packet
//! S to receiving the first client packet that acks S; smoothed tcp style,
//! srtt += (sample - srtt) / 8.

use std::collections::VecDeque;

use crate::reliable::sequence_delta;

/// sends remembered for matching acks (the client acks within a few packets).
const REMEMBERED: usize = 128;
pub const RTT_GAIN: f64 = 1.0 / 8.0;

#[derive(Debug, Default, Clone)]
pub struct RttEstimator {
    sends: VecDeque<(u32, f64)>,
    last_acked: Option<u32>,
    /// smoothed rtt in seconds; none until the first sample
    pub srtt: Option<f64>,
    pub last_sample: Option<f64>,
}

impl RttEstimator {
    pub fn sent(&mut self, sequence: u32, now: f64) {
        if self.sends.len() == REMEMBERED {
            self.sends.pop_front();
        }
        self.sends.push_back((sequence & 0x3FFF, now));
    }

    /// a client packet acked `acked` (its newest received server sequence).
    pub fn acked(&mut self, acked: u32, now: f64) {
        let acked = acked & 0x3FFF;
        if self.last_acked.is_some_and(|last| sequence_delta(acked, last) <= 0) {
            return;
        }
        let Some(index) = self.sends.iter().rposition(|(sequence, _)| *sequence == acked) else {
            return;
        };
        self.last_acked = Some(acked);
        let sample = (now - self.sends[index].1).max(0.0);
        self.sends.drain(..=index);
        self.last_sample = Some(sample);
        self.srtt = Some(match self.srtt {
            Some(srtt) => srtt + (sample - srtt) * RTT_GAIN,
            None => sample,
        });
    }

    pub fn srtt_or_zero(&self) -> f64 {
        self.srtt.unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smooths_samples_and_ignores_repeats() {
        let mut rtt = RttEstimator::default();
        rtt.sent(10, 1.0);
        rtt.sent(11, 1.1);
        rtt.acked(10, 1.02);
        assert_eq!(rtt.srtt, Some(0.020000000000000018));
        rtt.acked(10, 1.5); // the same ack again: no sample
        rtt.acked(11, 1.14);
        let expected = 0.020000000000000018 + (0.039999999999999813 - 0.020000000000000018) / 8.0;
        assert_eq!(rtt.srtt, Some(expected));
    }

    #[test]
    fn wraps_and_skips_unknown_acks() {
        let mut rtt = RttEstimator::default();
        rtt.sent(0x3FFF, 0.0);
        rtt.sent(0, 0.01);
        rtt.acked(5, 0.02); // never sent
        assert_eq!(rtt.srtt, None);
        rtt.acked(0, 0.03);
        assert_eq!(rtt.last_sample, Some(0.019999999999999997));
        rtt.acked(0x3FFF, 0.05); // older than the last ack
        assert_eq!(rtt.last_sample, Some(0.019999999999999997));
    }
}
