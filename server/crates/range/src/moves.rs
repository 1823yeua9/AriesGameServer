//! the move-ack block (python 4412-4535): placement correction once, then
//! ClientAckInputAndMovement every tick. docs/port/04_bringup_b.md section 9.

use crate::config::RangeConfig;
use crate::send::Reply;

pub fn run_move_ack(_reply: &mut Reply, _config: &RangeConfig) {}
