//! interpolation, extrapolation and lag compensation for the 128 hz server.
//! the design follows valve's source sdk 2013 (CInterpolatedVar and
//! CLagCompensationManager); this is our own implementation of those
//! algorithms (the sdk license covers source-engine mods only).
//! docs/port/10_interp_source_sdk.md.

pub mod lagcomp;
pub mod stream;
pub mod time;
pub mod value;
pub mod var;

pub use lagcomp::{EntityKey, HitShape, LagCompConfig, LagCompensation, LagRecord, Lost, Pose, Rewind, Target, TargetSource};
pub use stream::{OracleStream, StreamValue};
pub use time::{TICK_DT, TICK_HZ, entity_interp, ticks_to_time, time_to_ticks};
pub use value::{Interp, Rot, Step, Vec3, Velocity, wrap180};
pub use var::{InterpolatedVar, Kind, Locate, MAX_EXTRAPOLATE, Sample, Value};
