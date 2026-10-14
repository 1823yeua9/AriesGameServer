//! misc production reads: hud refresh (R13), client projectile snapshot (R14),
//! ultimate points (R12). the responder's inline blocks (H:1824-1845, 1991-2007)
//! and range_projectiles.read_projectile (RP:308-343).

use crate::client::LiveClient;
use crate::world::{at, find_controller, find_pawn, find_world};
use crate::{Memory, OracleError};

/// pawn +0xB80 Widget_BottomHUD, widget +0x11C visibility (1 = Collapsed) (H:1985, 2002-2003)
const PAWN_BOTTOM_HUD: u64 = 0xB80;
const WIDGET_VISIBILITY: u64 = 0x11C;
const COLLAPSED: u8 = 1;
/// controller +0x690 flags, bit 1 = cinematic mode (H:1989, 2004)
const CONTROLLER_FLAGS: u64 = 0x690;
const CINEMATIC_BIT: u32 = 2;
/// pawn +0x14F4 ultimate points required (H:1826, 1837)
const PAWN_ULT_REQUIRED: u64 = 0x14F4;
/// netdriver +0xC0 ServerConnection, connection +0x13F8/+0x1400 channels (RP:320, 323)
const DRIVER_CONNECTION: u64 = 0xC0;
const CONNECTION_CHANNELS: u64 = 0x13F8;
const CONNECTION_CHANNEL_COUNT: u64 = 0x1400;
/// actor channel +0x78 actor; actor +0x290 root, +0x610 FiniteSpeedMovement (RP:310, 327, 335, 337)
const CHANNEL_ACTOR: u64 = 0x78;
const ACTOR_ROOT: u64 = 0x290;
const ACTOR_MOVEMENT: u64 = 0x610;
/// root +0x320 projectile location (not the pawn's +0x170, spec 09 §8 note)
const ROOT_LOCATION: u64 = 0x320;
/// movement +0x198 NumCollisions, +0x228 last hit (location +0x00, normal +0x30) (RP:340-343)
const MOVEMENT_COLLISIONS: u64 = 0x198;
const MOVEMENT_HIT: u64 = 0x228;

/// the projectile class a Cloudburst flight waits for (RP:348).
pub const SMOKE_PROJECTILE_CLASS: &str = "Projectile_Wushu_4_Smoke_C";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HudState {
    pub collapsed: bool,
    pub cinematic: bool,
}

impl HudState {
    /// the responder's failure value: never refresh on a failed read (H:2005-2007).
    pub const ON_ERROR: Self = Self { collapsed: false, cinematic: true };

    /// a SpawnedCharacter refresh is sent iff collapsed and not cinematic (H:2008).
    pub fn wants_refresh(self) -> bool {
        self.collapsed && !self.cinematic
    }
}

/// H:1999-2004. the flags are read even with a null controller, which fails.
pub fn read_hud_state(memory: &impl Memory) -> Result<HudState, OracleError> {
    let controller = find_controller(memory)?;
    let pawn = if controller == 0 { 0 } else { memory.u64(at(controller, 0x4B0))? };
    let widget = if pawn == 0 { 0 } else { memory.u64(at(pawn, PAWN_BOTTOM_HUD))? };
    let collapsed = widget != 0 && memory.u8(at(widget, WIDGET_VISIBILITY))? == COLLAPSED;
    let cinematic = memory.u32(at(controller, CONTROLLER_FLAGS))? & CINEMATIC_BIT != 0;
    Ok(HudState { collapsed, cinematic })
}

/// H:1835-1837: i32 at pawn +0x14F4. a null pawn fails the read. the responder
/// maps a failure to 0 and sends only 0 < ult < 50.
pub fn read_ult_required(memory: &impl Memory) -> Result<i32, OracleError> {
    let pawn = find_pawn(memory)?;
    memory.i32(at(pawn, PAWN_ULT_REQUIRED))
}

/// RP:298-305
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectileSnapshot {
    pub class_name: String,
    pub location: Option<[f64; 3]>,
    pub collisions: u32,
    pub hit_location: [f64; 3],
    pub hit_normal: [f64; 3],
}

fn vec3(raw: &[u8]) -> [f64; 3] {
    std::array::from_fn(|i| f64::from_le_bytes(raw[8 * i..8 * i + 8].try_into().unwrap()))
}

impl<M: Memory> LiveClient<M> {
    /// R14 read_projectile (RP:308-343): the actor on our channel `channel` of the
    /// client's ServerConnection. the class name goes through the shared class cache.
    pub fn projectile(&mut self, channel: u32) -> Result<Option<ProjectileSnapshot>, OracleError> {
        let world = find_world(&self.memory)?;
        let driver = if world == 0 { 0 } else { self.memory.u64(at(world, 0x40))? };
        let connection = if driver == 0 { 0 } else { self.memory.u64(at(driver, DRIVER_CONNECTION))? };
        if connection == 0 {
            return Ok(None);
        }
        let channels = self.memory.u64(at(connection, CONNECTION_CHANNELS))?;
        let count = self.memory.u32(at(connection, CONNECTION_CHANNEL_COUNT))?;
        if channels == 0 || channel >= count {
            return Ok(None);
        }
        let actor_channel = self.memory.u64(at(channels, 8 * channel as u64))?;
        let actor = if actor_channel == 0 { 0 } else { self.memory.u64(at(actor_channel, CHANNEL_ACTOR))? };
        if actor == 0 {
            return Ok(None);
        }
        let class_name = self.cached_name(actor, false)?;
        let root = self.memory.u64(at(actor, ACTOR_ROOT))?;
        let location = if root == 0 { None } else { Some(self.memory.vec3(at(root, ROOT_LOCATION))?) };
        let movement = self.memory.u64(at(actor, ACTOR_MOVEMENT))?;
        let (Some(location), true) = (location, movement != 0) else {
            return Ok(Some(ProjectileSnapshot { class_name, location, collisions: 0, hit_location: [0.0; 3], hit_normal: [0.0; 3] }));
        };
        let collisions = self.memory.u32(at(movement, MOVEMENT_COLLISIONS))?;
        let hit: [u8; 0x48] = self.memory.read(at(movement, MOVEMENT_HIT))?;
        Ok(Some(ProjectileSnapshot { class_name, location: Some(location), collisions, hit_location: vec3(&hit[0..24]), hit_normal: vec3(&hit[0x30..0x48]) }))
    }
}
