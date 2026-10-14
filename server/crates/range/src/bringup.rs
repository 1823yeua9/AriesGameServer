//! the bring-up stage chain (python elif chain 2185-4407): per session-ack tick
//! the first ready stage sends, in chain order. docs/port/03_bringup_a.md and 04_bringup_b.md.

use crate::config::RangeConfig;
use crate::send::Reply;

/// times (seconds) each stage was sent, plus multi-step counters.
#[derive(Debug, Default, Clone)]
pub struct BringupState {}

/// runs at most one stage. returns true when a stage sent.
pub fn run_stage_chain(_reply: &mut Reply, _config: &RangeConfig) -> bool {
    false
}
