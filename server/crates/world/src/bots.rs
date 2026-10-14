//! range target bots and the spray-wall dummy (range_bots.py, spec 08 section 4):
//! per-connection allocation and seeded placement, the hit model, the
//! open/health/alive/damage-text bodies, and the bot tick and shot systems
//! (range_udp_handshake.py part C) as plain state changes plus the bunches to send.
//!
//! rb = range_bots.py, re = range_equippables.py, ruh = range_udp_handshake.py.

use aries_bits::{BitWriter, Bits};
use aries_rep::block::{
    Spawn, Target, append_dynamic_actor_header, append_export_header, append_rep_block, append_rpc_block, append_stable_child_entry,
    append_stable_child_export, append_static_export_entry, dynamic_actor_reference,
};
use aries_rep::{RepError, RepResult};

use crate::PyRandom;

/// rb:41-43
pub const BOT_ARCHETYPE: (&str, &str) = ("Default__TrainingBot_PC_C", "/Game/Characters/TrainingBot/TrainingBot_PC");
pub const BOT_ARCHETYPE_GUID: u32 = 300001;
pub const BOT_PACKAGE_GUID: u32 = 300003;
/// rb:44-47: one channel per entity, never reused; actor, health, handler, BombTeam per bot.
pub const FIRST_BOT_CHANNEL: u32 = 400;
pub const FIRST_BOT_GUID: u32 = 300100;
pub const BOT_GUID_STRIDE: u32 = 8;
/// rb:50-52: BombTeamComponent Team (FEnumProperty EAresTeam, 8 bits, IDA sub_7FF7368FA3B0).
pub const BOMB_TEAM_TEAM_HANDLE: u32 = 3;
pub const BOMB_TEAM_TEAM_BITS: u32 = 8;
pub const BOT_TEAM_COMPONENT_NAME: &str = "BombTeam";
/// rb:53-54: the player's PlayerState BombTeam and the enemy (defender) team.
pub const PLAYER_TEAM: u32 = 1;
pub const RED_TEAM: u32 = 0;
/// rb:146: red_team if player_team != red_team else 1.
pub const BOT_TEAM: u32 = if PLAYER_TEAM != RED_TEAM { RED_TEAM } else { 1 };
/// rb:55
pub const BOT_MAX_LIFE: f64 = 100.0;
/// rb:59-60: AimBotsManager origin; capsule centre = firing-line floor + half height.
pub const MANAGER_ORIGIN: (f64, f64) = (4825.0, -13600.0);
pub const BOT_CENTER_Z: f64 = 1253.8 + 98.0;
/// rb:150-157: placement cone, ring and spacing, and the spot used when every try fails.
pub const PLACE_ATTEMPTS: usize = 64;
pub const PLACE_HALF_ANGLE: f64 = 30.0;
pub const PLACE_MIN_DISTANCE: f64 = 700.0;
pub const PLACE_MAX_DISTANCE: f64 = 1800.0;
pub const PLACE_MIN_SPACING: f64 = 200.0;
pub const PLACE_FALLBACK_BACK: f64 = 1000.0;
/// rb:66-68: hit capsule regions relative to the bot centre.
pub const HEAD_CENTER_Z: f64 = 75.0;
pub const HEAD_RADIUS: f64 = 22.0;
pub const BODY_RADIUS: f64 = 28.0;
pub const BODY_LOW: f64 = -22.0;
pub const BODY_HIGH: f64 = 53.0;
pub const LEG_RADIUS: f64 = 22.0;
pub const LEG_LOW: f64 = -98.0;
/// rb:310: rays this close to vertical never hit a cylinder.
const VERTICAL_RAY_EPSILON: f64 = 1e-9;

/// rb:71-82: (head, body, legs) damage by weapon class.
pub const WEAPON_DAMAGE: [(&str, [u32; 3]); 20] = [
    ("BasePistol_C", [78, 26, 22]),
    ("SawedOffShotgun_C", [36, 12, 10]),
    ("AutomaticPistol_C", [78, 26, 22]),
    ("LugerPistol_C", [105, 30, 25]),
    ("CompactPistol_C", [160, 40, 34]),
    ("RevolverPistol_C", [159, 55, 46]),
    ("Vector_C", [67, 27, 22]),
    ("SubMachineGun_MP5_C", [78, 26, 22]),
    ("PumpShotgun_C", [40, 20, 17]),
    ("AutomaticShotgun_C", [34, 17, 14]),
    ("AssaultRifle_Burst_C", [116, 35, 29]),
    ("DMR_C", [195, 65, 48]),
    ("AssaultRifle_ACR_C", [156, 39, 33]),
    ("AssaultRifle_AK_C", [160, 40, 34]),
    ("BattleRifle_C", [176, 44, 37]),
    ("LeverSniperRifle_C", [202, 101, 85]),
    ("DS_Gun_C", [238, 140, 119]),
    ("BoltSniper_C", [255, 150, 127]),
    ("LightMachineGun_C", [72, 30, 25]),
    ("HeavyMachineGun_C", [95, 38, 32]),
];
/// rb:83
pub const DEFAULT_DAMAGE: [u32; 3] = [100, 35, 30];

/// rb:87-93: the spray-wall TargetPracticeTrainingBot.
pub const DUMMY_ARCHETYPE: (&str, &str) = ("Default__TargetPracticeTrainingBot_C", "/Game/Heroes/Decoy/TargetPracticeTrainingBot");
pub const DUMMY_ARCHETYPE_GUID: u32 = 300005;
pub const DUMMY_PACKAGE_GUID: u32 = 300007;
pub const DUMMY_LOCATION: [f64; 3] = [6190.0, -11640.0, 1270.0];
pub const DUMMY_YAW: f64 = 180.0;
pub const DUMMY_MAX_LIFE: f64 = 1000.0;
pub const DUMMY_MESH_LIFT: f64 = 4.0;
/// rb:96-97: UpdateDamageText field 57, class max 58 -> SerializeInt(57, 59).
pub const DUMMY_UPDATE_DAMAGE_TEXT_FIELD: u32 = 57;
pub const DUMMY_MAX_FIELD_INDEX: u32 = 58;

/// re:156: UChildDamageSectionComponent handles.
pub const DAMAGE_SECTION_ALIVE_HANDLE: u32 = 3;
pub const DAMAGE_SECTION_LIFE_HANDLE: u32 = 4;
pub const DAMAGE_SECTION_MAXIMUM_LIFE_HANDLE: u32 = 5;
/// rb:280: DamageHandlerComponent (UDamageableComponent) bAlive.
pub const DAMAGE_HANDLER_ALIVE_HANDLE: u32 = 3;
pub const HEALTH_SECTION_NAME: &str = "HealthDamageSection";
pub const DAMAGE_HANDLER_NAME: &str = "DamageHandlerComponent";
/// rb:180, 201: the Range level.
pub const LEVEL_GUID: u32 = 5;

/// ruh:2039 BotField(count=range_bot_count): the --range-bot-count default
/// (ruh:4684), never passed by the launchers.
pub const DEFAULT_BOT_COUNT: usize = 5;
/// ruh:2048: a dead bot's channel closes this long after it died.
pub const RETIRE_AFTER: f64 = 1.5;
/// ruh:2053: at most one spawn per this spacing.
pub const SPAWN_SPACING: f64 = 0.3;
/// ruh:2035: the bot tick runs at most this often.
pub const BOT_TICK_INTERVAL: f64 = 0.1;
/// spec 08 open question 4: python never guards bot channels; stop below the
/// projectile range (range_projectiles first_projectile_channel, rp:60).
pub const BOT_CHANNEL_LIMIT: u32 = 10000;

/// one bunch for the range crate to send reliably on `channel`
/// (python send_reliable(channel, body, bits, has_exports, is_open, is_close)).
/// an `is_open` send on a dynamic channel starts its sequence counter at +0
/// (python channel_first_dynamic_offset[ch] = 0 right before the open).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSend {
    pub channel: u32,
    pub body: Bits,
    pub has_exports: bool,
    pub is_open: bool,
    pub is_close: bool,
}

impl ChannelSend {
    pub fn open(channel: u32, body: Bits) -> Self {
        Self { channel, body, has_exports: true, is_open: true, is_close: false }
    }

    pub fn update(channel: u32, body: Bits, has_exports: bool) -> Self {
        Self { channel, body, has_exports, is_open: false, is_close: false }
    }

    /// empty reliable bunch with bClose (reason 0, destroyed).
    pub fn close(channel: u32) -> Self {
        Self { channel, body: Bits::empty(), has_exports: false, is_open: false, is_close: true }
    }
}

// ---------------------------------------------------------------- python float helpers

/// math.radians: x * (pi / 180) as one multiply.
pub fn py_radians(degrees: f64) -> f64 {
    degrees * (std::f64::consts::PI / 180.0)
}

/// math.degrees: x * (180 / pi) as one multiply.
pub fn py_degrees(radians: f64) -> f64 {
    radians * (180.0 / std::f64::consts::PI)
}

/// math.atan2 (cpython m_atan2): its special cases, then the crt atan2.
pub fn py_atan2(y: f64, x: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    if y.is_infinite() {
        let pi = std::f64::consts::PI;
        return if x.is_infinite() {
            if x.is_sign_positive() { (pi / 4.0).copysign(y) } else { (0.75 * pi).copysign(y) }
        } else {
            (pi / 2.0).copysign(y)
        };
    }
    if x.is_infinite() || y == 0.0 {
        return if x.is_sign_positive() { 0.0f64.copysign(y) } else { std::f64::consts::PI.copysign(y) };
    }
    y.atan2(x)
}

/// python round() on a float to an int: half to even.
pub fn py_round(value: f64) -> i64 {
    value.round_ties_even() as i64
}

/// struct.pack("<f", value) as u32 bits: round to nearest, overflow refused
/// (cpython PyFloat_Pack4 raises when a finite double rounds to infinity).
pub fn py_pack_f32(value: f64) -> RepResult<u32> {
    let single = value as f32;
    if single.is_infinite() && !value.is_infinite() {
        return Err(RepError::Invalid("float too large to pack with f format".into()));
    }
    Ok(single.to_bits())
}

/// builtin sum() over floats on cpython 3.12: an int 0 start, then neumaier
/// compensated addition (rb:296-297 depend on it; plain + differs).
pub fn py_sum(values: &[f64]) -> f64 {
    let Some((&first, rest)) = values.split_first() else {
        return 0.0;
    };
    let mut total = 0.0 + first;
    let mut compensation = 0.0;
    for &value in rest {
        let next = total + value;
        if total.abs() >= value.abs() {
            compensation += (total - next) + value;
        } else {
            compensation += (value - next) + total;
        }
        total = next;
    }
    if compensation != 0.0 && compensation.is_finite() {
        total += compensation;
    }
    total
}

/// error-free product (cpython dl_mul).
fn dl_mul(x: f64, y: f64) -> (f64, f64) {
    let z = x * y;
    (z, x.mul_add(y, -z))
}

/// error-free sum for |a| >= |b| (cpython dl_fast_sum).
fn dl_fast_sum(a: f64, b: f64) -> (f64, f64) {
    let x = a + b;
    (x, (a - x) + b)
}

/// frexp's exponent: value = m * 2^e with m in [0.5, 1). value finite and > 0.
fn frexp_exponent(value: f64) -> i32 {
    let biased = ((value.to_bits() >> 52) & 0x7FF) as i32;
    if biased == 0 {
        // subnormal: normalise first
        return frexp_exponent(value * 18014398509481984.0) - 54;
    }
    biased - 1022
}

/// ldexp(1.0, exponent) for -1074 <= exponent <= 1023.
fn ldexp_one(exponent: i32) -> f64 {
    if exponent >= -1022 {
        f64::from_bits(((exponent + 1023) as u64) << 52)
    } else {
        f64::from_bits(1u64 << (exponent + 1074))
    }
}

/// cpython 3.12 vector_norm (Modules/mathmodule.c), behind math.hypot and math.dist.
fn vector_norm(values: &mut [f64], max: f64, found_nan: bool) -> f64 {
    if max.is_infinite() {
        return max;
    }
    if found_nan {
        return f64::NAN;
    }
    if max == 0.0 || values.len() <= 1 {
        return max;
    }
    let max_e = frexp_exponent(max);
    if max_e < -1023 {
        // ldexp(1.0, -max_e) would overflow: rescale subnormals first
        for value in values.iter_mut() {
            *value /= f64::MIN_POSITIVE;
        }
        return f64::MIN_POSITIVE * vector_norm(values, max / f64::MIN_POSITIVE, found_nan);
    }
    let scale = ldexp_one(-max_e);
    let (mut csum, mut frac1, mut frac2) = (1.0f64, 0.0f64, 0.0f64);
    for &value in values.iter() {
        let x = value * scale;
        let (high, low) = dl_mul(x, x);
        let (sum_high, sum_low) = dl_fast_sum(csum, high);
        csum = sum_high;
        frac1 += low;
        frac2 += sum_low;
    }
    let mut h = (csum - 1.0 + (frac1 + frac2)).sqrt();
    let (high, low) = dl_mul(-h, h);
    let (sum_high, sum_low) = dl_fast_sum(csum, high);
    csum = sum_high;
    frac1 += low;
    frac2 += sum_low;
    let x = csum - 1.0 + (frac1 + frac2);
    h += x / (2.0 * h);
    h / scale
}

/// math.hypot(x, y) exactly as cpython 3.12 computes it (rb:155 compares it to 200.0).
pub fn py_hypot(x: f64, y: f64) -> f64 {
    let mut values = [x.abs(), y.abs()];
    let mut max = 0.0f64;
    let mut found_nan = false;
    for &value in &values {
        found_nan |= value.is_nan();
        if value > max {
            max = value;
        }
    }
    vector_norm(&mut values, max, found_nan)
}

// ---------------------------------------------------------------- bots and the field

/// rb:100-114
#[derive(Debug, Clone, PartialEq)]
pub struct Bot {
    /// position in BotField.bots when allocated; -1 for the dummy
    pub index: i64,
    pub channel: u32,
    pub actor_guid: u32,
    pub health_guid: u32,
    pub location: [f64; 3],
    pub handler_guid: u32,
    pub yaw: f64,
    pub team_guid: u32,
    pub team: u32,
    pub life: f64,
    pub alive: bool,
    pub opened: bool,
    pub died_at: Option<f64>,
}

/// per-connection bots (rb:117-157) plus the connection's last tick and spawn
/// times (ruh bot_ticks, bot_spawned_at; kept here because they live as long).
#[derive(Clone)]
pub struct BotField {
    pub count: usize,
    pub next_channel: u32,
    pub next_guid: u32,
    pub bots: Vec<Bot>,
    pub rng: PyRandom,
    pub dummy: Option<Bot>,
    /// ruh bot_ticks.get(source, 0)
    pub ticked_at: f64,
    /// ruh bot_spawned_at.get(source, 0)
    pub spawned_at: f64,
}

impl Default for BotField {
    fn default() -> Self {
        Self::new(DEFAULT_BOT_COUNT)
    }
}

impl BotField {
    /// rb:120-125: Random(1306) per field.
    pub fn new(count: usize) -> Self {
        Self {
            count,
            next_channel: FIRST_BOT_CHANNEL,
            next_guid: FIRST_BOT_GUID,
            bots: Vec::new(),
            rng: PyRandom::new(1306),
            dummy: None,
            ticked_at: 0.0,
            spawned_at: 0.0,
        }
    }

    /// rb:136-146: next channel, 8 guids, a placement (2+ rng draws), facing the manager.
    pub fn allocate(&mut self, index: i64) -> RepResult<Bot> {
        if self.next_channel >= BOT_CHANNEL_LIMIT {
            return Err(RepError::Invalid("out of bot channels".into()));
        }
        let channel = self.next_channel;
        self.next_channel += 1;
        let guid = self.next_guid;
        self.next_guid += BOT_GUID_STRIDE;
        let location = self.place();
        let yaw = py_degrees(py_atan2(MANAGER_ORIGIN.1 - location[1], MANAGER_ORIGIN.0 - location[0]));
        Ok(Bot {
            index,
            channel,
            actor_guid: guid,
            health_guid: guid + 2,
            location,
            handler_guid: guid + 4,
            yaw,
            team_guid: guid + 6,
            team: BOT_TEAM,
            life: BOT_MAX_LIFE,
            alive: true,
            opened: false,
            died_at: None,
        })
    }

    /// rb:127-134: the dummy takes a channel, 8 guids and a placement's draws
    /// like a bot, then moves to the spray wall. never in `bots`.
    pub fn allocate_dummy(&mut self) -> RepResult<&mut Bot> {
        let mut dummy = self.allocate(-1)?;
        dummy.location = DUMMY_LOCATION;
        dummy.yaw = DUMMY_YAW;
        dummy.life = DUMMY_MAX_LIFE;
        Ok(self.dummy.insert(dummy))
    }

    /// rb:148-157: up to 64 tries in the cone, 200 apart from every live bot.
    pub fn place(&mut self) -> [f64; 3] {
        let taken: Vec<[f64; 3]> = self.bots.iter().filter(|bot| bot.alive).map(|bot| bot.location).collect();
        for _ in 0..PLACE_ATTEMPTS {
            let angle = py_radians(self.rng.uniform(-PLACE_HALF_ANGLE, PLACE_HALF_ANGLE));
            let distance = self.rng.uniform(PLACE_MIN_DISTANCE, PLACE_MAX_DISTANCE);
            let x = MANAGER_ORIGIN.0 + distance * angle.sin();
            let y = MANAGER_ORIGIN.1 - distance * angle.cos();
            if taken.iter().all(|other| py_hypot(x - other[0], y - other[1]) >= PLACE_MIN_SPACING) {
                return [x, y, BOT_CENTER_Z];
            }
        }
        [MANAGER_ORIGIN.0, MANAGER_ORIGIN.1 - PLACE_FALLBACK_BACK, BOT_CENTER_Z]
    }

    pub fn alive_count(&self) -> usize {
        self.bots.iter().filter(|bot| bot.alive).count()
    }
}

// ---------------------------------------------------------------- bodies

fn invalid(message: &str) -> RepError {
    RepError::Invalid(message.into())
}

/// bunch bodies stay under 2^13 bits (rb:193, the python guard).
fn under_bunch_limit(writer: BitWriter, what: &str) -> RepResult<Bits> {
    let bits = writer.finish()?;
    if bits.bit_count >= 1 << 13 {
        return Err(RepError::Invalid(format!("{what} exceeds the bunch size limit")));
    }
    Ok(bits)
}

/// rb:160-195: archetype + stably-named BombTeam exports, the spawn header,
/// then BombTeam.Team = bot.team so BeginPlay sees the enemy team.
pub fn build_bot_open(bot: &Bot) -> RepResult<Bits> {
    if bot.team_guid == 0 || bot.team_guid & 1 != 0 || bot.team == PLAYER_TEAM {
        return Err(invalid("bot needs an even BombTeam NetGUID and an enemy team"));
    }
    let mut w = BitWriter::new();
    append_export_header(&mut w, 2);
    append_static_export_entry(&mut w, BOT_ARCHETYPE.0, BOT_ARCHETYPE.1, BOT_ARCHETYPE_GUID, BOT_PACKAGE_GUID);
    // outer: the actor by guid only, spawned by this bunch
    append_stable_child_entry(&mut w, bot.team_guid, bot.actor_guid, BOT_TEAM_COMPONENT_NAME);
    append_dynamic_actor_header(&mut w, bot.actor_guid, BOT_ARCHETYPE_GUID, LEVEL_GUID, Some(Spawn { location: bot.location, yaw: bot.yaw }));
    let mut rep = BitWriter::new();
    rep.append_bits(0, 1);
    rep.append_packed_uint(BOMB_TEAM_TEAM_HANDLE as u64);
    rep.append_bits(bot.team as u64, BOMB_TEAM_TEAM_BITS);
    rep.append_packed_uint(0);
    append_rep_block(&mut w, Target::Subobject(bot.team_guid), &rep.finish()?, bot.actor_guid)?;
    under_bunch_limit(w, "bot open")
}

/// rb:198-202
pub fn build_dummy_open(dummy: &Bot) -> RepResult<Bits> {
    Ok(dynamic_actor_reference(
        DUMMY_ARCHETYPE,
        dummy.actor_guid,
        DUMMY_ARCHETYPE_GUID,
        DUMMY_PACKAGE_GUID,
        LEVEL_GUID,
        Some(Spawn { location: dummy.location, yaw: dummy.yaw }),
    )?)
}

/// rb:205-207: full life, alive, exported.
pub fn build_dummy_health_body(dummy: &Bot) -> RepResult<Bits> {
    build_damage_section_body(dummy.actor_guid, dummy.health_guid, HEALTH_SECTION_NAME, DUMMY_MAX_LIFE, DUMMY_MAX_LIFE, true, true)
}

/// rb:210-219: [SerializeInt(57, 59)][packed 65][presence 1][f64 damage].
pub fn build_update_damage_text_payload(damage: f64) -> RepResult<Bits> {
    let mut params = BitWriter::new();
    params.append_bits(1, 1); // presence (legacy path; not a bool parameter)
    params.append_f64(damage);
    let params = params.finish()?;
    let mut w = BitWriter::new();
    w.append_serialize_int(DUMMY_UPDATE_DAMAGE_TEXT_FIELD as u64, DUMMY_MAX_FIELD_INDEX as u64 + 1);
    w.append_packed_uint(params.bit_count as u64);
    w.append_bits_value(&params);
    Ok(w.finish()?)
}

/// rb:222-226: the multicast rpc on the dummy's own channel (no exports).
pub fn build_update_damage_text_body(dummy: &Bot, damage: f64) -> RepResult<Bits> {
    let payload = build_update_damage_text_payload(damage)?;
    let mut w = BitWriter::new();
    append_rpc_block(&mut w, Target::Actor, &payload, dummy.actor_guid)?;
    Ok(w.finish()?)
}

/// rb:237-261: bAlive/Life/MaximumLife on a stably-named damage section.
pub fn build_damage_section_body(actor_guid: u32, section_guid: u32, section_name: &str, life: f64, maximum: f64, alive: bool, export: bool) -> RepResult<Bits> {
    if !(0.0 <= life && life <= maximum && maximum <= 1000.0) {
        return Err(invalid("damage section values out of range"));
    }
    let mut w = BitWriter::new();
    if export {
        append_stable_child_export(&mut w, section_guid, actor_guid, section_name);
    }
    let mut rep = BitWriter::new();
    rep.append_bits(0, 1);
    rep.append_packed_uint(DAMAGE_SECTION_ALIVE_HANDLE as u64);
    rep.append_bool(alive);
    rep.append_packed_uint(DAMAGE_SECTION_LIFE_HANDLE as u64);
    rep.append_u32(py_pack_f32(life)?);
    rep.append_packed_uint(DAMAGE_SECTION_MAXIMUM_LIFE_HANDLE as u64);
    rep.append_u32(py_pack_f32(maximum)?);
    rep.append_packed_uint(0);
    append_rep_block(&mut w, Target::Subobject(section_guid), &rep.finish()?, actor_guid)?;
    Ok(w.finish()?)
}

/// rb:264-266: life clamped at 0 (python max(0.0, life)).
pub fn build_bot_health_body(bot: &Bot, export: bool) -> RepResult<Bits> {
    let life = if bot.life > 0.0 { bot.life } else { 0.0 };
    build_damage_section_body(bot.actor_guid, bot.health_guid, HEALTH_SECTION_NAME, life, BOT_MAX_LIFE, bot.alive, export)
}

/// rb:269-286, export form (the only one production sends, ruh:1010).
pub fn build_bot_alive_body(bot: &Bot) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    append_stable_child_export(&mut w, bot.handler_guid, bot.actor_guid, DAMAGE_HANDLER_NAME);
    let mut rep = BitWriter::new();
    rep.append_bits(0, 1);
    rep.append_packed_uint(DAMAGE_HANDLER_ALIVE_HANDLE as u64);
    rep.append_bool(bot.alive);
    rep.append_packed_uint(0);
    append_rep_block(&mut w, Target::Subobject(bot.handler_guid), &rep.finish()?, bot.actor_guid)?;
    Ok(w.finish()?)
}

// ---------------------------------------------------------------- hits

/// hit regions, in hit_region's evaluation order (rb:326-338).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Head,
    Body,
    Legs,
}

impl Region {
    pub fn name(self) -> &'static str {
        match self {
            Region::Head => "head",
            Region::Body => "body",
            Region::Legs => "legs",
        }
    }

    fn slot(self) -> usize {
        match self {
            Region::Head => 0,
            Region::Body => 1,
            Region::Legs => 2,
        }
    }
}

/// rb:289-291: unit view direction from camera pitch/yaw in degrees.
pub fn view_ray(pitch_deg: f64, yaw_deg: f64) -> [f64; 3] {
    let (pitch, yaw) = (py_radians(pitch_deg), py_radians(yaw_deg));
    [pitch.cos() * yaw.cos(), pitch.cos() * yaw.sin(), pitch.sin()]
}

/// rb:294-302: first positive hit distance on a sphere (dot products via sum()).
fn ray_sphere(origin: [f64; 3], direction: [f64; 3], center: [f64; 3], radius: f64) -> Option<f64> {
    let oc = [origin[0] - center[0], origin[1] - center[1], origin[2] - center[2]];
    let b = py_sum(&[oc[0] * direction[0], oc[1] * direction[1], oc[2] * direction[2]]);
    let c = py_sum(&[oc[0] * oc[0], oc[1] * oc[1], oc[2] * oc[2]]) - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let t = -b - disc.sqrt();
    (t > 0.0).then_some(t)
}

/// rb:305-323: vertical finite cylinder about the centre axis, no end caps.
fn ray_cylinder(origin: [f64; 3], direction: [f64; 3], center: [f64; 3], radius: f64, low: f64, high: f64) -> Option<f64> {
    let (ox, oy) = (origin[0] - center[0], origin[1] - center[1]);
    let (dx, dy) = (direction[0], direction[1]);
    let a = dx * dx + dy * dy;
    if a < VERTICAL_RAY_EPSILON {
        return None;
    }
    let b = ox * dx + oy * dy;
    let c = ox * ox + oy * oy - radius * radius;
    let disc = b * b - a * c;
    if disc < 0.0 {
        return None;
    }
    for t in [(-b - disc.sqrt()) / a, (-b + disc.sqrt()) / a] {
        if t <= 0.0 {
            continue;
        }
        let z = origin[2] + direction[2] * t;
        if center[2] + low <= z && z <= center[2] + high {
            return Some(t);
        }
    }
    None
}

/// rb:326-338: nearest of head/body/legs; the first wins a tie (python min).
pub fn hit_region(origin: [f64; 3], direction: [f64; 3], center: [f64; 3]) -> Option<(Region, f64)> {
    let head = ray_sphere(origin, direction, [center[0], center[1], center[2] + HEAD_CENTER_Z], HEAD_RADIUS);
    let body = ray_cylinder(origin, direction, center, BODY_RADIUS, BODY_LOW, BODY_HIGH);
    let legs = ray_cylinder(origin, direction, center, LEG_RADIUS, LEG_LOW, BODY_LOW);
    let mut best: Option<(Region, f64)> = None;
    for (region, distance) in [(Region::Head, head), (Region::Body, body), (Region::Legs, legs)] {
        let Some(distance) = distance else { continue };
        if best.is_none_or(|(_, current)| distance < current) {
            best = Some((region, distance));
        }
    }
    best
}

/// rb:229-234: the dummy's mesh sits 4 units higher than a training bot's.
pub fn dummy_hit(dummy: Option<&Bot>, origin: [f64; 3], direction: [f64; 3]) -> Option<(Region, f64)> {
    let dummy = dummy.filter(|dummy| dummy.opened)?;
    let [x, y, z] = dummy.location;
    hit_region(origin, direction, [x, y, z + DUMMY_MESH_LIFT])
}

/// rb:341-343: unknown (or missing) classes take the default row.
pub fn shot_damage(class_name: Option<&str>, region: Region) -> u32 {
    let class_name = class_name.unwrap_or("");
    let row = WEAPON_DAMAGE.iter().find(|(name, _)| *name == class_name).map_or(DEFAULT_DAMAGE, |(_, row)| *row);
    row[region.slot()]
}

/// rb:346-355: (index into bots, region, distance) of the nearest live, opened bot.
pub fn first_hit(bots: &[Bot], origin: [f64; 3], direction: [f64; 3]) -> Option<(usize, Region, f64)> {
    let mut best: Option<(usize, Region, f64)> = None;
    for (index, bot) in bots.iter().enumerate() {
        if !bot.alive || !bot.opened {
            continue;
        }
        if let Some((region, distance)) = hit_region(origin, direction, bot.location)
            && best.is_none_or(|(_, _, current)| distance < current)
        {
            best = Some((index, region, distance));
        }
    }
    best
}

// ---------------------------------------------------------------- the per-packet systems

/// what one bot tick did (ruh:2034-2062): bunches in send order and the event values.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BotTick {
    pub sends: Vec<ChannelSend>,
    /// event dummy_spawned: [channel, actor]
    pub dummy_spawned: Option<(u32, u32)>,
    /// event bot_retired: actor guids
    pub retired: Vec<u32>,
    /// event bot_spawned: [channel, actor, location rounded half-even]
    pub spawned: Vec<(u32, u32, [i64; 3])>,
}

/// one shot's outcome (ruh:987-1012).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShotOutcome {
    /// event dummy_hits: [region, damage, round(distance)]
    Dummy { region: Region, damage: u32, distance: f64 },
    /// event bot_shot_missed += 1
    Missed,
    /// event bot_hits: [actor, region, damage, round(life), round(distance)]
    Bot { actor_guid: u32, region: Region, damage: u32, life: f64, distance: f64, killed: bool },
}

/// what bot_shots did for one camera read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShotsResult {
    pub sends: Vec<ChannelSend>,
    pub outcomes: Vec<ShotOutcome>,
}

impl BotField {
    /// ruh:2035: the 0.1 s tick gate; the caller adds range_bots and the
    /// range-streaming-sent gate.
    pub fn tick_due(&self, now: f64) -> bool {
        now - self.ticked_at >= BOT_TICK_INTERVAL
    }

    /// ruh:2036-2062, one tick: the dummy once (open +0, health +1), retire bots
    /// dead >= 1.5 s (close), then at most one spawn (open +0, health +1) while
    /// fewer than `count` are alive and >= 0.3 s since the last spawn.
    pub fn tick(&mut self, now: f64) -> RepResult<BotTick> {
        self.ticked_at = now;
        let mut tick = BotTick::default();
        if self.dummy.is_none() {
            let dummy = self.allocate_dummy()?.clone();
            tick.sends.push(ChannelSend::open(dummy.channel, build_dummy_open(&dummy)?));
            tick.sends.push(ChannelSend::update(dummy.channel, build_dummy_health_body(&dummy)?, true));
            if let Some(dummy) = self.dummy.as_mut() {
                dummy.opened = true;
            }
            tick.dummy_spawned = Some((dummy.channel, dummy.actor_guid));
        }
        let mut index = 0;
        while index < self.bots.len() {
            let bot = &self.bots[index];
            if !bot.alive && bot.died_at.is_some_and(|died_at| now - died_at >= RETIRE_AFTER) {
                tick.sends.push(ChannelSend::close(bot.channel));
                tick.retired.push(bot.actor_guid);
                self.bots.remove(index);
            } else {
                index += 1;
            }
        }
        if self.alive_count() < self.count && now - self.spawned_at >= SPAWN_SPACING {
            let mut bot = self.allocate(self.bots.len() as i64)?;
            tick.sends.push(ChannelSend::open(bot.channel, build_bot_open(&bot)?));
            tick.sends.push(ChannelSend::update(bot.channel, build_bot_health_body(&bot, true)?, true));
            bot.opened = true;
            self.spawned_at = now;
            tick.spawned.push((bot.channel, bot.actor_guid, bot.location.map(py_round)));
            self.bots.push(bot);
        }
        Ok(tick)
    }

    /// ruh:985-1012 after a camera read: `shots` shots along the view. the dummy
    /// takes the shot when it is strictly nearer than the first live bot; a bot
    /// hit sends its health, and on death its DamageHandler bAlive (exported).
    pub fn shots(&mut self, class_name: Option<&str>, shots: u32, camera_location: [f64; 3], pitch: f64, yaw: f64, now: f64) -> RepResult<ShotsResult> {
        let direction = view_ray(pitch, yaw);
        let origin = camera_location;
        let mut result = ShotsResult::default();
        for _ in 0..shots {
            let hit = first_hit(&self.bots, origin, direction);
            let on_dummy = dummy_hit(self.dummy.as_ref(), origin, direction);
            if let (Some((region, distance)), Some(dummy)) = (on_dummy, self.dummy.as_ref())
                && hit.is_none_or(|(_, _, bot_distance)| distance < bot_distance)
            {
                let damage = shot_damage(class_name, region);
                result.sends.push(ChannelSend::update(dummy.channel, build_update_damage_text_body(dummy, damage as f64)?, false));
                result.outcomes.push(ShotOutcome::Dummy { region, damage, distance });
                continue;
            }
            let Some((index, region, distance)) = hit else {
                result.outcomes.push(ShotOutcome::Missed);
                continue;
            };
            let damage = shot_damage(class_name, region);
            let bot = &mut self.bots[index];
            bot.life -= damage as f64;
            if bot.life <= 0.0 {
                bot.life = 0.0;
                bot.alive = false;
                bot.died_at = Some(now);
            }
            result.sends.push(ChannelSend::update(bot.channel, build_bot_health_body(bot, false)?, false));
            if !bot.alive {
                result.sends.push(ChannelSend::update(bot.channel, build_bot_alive_body(bot)?, true));
            }
            result.outcomes.push(ShotOutcome::Bot { actor_guid: bot.actor_guid, region, damage, life: bot.life, distance, killed: !bot.alive });
        }
        Ok(result)
    }
}

/// ruh:1622-1632: per connection and equippable key, the last predicted
/// networked random seed of the gun's firing state (+0x4A0 -> +0xEC). a change
/// between two reads is one shot, however many rounds were fired in between.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiringSeeds {
    seeds: std::collections::HashMap<String, u32>,
}

impl FiringSeeds {
    /// store the seed; true when an earlier seed exists and differs.
    pub fn observe(&mut self, key: &str, seed: u32) -> bool {
        match self.seeds.insert(key.to_string(), seed) {
            Some(previous) => previous != seed,
            None => false,
        }
    }
}
