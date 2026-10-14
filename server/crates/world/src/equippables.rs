//! replicated range equippables and their authority mirror (range_equippables.py,
//! spec 07 sections 4-5).
//!
//! every weapon actor the range owns (the starting classic and knife, the wushu
//! charges, bought guns) is one EquippableActor: channel, netguids, slot, its
//! EquippableStateMachine subobject and the guids its named states export under.
//! the client predicts state transitions locally; a real server confirms each by
//! replicating AuthCurrentState.CurrentState (handle 3), or the client's
//! disagreement timer rolls the prediction back (IDA sub_7FF73B65C9E0 /
//! sub_7FF73C3AB9B0). ammo and charges work the same way through
//! AuthResourceAmount: it starts at 0, so an unconfirmed gun never fires.
//!
//! times are seconds on the caller's monotonic clock (python time.monotonic per
//! datagram); nothing here is rounded to the 128 hz tick.

use std::collections::BTreeSet;

use aries_bits::{BitWriter, Bits};
use aries_rep::block::{Target, append_rep_block, append_stable_child_export};
use aries_rep::{RepError, RepResult};

/// UStateMachineComponent AuthCurrentState.CurrentState (range_equippables.py:19).
pub const AUTH_CURRENT_STATE_HANDLE: u32 = 3;
/// UResourceComponent AuthResourceAmount, +0x110 (range_equippables.py:40).
pub const AUTH_RESOURCE_AMOUNT_HANDLE: u32 = 3;
/// default amount seed: UAmmoComponent.Ammo (range_equippables.py:36).
pub const AMMO_DEFAULT_OFFSET: u32 = 0x12C;
/// default amount seed for charges: MaxCharges (range_equippables.py:255-257, 268).
pub const CHARGES_DEFAULT_OFFSET: u32 = 0x188;
/// Gun_C ammo components (range_equippables.py:324-327, range_gun_loader.py:238-239).
pub const MAGAZINE_AMMO_OFFSET: u32 = 0x1208;
pub const RESERVE_AMMO_OFFSET: u32 = 0x1200;
/// range ability refill delay after a use (range_equippables.py:269).
pub const ABILITY_REFILL_AFTER: f64 = 4.0;

/// the pawn: outer of the shield section and the seed of its channel (range_equippables.py:151).
pub const PAWN_ACTOR_GUID: u32 = 10;
/// AShooterCharacter "ShieldDamageSection" (range_equippables.py:152).
pub const SHIELD_SECTION_GUID: u32 = 316;
pub const SHIELD_SECTION_NAME: &str = "ShieldDamageSection";
/// UChildDamageSectionComponent net props after ActorComponent's two: bAlive
/// +0x120, Life +0x124, MaximumLife +0x128 (range_equippables.py:153-156).
pub const DAMAGE_SECTION_ALIVE_HANDLE: u32 = 3;
pub const DAMAGE_SECTION_LIFE_HANDLE: u32 = 4;
pub const DAMAGE_SECTION_MAXIMUM_LIFE_HANDLE: u32 = 5;
/// shield per armor class (range_equippables.py:157), python order.
pub const ARMOR_SHIELD_VALUES: [(&str, f64); 3] = [("LightArmorItem_C", 25.0), ("HeavyArmorItem_C", 50.0), ("PlasmaArmorItem_C", 25.0)];
/// shield for an armor class missing from the table (range_udp_handshake.py:1203).
pub const ARMOR_SHIELD_FALLBACK: f64 = 25.0;

/// starting BasePistol on ch9 (range_server_bunch.py:755-756, range_equippables.py:320-321).
pub const BASE_PISTOL_CHANNEL: u32 = 9;
pub const BASE_PISTOL_ACTOR_GUID: u32 = 94;
pub const BASE_PISTOL_STATE_MACHINE_GUID: u32 = 96;
pub const BASE_PISTOL_SLOT: u32 = 1;
/// live SM+0x160 names and their export guids (range_server_bunch.py:757-767).
/// 98/99 are the proven Inactive and legacy Equip exports.
pub const BASE_PISTOL_STATE_GUIDS: [(&str, u32); 9] = [
    ("InactiveState", 98),
    ("EquipState", 99),
    ("IdleWaitingForEventState", 240),
    ("IdleState", 242),
    ("FiringState", 244),
    ("ReloadState", 246),
    ("ReadyingState", 248),
    ("JammedState", 250),
    ("FiringStateBurst", 252),
];
/// pistol ammo netguids: they follow the knife states 256-304 (range_equippables.py:326-327).
pub const BASE_PISTOL_MAGAZINE_GUID: u32 = 308;
pub const BASE_PISTOL_RESERVE_GUID: u32 = 306;

/// wushu's AShooterCharacter.StartingEquippableClasses in live 13.06 order:
/// (archetype, package, EAresItemSlot). private copy of range_server_bunch.py:680-691.
pub(crate) const STARTING_EQUIPPABLES: [(&str, &str, u32); 7] = [
    ("Default__Equippable_Unarmed_C", "/Game/Characters/_Core/Equippable_Unarmed", 10),
    ("Default__Ability_Melee_Base_C", "/Game/Equippables/Melee/Ability_Melee_Base", 2),
    ("Default__Ability_Wushu_4_Smoke_C", "/Game/Characters/Wushu/S0/Ability_4/Ability_Wushu_4_Smoke", 3),
    ("Default__Ability_Wushu_E_Dash_C", "/Game/Characters/Wushu/S0/Ability_E/Ability_Wushu_E_Dash", 5),
    ("Default__Ability_Wushu_Q_CycloneBoost_C", "/Game/Characters/Wushu/S0/Ability_Q/Ability_Wushu_Q_CycloneBoost", 4),
    ("Default__Ability_Wushu_X_Dagger_Production_C", "/Game/Characters/Wushu/S0/Ability_X/Ability_Wushu_X_Dagger_Production", 9),
    ("Default__Ability_Wushu_Passive_Glide_C", "/Game/Characters/Wushu/S0/Glide/Ability_Wushu_Passive_Glide", 6),
];
/// range_server_bunch.py:692-693.
pub(crate) const STARTING_EQUIPPABLE_FIRST_CHANNEL: u32 = 12;
const STARTING_EQUIPPABLE_FIRST_GUID: u32 = 200;

/// actor (even), archetype and package (odd) of starting equippable `index`
/// (range_server_bunch.py:701-706).
pub(crate) const fn starting_equippable_guids(index: usize) -> (u32, u32, u32) {
    let actor = STARTING_EQUIPPABLE_FIRST_GUID + 4 * index as u32;
    (actor, actor + 1, actor + 3)
}

/// state name -> export guid, in python dict insertion order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateGuids {
    entries: Vec<(String, u32)>,
}

impl StateGuids {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<u32> {
        self.entries.iter().find(|(key, _)| key == name).map(|&(_, guid)| guid)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// python dict assignment: an existing name keeps its place.
    pub fn insert(&mut self, name: impl Into<String>, guid: u32) {
        let name = name.into();
        match self.entries.iter_mut().find(|(key, _)| *key == name) {
            Some(entry) => entry.1 = guid,
            None => self.entries.push((name, guid)),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, u32)> {
        self.entries.iter().map(|(name, guid)| (name.as_str(), *guid))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<S: Into<String>> FromIterator<(S, u32)> for StateGuids {
    fn from_iter<I: IntoIterator<Item = (S, u32)>>(iter: I) -> Self {
        let mut guids = Self::new();
        for (name, guid) in iter {
            guids.insert(name, guid);
        }
        guids
    }
}

/// a stably-named UAmmoComponent (UResourceComponent) default subobject
/// (range_equippables.py:22-37). handles follow offset order: bReplicates 1,
/// bIsActive 2, AuthResourceAmount 3; FIntProperty sends 32 raw bits.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceSubobject {
    /// default-subobject name, e.g. "MagazineAmmo"
    pub name: String,
    /// even netguid we export it under
    pub guid: u32,
    /// pointer to the component inside the item actor
    pub item_offset: u32,
    /// where the default amount sits in the component
    pub default_offset: u32,
    /// range ability refill delay after a use
    pub refill_after: Option<f64>,
}

impl ResourceSubobject {
    /// an ammo component seeded from UAmmoComponent.Ammo, no refill.
    pub fn ammo(name: impl Into<String>, guid: u32, item_offset: u32) -> Self {
        Self { name: name.into(), guid, item_offset, default_offset: AMMO_DEFAULT_OFFSET, refill_after: None }
    }
}

/// a further UEquippableStateMachineComponent in the item's machine array
/// (item+0xE30), e.g. a rifle's ZoomStateMachine for right-click ads. without an
/// authoritative state the client resets it to no state (range_equippables.py:43-51).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraStateMachine {
    pub name: String,
    pub guid: u32,
    pub starting_state: String,
    pub state_guids: StateGuids,
}

/// the mirror's view of one replicated item (range_equippables.py:54-73).
#[derive(Debug, Clone, PartialEq)]
pub struct EquippableActor {
    pub key: String,
    /// live UClass name, e.g. "BasePistol_C"
    pub class_name: String,
    pub channel: u32,
    pub actor_guid: u32,
    /// EAresItemSlot the item sits in
    pub slot: u32,
    pub state_machine_guid: u32,
    pub state_guids: StateGuids,
    /// ammo (guns) or charges (abilities), in send order
    pub resources: Vec<ResourceSubobject>,
    pub extra_machines: Vec<ExtraStateMachine>,
    /// false: only the resources are replicated
    pub mirror_states: bool,
}

impl EquippableActor {
    /// python EquippableActor(...) with no resources or extra machines.
    pub fn new(key: impl Into<String>, class_name: impl Into<String>, channel: u32, actor_guid: u32, slot: u32, state_machine_guid: u32, state_guids: StateGuids) -> RepResult<Self> {
        let actor = Self {
            key: key.into(),
            class_name: class_name.into(),
            channel,
            actor_guid,
            slot,
            state_machine_guid,
            state_guids,
            resources: Vec::new(),
            extra_machines: Vec::new(),
            mirror_states: true,
        };
        actor.validate()?;
        Ok(actor)
    }

    pub fn with_resources(mut self, resources: Vec<ResourceSubobject>) -> Self {
        self.resources = resources;
        self
    }

    pub fn with_extra_machines(mut self, extra_machines: Vec<ExtraStateMachine>) -> Self {
        self.extra_machines = extra_machines;
        self
    }

    /// mirror_states=False: a resource-only entry (wushu charges).
    pub fn resources_only(mut self) -> Self {
        self.mirror_states = false;
        self
    }

    /// python __post_init__ (range_equippables.py:67-73), same messages.
    pub fn validate(&self) -> RepResult<()> {
        if self.actor_guid == 0 || self.actor_guid & 1 != 0 {
            return Err(invalid("dynamic actor NetGUID must be positive and even"));
        }
        if self.slot >= 16 {
            return Err(invalid("inventory slot out of range"));
        }
        if !self.state_guids.contains("InactiveState") {
            return Err(invalid("every equippable state machine has an InactiveState"));
        }
        Ok(())
    }
}

fn invalid(message: &str) -> RepError {
    RepError::Invalid(message.to_string())
}

/// python _auth_state_body (range_equippables.py:99-116): optional stable child
/// export of the state (outer = the actor), then AuthCurrentState on the machine.
/// the bunch must carry has_exports exactly when `export`.
pub fn auth_state_body(actor_guid: u32, machine_guid: u32, state_guid: u32, state_name: &str, export: bool) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    if export {
        append_stable_child_export(&mut w, state_guid, actor_guid, state_name);
    }
    let mut rep = BitWriter::new();
    rep.append_bits(0, 1);
    rep.append_packed_uint(AUTH_CURRENT_STATE_HANDLE as u64);
    rep.append_packed_uint(state_guid as u64);
    rep.append_packed_uint(0);
    append_rep_block(&mut w, Target::Subobject(machine_guid), &rep.finish()?, actor_guid)?;
    Ok(w.finish()?)
}

/// python build_auth_state_body (range_equippables.py:88-96).
pub fn build_auth_state_body(equippable: &EquippableActor, state_name: &str, export: bool) -> RepResult<Bits> {
    let state_guid = equippable.state_guids.get(state_name).ok_or_else(|| unknown_state(state_name))?;
    auth_state_body(equippable.actor_guid, equippable.state_machine_guid, state_guid, state_name, export)
}

fn unknown_state(state_name: &str) -> RepError {
    RepError::Invalid(format!("unknown state {state_name:?}"))
}

/// python build_machine_open_body (range_equippables.py:119-134): exports a
/// stably-named machine with an empty layout, creating its client replicator.
pub fn build_machine_open_body(actor_guid: u32, machine_guid: u32, machine_name: &str) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    append_stable_child_export(&mut w, machine_guid, actor_guid, machine_name);
    append_rep_block(&mut w, Target::Subobject(machine_guid), &aries_rep::props::empty_rep(), actor_guid)?;
    Ok(w.finish()?)
}

/// python build_machine_state_body (range_equippables.py:137-141).
pub fn build_machine_state_body(equippable: &EquippableActor, machine: &ExtraStateMachine, state_name: &str, export: bool) -> RepResult<Bits> {
    let state_guid = machine.state_guids.get(state_name).ok_or_else(|| unknown_state(state_name))?;
    auth_state_body(equippable.actor_guid, machine.guid, state_guid, state_name, export)
}

/// RemoteRole (handle 4, 3-bit ENetRole) = AutonomousProxy on the channel actor:
/// python gun_role_body / build_melee_autonomous_role_body (range_gun_loader.py:194-203,
/// range_melee_equippable.py:60-74). the 8-bit write caused "Error reading RepIndex".
pub fn remote_role_body(actor_guid: u32) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    append_rep_block(&mut w, Target::Actor, &aries_rep::props::autonomous_role(3, 2)?, actor_guid)?;
    Ok(w.finish()?)
}

/// shield of an armor class, python armor_shield_values.get (range_equippables.py:157).
pub fn armor_shield_value(class_name: &str) -> Option<f64> {
    ARMOR_SHIELD_VALUES.iter().find(|(class, _)| *class == class_name).map(|&(_, shield)| shield)
}

/// python build_shield_section_body (range_equippables.py:160-184): the pawn's
/// ShieldDamageSection bAlive / Life / MaximumLife on ch8. floats go out as f32
/// bits (struct.pack("<f") rounds the double to nearest).
pub fn build_shield_section_body(life: f64, maximum: f64, export: bool) -> RepResult<Bits> {
    if !(0.0 <= life && life <= maximum && maximum <= 1000.0) {
        return Err(invalid("shield values out of range"));
    }
    let mut w = BitWriter::new();
    if export {
        append_stable_child_export(&mut w, SHIELD_SECTION_GUID, PAWN_ACTOR_GUID, SHIELD_SECTION_NAME);
    }
    let mut rep = BitWriter::new();
    rep.append_bits(0, 1);
    rep.append_packed_uint(DAMAGE_SECTION_ALIVE_HANDLE as u64);
    rep.append_bits(1, 1);
    rep.append_packed_uint(DAMAGE_SECTION_LIFE_HANDLE as u64);
    rep.append_f32(life as f32);
    rep.append_packed_uint(DAMAGE_SECTION_MAXIMUM_LIFE_HANDLE as u64);
    rep.append_f32(maximum as f32);
    rep.append_packed_uint(0);
    append_rep_block(&mut w, Target::Subobject(SHIELD_SECTION_GUID), &rep.finish()?, PAWN_ACTOR_GUID)?;
    Ok(w.finish()?)
}

/// python build_resource_amount_body (range_equippables.py:187-207):
/// <resource>.AuthResourceAmount = amount on the item's actor channel.
pub fn build_resource_amount_body(equippable: &EquippableActor, resource: &ResourceSubobject, amount: i64, export: bool) -> RepResult<Bits> {
    if !(0..1i64 << 31).contains(&amount) {
        return Err(invalid("resource amount must be a non-negative int32"));
    }
    let mut w = BitWriter::new();
    if export {
        append_stable_child_export(&mut w, resource.guid, equippable.actor_guid, &resource.name);
    }
    let mut rep = BitWriter::new();
    rep.append_bits(0, 1);
    rep.append_packed_uint(AUTH_RESOURCE_AMOUNT_HANDLE as u64);
    rep.append_bits(amount as u64, 32);
    rep.append_packed_uint(0);
    append_rep_block(&mut w, Target::Subobject(resource.guid), &rep.finish()?, equippable.actor_guid)?;
    Ok(w.finish()?)
}

/// per connection and resource: what we last made authoritative
/// (range_equippables.py:210-214).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ResourceMirrorState {
    /// last AuthResourceAmount we replicated
    pub auth: Option<i64>,
    /// the client's prediction once it applied our value
    pub baseline: Option<i64>,
    /// when auth was last sent
    pub sent_at: f64,
}

impl ResourceMirrorState {
    /// the first send exports the component (range_udp_handshake.py:1660).
    pub fn needs_export(&self) -> bool {
        self.auth.is_none()
    }
}

/// python resource_target (range_equippables.py:217-242): the amount to make
/// authoritative now, or None. the first send seeds the component's default;
/// nothing more goes out until the client applied our last value, the
/// prediction seen then is the baseline, and only a later change of it (a shot,
/// a reload) is confirmed. a prediction that changes between our send and its
/// application is absorbed into the baseline (kept for parity, 07 5.3).
pub fn resource_target(state: &mut ResourceMirrorState, predicted: i64, authoritative: i64, default_amount: i64, now: f64, refill_after: Option<f64>) -> Option<i64> {
    let Some(auth) = state.auth else {
        return (0..10000).contains(&default_amount).then_some(default_amount);
    };
    if authoritative != auth {
        return None;
    }
    let Some(baseline) = state.baseline else {
        state.baseline = Some(predicted);
        return None;
    };
    if predicted != baseline && predicted != auth && (0..10000).contains(&predicted) {
        return Some(predicted);
    }
    // range abilities refill: a used charge goes straight back to full after a pause
    if let Some(refill_after) = refill_after
        && auth < default_amount
        && default_amount < 10000
        && now - state.sent_at >= refill_after
    {
        return Some(default_amount);
    }
    None
}

/// python resource_confirmed (range_equippables.py:245-249).
pub fn resource_confirmed(state: &mut ResourceMirrorState, amount: i64, now: f64) {
    state.auth = Some(amount);
    state.baseline = None;
    state.sent_at = now;
}

/// per connection and machine: what we last made authoritative
/// (range_equippables.py:278-284).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorState {
    pub auth: String,
    pub exported: BTreeSet<String>,
    /// the client has predicted a non-Inactive state
    pub active: bool,
    /// we pushed EquipState for a correction-equipped item (never reset)
    pub kicked: bool,
}

impl Default for MirrorState {
    fn default() -> Self {
        Self::starting_at("InactiveState")
    }
}

impl MirrorState {
    /// auth = exported = {state}, inactive: the mirror's setdefault for an extra
    /// machine (range_udp_handshake.py:1682-1685).
    pub fn starting_at(state: &str) -> Self {
        Self { auth: state.to_string(), exported: BTreeSet::from([state.to_string()]), active: false, kicked: false }
    }

    /// after send_extra_machines exported and confirmed the starting state
    /// (range_udp_handshake.py:1127-1128).
    pub fn extra_machine_sent(starting_state: &str) -> Self {
        Self { active: true, ..Self::starting_at(starting_state) }
    }

    /// the send exports the state the first time it names it (range_udp_handshake.py:1704).
    pub fn needs_export(&self, target: &str) -> bool {
        !self.exported.contains(target)
    }

    /// after the send (range_udp_handshake.py:1707-1708).
    pub fn confirm(&mut self, target: &str) {
        self.exported.insert(target.to_string());
        self.auth = target.to_string();
    }
}

/// python mirror_target (range_equippables.py:287-309): the state to make
/// authoritative now, or None.
/// - confirm every named state the client predicts.
/// - a server correction can make an item current while its machine still
///   predicts Inactive: push EquipState once so it equips.
/// - never confirm Inactive before the client left it: a stale Inactive right
///   after the push must not undo it.
pub fn mirror_target(equippable: &EquippableActor, state: &mut MirrorState, predicted: Option<&str>, is_current: bool) -> Option<String> {
    let predicted = predicted.filter(|name| equippable.state_guids.contains(name))?;
    if predicted != "InactiveState" {
        state.active = true;
    }
    if is_current && !state.active && !state.kicked && equippable.state_guids.contains("EquipState") {
        state.kicked = true;
        return Some("EquipState".to_string());
    }
    if predicted == state.auth || (predicted == "InactiveState" && !state.active) {
        return None;
    }
    Some(predicted.to_string())
}

/// python machine_mirror_target (range_equippables.py:144-148): an extra
/// machine's prediction whenever it names a known state other than auth.
pub fn machine_mirror_target(state: &MirrorState, predicted: Option<&str>, state_guids: &StateGuids) -> Option<String> {
    predicted.filter(|name| state_guids.contains(name) && *name != state.auth).map(str::to_string)
}

/// python starting_equippables (range_equippables.py:312-332): the classic and
/// the knife, the two starting items with replicated state machines.
pub fn starting_equippables() -> [EquippableActor; 2] {
    let pistol = EquippableActor::new(
        "BasePistol",
        "BasePistol_C",
        BASE_PISTOL_CHANNEL,
        BASE_PISTOL_ACTOR_GUID,
        BASE_PISTOL_SLOT,
        BASE_PISTOL_STATE_MACHINE_GUID,
        BASE_PISTOL_STATE_GUIDS.iter().copied().collect(),
    )
    .expect("fixed pistol layout")
    .with_resources(vec![
        ResourceSubobject::ammo("MagazineAmmo", BASE_PISTOL_MAGAZINE_GUID, MAGAZINE_AMMO_OFFSET),
        ResourceSubobject::ammo("ReserveAmmo", BASE_PISTOL_RESERVE_GUID, RESERVE_AMMO_OFFSET),
    ]);
    let knife = EquippableActor::new(
        "Melee",
        "Ability_Melee_Base_C",
        crate::melee::MELEE_CHANNEL,
        crate::melee::MELEE_ACTOR_GUID,
        crate::melee::MELEE_SLOT,
        crate::melee::MELEE_STATE_MACHINE_GUID,
        crate::melee::melee_state_guids(),
    )
    .expect("fixed knife layout");
    [pistol, knife]
}

/// python starting_ability_resources (range_equippables.py:252-275): wushu's
/// charge components, mirrored so the ability hud shows charges. offsets are the
/// live actor members holding each scs component (pointer scan on 13.06). order:
/// cloudburst (C, 2 charges), updraft (Q, 1), tailwind (E, 1).
pub fn starting_ability_resources() -> [EquippableActor; 3] {
    let ability = |index: usize, component: &str, offset: u32, guid: u32| {
        let (archetype, _, slot) = STARTING_EQUIPPABLES[index];
        let class_name = archetype.strip_prefix("Default__").unwrap_or(archetype);
        EquippableActor::new(
            class_name,
            class_name,
            STARTING_EQUIPPABLE_FIRST_CHANNEL + index as u32,
            starting_equippable_guids(index).0,
            slot,
            0,
            StateGuids::from_iter([("InactiveState", 0)]),
        )
        .expect("fixed ability layout")
        .with_resources(vec![ResourceSubobject {
            name: component.to_string(),
            guid,
            item_offset: offset,
            default_offset: CHARGES_DEFAULT_OFFSET,
            refill_after: Some(ABILITY_REFILL_AFTER),
        }])
        .resources_only()
    };
    [
        ability(2, "EquipmentCharge", 0x1240, 310),
        ability(4, "EquipmentCharge", 0x12A0, 312),
        ability(3, "SignatureAbilityResource", 0x1290, 314),
    ]
}
