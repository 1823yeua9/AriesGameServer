//! wushu's starting abilities made usable (range_abilities.py, spec 08 section 2):
//! the per-connection guid allocator, the four ability loads, their three setup
//! bodies, the AbilityTuning open used at B18, the registration plan that hands
//! out state guids from oracle data, and the job pacing (ruh:1130-1170, 2064-2078).
//!
//! ra = range_abilities.py, rsb = range_server_bunch.py, re = range_equippables.py,
//! rgl = range_gun_loader.py, ruh = range_udp_handshake.py.

use std::collections::HashMap;

use aries_bits::{BitWriter, Bits};
use aries_rep::block::{Target, append_dynamic_actor_header, append_export_header, append_rep_block, append_stable_child_entry, append_stable_child_export, append_static_export_entry};
use aries_rep::props::{autonomous_role, empty_rep};
use aries_rep::{RepError, RepResult};

use crate::bots::{ChannelSend, py_pack_f32};

/// ra:23: Cloudburst (C), Tailwind (E), Updraft (Q), Blade Storm (X).
pub const ABILITY_INDICES: [usize; 4] = [2, 3, 4, 5];
/// ra:25: ability machine/state guids start clear of the purchase allocator.
pub const FIRST_ABILITY_GUID: u32 = 100000;
/// rsb:692-693: starting equippable i is on channel 12 + i, actor guid 200 + 4i.
pub const STARTING_EQUIPPABLE_FIRST_CHANNEL: u32 = 12;
pub const STARTING_EQUIPPABLE_FIRST_GUID: u32 = 200;
/// rsb:680-691 (len 7): starting_equippable_index reaching it gates the jobs (ruh:2067).
pub const STARTING_EQUIPPABLE_COUNT: usize = 7;
/// rgl:196 -> rsb:1463 (build_base_pistol_autonomous_role_payload(3)): RemoteRole
/// is a 3-bit ENetRole set to AutonomousProxy (2).
pub const ROLE_BIT_WIDTH: u32 = 3;
pub const AUTONOMOUS_PROXY: u32 = 2;
/// re:19: UStateMachineComponent AuthCurrentState.CurrentState.
pub const AUTH_CURRENT_STATE_HANDLE: u32 = 3;
pub const STATE_MACHINE_NAME: &str = "EquippableStateMachine";
pub const INACTIVE_STATE: &str = "InactiveState";
/// ra:194: the Range level.
const LEVEL_GUID: u32 = 5;

/// rsb:683-688: (archetype, package, EAresItemSlot) of the starting abilities 2-5.
const STARTING_ABILITIES: [(&str, &str, u32); 4] = [
    ("Default__Ability_Wushu_4_Smoke_C", "/Game/Characters/Wushu/S0/Ability_4/Ability_Wushu_4_Smoke", 3),
    ("Default__Ability_Wushu_E_Dash_C", "/Game/Characters/Wushu/S0/Ability_E/Ability_Wushu_E_Dash", 5),
    ("Default__Ability_Wushu_Q_CycloneBoost_C", "/Game/Characters/Wushu/S0/Ability_Q/Ability_Wushu_Q_CycloneBoost", 4),
    ("Default__Ability_Wushu_X_Dagger_Production_C", "/Game/Characters/Wushu/S0/Ability_X/Ability_Wushu_X_Dagger_Production", 9),
];

/// (archetype, package, slot) of starting equippable `index` (2..=5 only).
fn starting_ability(index: usize) -> Option<(&'static str, &'static str, u32)> {
    ABILITY_INDICES.iter().position(|&i| i == index).map(|row| STARTING_ABILITIES[row])
}

/// rsb:701-706: actor (even), archetype and package (odd) guids of starting equippable `index`.
pub fn starting_equippable_guids(index: usize) -> (u32, u32, u32) {
    let actor = STARTING_EQUIPPABLE_FIRST_GUID + 4 * index as u32;
    (actor, actor + 1, actor + 3)
}

fn invalid(message: &str) -> RepError {
    RepError::Invalid(message.into())
}

// ---------------------------------------------------------------- allocation and loads

/// ra:39-40 -> rgl:46-62: PurchaseAllocator(next_channel=0, next_guid=100000).
/// only dynamic_guid is used for abilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbilityAllocator {
    pub next_channel: u32,
    pub next_guid: u32,
}

impl Default for AbilityAllocator {
    fn default() -> Self {
        Self { next_channel: 0, next_guid: FIRST_ABILITY_GUID }
    }
}

impl AbilityAllocator {
    /// rgl:56-62: bump to even, hand it out, step 2.
    pub fn dynamic_guid(&mut self) -> u32 {
        if self.next_guid & 1 != 0 {
            self.next_guid += 1;
        }
        let guid = self.next_guid;
        self.next_guid += 2;
        guid
    }
}

/// ra:28-36
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbilityLoad {
    pub index: usize,
    pub class_name: &'static str,
    pub channel: u32,
    pub actor_guid: u32,
    pub slot: u32,
    pub state_machine_guid: u32,
    pub inactive_guid: u32,
}

/// ra:43-50: fixed channel/actor/slot, then two fresh guids (machine, InactiveState).
pub fn plan_ability_load(index: usize, allocator: &mut AbilityAllocator) -> RepResult<AbilityLoad> {
    let (archetype, _, slot) = starting_ability(index).ok_or_else(|| invalid("not a usable starting ability"))?;
    let state_machine_guid = allocator.dynamic_guid();
    let inactive_guid = allocator.dynamic_guid();
    Ok(AbilityLoad {
        index,
        class_name: archetype.strip_prefix("Default__").unwrap_or(archetype),
        channel: STARTING_EQUIPPABLE_FIRST_CHANNEL + index as u32,
        actor_guid: starting_equippable_guids(index).0,
        slot,
        state_machine_guid,
        inactive_guid,
    })
}

// ---------------------------------------------------------------- setup bodies

/// re:99-116: AuthCurrentState = the state on a state-machine subobject,
/// optionally exporting the stably-named state (outer = the item actor).
fn auth_state_body(actor_guid: u32, machine_guid: u32, state_guid: u32, state_name: &str, export: bool) -> RepResult<Bits> {
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

/// re:119-134: export a stably-named state machine with an empty layout.
fn machine_open_body(actor_guid: u32, machine_guid: u32, machine_name: &str) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    append_stable_child_export(&mut w, machine_guid, actor_guid, machine_name);
    append_rep_block(&mut w, Target::Subobject(machine_guid), &empty_rep(), actor_guid)?;
    Ok(w.finish()?)
}

/// ra:53-55 -> rgl:194-203: RemoteRole = AutonomousProxy on the actor (30 bits).
pub fn ability_role_body(load: &AbilityLoad) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    append_rep_block(&mut w, Target::Actor, &autonomous_role(ROLE_BIT_WIDTH, AUTONOMOUS_PROXY)?, load.actor_guid)?;
    Ok(w.finish()?)
}

/// ra:58-59: the EquippableStateMachine subobject (381 bits at production guids).
pub fn ability_state_machine_open_body(load: &AbilityLoad) -> RepResult<Bits> {
    machine_open_body(load.actor_guid, load.state_machine_guid, STATE_MACHINE_NAME)
}

/// ra:73-74: AuthCurrentState = InactiveState, exported (341 bits).
pub fn ability_initial_state_body(load: &AbilityLoad) -> RepResult<Bits> {
    auth_state_body(load.actor_guid, load.state_machine_guid, load.inactive_guid, INACTIVE_STATE, true)
}

/// ra:204-210: the three ability-channel bodies in order with their has_exports.
pub fn ability_setup_bodies(load: &AbilityLoad) -> RepResult<[(Bits, bool); 3]> {
    Ok([(ability_role_body(load)?, false), (ability_state_machine_open_body(load)?, true), (ability_initial_state_body(load)?, true)])
}

// ---------------------------------------------------------------- AbilityTuning

/// ra:83-86: UAbilityTuningComponent ReplicatedAbilityTunings and its element leaves.
pub const ABILITY_TUNING_ARRAY_HANDLE: u32 = 3;
pub const ABILITY_TUNING_LEAF_COUNT: u32 = 6;
pub const TUNING_TYPE_BITS: u32 = 3;
/// ra:97: the component's stable name.
pub const ABILITY_TUNING_COMPONENT: &str = "AbilityTuning";
/// ra:105: element count and tag width the builder accepts.
pub const MAX_TUNING_ENTRIES: usize = 32;
pub const MAX_TAG_BITS: u32 = 16;

/// a tuning value; the kind follows the variant (ra:86 EAbilityTuningValueType:
/// int 1, float 2). bool rows exist in the enum but no production row uses them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TuningValue {
    Int(i64),
    Float(f64),
}

impl TuningValue {
    fn type_id(self) -> u64 {
        match self {
            TuningValue::Int(_) => 1,
            TuningValue::Float(_) => 2,
        }
    }
}

/// ra:89-93
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TuningEntry {
    pub tag_net_index: u32,
    pub value: TuningValue,
}

/// ra:148-155: rows each ability blueprint reads at BeginPlay; they must ride the open bunch.
pub const STARTING_ABILITY_TUNINGS: [(usize, &[(&str, TuningValue)]); 3] = [
    (2, &[("AbilityTuning.Wushu.4.SmokeDuration", TuningValue::Float(2.5)), ("AbilityTuning.Wushu.4.SmokeMaxCharges", TuningValue::Int(2))]),
    (
        3,
        &[
            ("AbilityTuning.Wushu.E.DashVelocityMultiplier", TuningValue::Float(1.0)),
            ("AbilityTuning.Wushu.E.PrimeDuration", TuningValue::Float(7.5)),
            ("AbilityTuning.Wushu.E.MaxCharges", TuningValue::Int(1)),
        ],
    ),
    (4, &[("AbilityTuning.Wushu.Q.MaxCharges", TuningValue::Int(1))]),
];

/// tuning rows of ability `index` (2, 3, 4), or none.
pub fn starting_ability_tunings(index: usize) -> Option<&'static [(&'static str, TuningValue)]> {
    STARTING_ABILITY_TUNINGS.iter().find(|(i, _)| *i == index).map(|(_, rows)| *rows)
}

/// every tag name the B18 tuning opens need (the oracle reads just these).
pub fn starting_ability_tuning_tags() -> impl Iterator<Item = &'static str> {
    STARTING_ABILITY_TUNINGS.iter().flat_map(|(_, rows)| rows.iter().map(|(tag, _)| *tag))
}

/// ra:158-160: static even guid of ability `index`'s AbilityTuning component.
pub fn ability_tuning_guid(index: usize) -> u32 {
    190000 + 2 * index as u32
}

/// ra:96-140 with export=False (the production form, ra:197): the
/// ReplicatedAbilityTunings content block on the component subobject.
pub fn build_ability_tuning_body(actor_guid: u32, component_guid: u32, entries: &[TuningEntry], tag_bits: u32) -> RepResult<Bits> {
    if entries.is_empty() || entries.len() > MAX_TUNING_ENTRIES || !(1..=MAX_TAG_BITS).contains(&tag_bits) {
        return Err(invalid("tuning entry count or tag width out of range"));
    }
    let mut rep = BitWriter::new();
    rep.append_bits(0, 1);
    rep.append_packed_uint(ABILITY_TUNING_ARRAY_HANDLE as u64);
    rep.append_bits(entries.len() as u64, 16);
    for (element, entry) in entries.iter().enumerate() {
        if entry.tag_net_index >= 1 << tag_bits {
            return Err(invalid("tuning entry outside the supported shape"));
        }
        let first = element as u64 * ABILITY_TUNING_LEAF_COUNT as u64;
        rep.append_packed_uint(first + 1);
        rep.append_bits(entry.tag_net_index as u64, tag_bits);
        rep.append_packed_uint(first + 2);
        rep.append_bits(entry.value.type_id(), TUNING_TYPE_BITS);
        match entry.value {
            TuningValue::Int(value) => {
                rep.append_packed_uint(first + 4);
                rep.append_bits(value as u64 & 0xFFFF_FFFF, 32);
            }
            TuningValue::Float(value) => {
                rep.append_packed_uint(first + 5);
                rep.append_u32(py_pack_f32(value)?);
            }
        }
    }
    rep.append_packed_uint(0); // end of the array's element handles
    rep.append_packed_uint(0); // end of the component's properties
    let mut w = BitWriter::new();
    append_rep_block(&mut w, Target::Subobject(component_guid), &rep.finish()?, actor_guid)?;
    Ok(w.finish()?)
}

/// ra:163-201: open ability `index` with its AbilityTuning values in the same
/// bunch: archetype + stably-named component exports, the default-transform
/// header, then the tuning block. `tag_indices` maps complete tag names to net
/// indices (the client's own, read by the oracle); `tag_bits` is TrueBitNum.
pub fn build_ability_open_with_tuning(index: usize, tag_indices: &HashMap<String, u32>, tag_bits: u32) -> RepResult<Bits> {
    let rows = starting_ability_tunings(index).ok_or_else(|| invalid("ability has no tuning rows"))?;
    let mut entries = Vec::with_capacity(rows.len());
    for (tag, value) in rows {
        let tag_net_index = *tag_indices.get(*tag).ok_or_else(|| RepError::Invalid(format!("no net index for tag {tag}")))?;
        entries.push(TuningEntry { tag_net_index, value: *value });
    }
    let (archetype, package, _) = starting_ability(index).ok_or_else(|| invalid("not a usable starting ability"))?;
    let (actor_guid, archetype_guid, package_guid) = starting_equippable_guids(index);
    let tuning_guid = ability_tuning_guid(index);

    let mut w = BitWriter::new();
    append_export_header(&mut w, 2);
    append_static_export_entry(&mut w, archetype, package, archetype_guid, package_guid);
    // outer: guid only, the actor is spawned by this bunch
    append_stable_child_entry(&mut w, tuning_guid, actor_guid, ABILITY_TUNING_COMPONENT);
    append_dynamic_actor_header(&mut w, actor_guid, archetype_guid, LEVEL_GUID, None);
    w.append_bits_value(&build_ability_tuning_body(actor_guid, tuning_guid, &entries, tag_bits)?);
    let body = w.finish()?;
    if body.bit_count >= 1 << 13 {
        return Err(invalid("ability open exceeds the bunch size limit"));
    }
    Ok(body)
}

// ---------------------------------------------------------------- registration

/// the oracle's read of an ability's inventory slot (read_slot_machines, res:132-164).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotMachine {
    pub item: u64,
    pub class_name: Option<String>,
    /// item+0xE00
    pub state_machine: u64,
    /// the main machine's state names in SM+0x160 order (the reader's cached names)
    pub state_names: Vec<String>,
}

/// one machine of the item's machine array (read_item_machines, res:177-202).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemMachine {
    pub name: String,
    pub address: u64,
    /// the state named by machine+0x1E8, if any
    pub starting: Option<String>,
    pub state_names: Vec<String>,
}

/// re:43-51: a further state machine of the item (e.g. Tailwind's StateMachine_Priming).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraMachine {
    pub name: String,
    pub guid: u32,
    pub starting_state: String,
    /// state name -> guid, in the machine's state order
    pub state_guids: Vec<(String, u32)>,
}

impl ExtraMachine {
    pub fn state_guid(&self, name: &str) -> Option<u32> {
        guid_of(&self.state_guids, name)
    }
}

/// ra:62-70: the mirror's view of a registered ability, keyed like its
/// resource-only entry (re:252-275) so the charge state carries over. the
/// resources are that entry's (ruh:1161-1162); the caller attaches them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbilityEquippable {
    pub key: &'static str,
    pub class_name: &'static str,
    pub channel: u32,
    pub actor_guid: u32,
    pub slot: u32,
    pub state_machine_guid: u32,
    /// InactiveState first, then the machine's other states in order
    pub state_guids: Vec<(String, u32)>,
    pub extra_machines: Vec<ExtraMachine>,
    pub mirror_states: bool,
}

impl AbilityEquippable {
    pub fn state_guid(&self, name: &str) -> Option<u32> {
        guid_of(&self.state_guids, name)
    }
}

fn guid_of(guids: &[(String, u32)], name: &str) -> Option<u32> {
    guids.iter().find(|(state, _)| state == name).map(|(_, guid)| *guid)
}

/// python dict assignment on an ordered (name, guid) list: a repeated name keeps
/// its first position and takes the new guid.
fn put_guid(guids: &mut Vec<(String, u32)>, name: &str, guid: u32) {
    match guids.iter_mut().find(|(state, _)| state == name) {
        Some(entry) => entry.1 = guid,
        None => guids.push((name.to_string(), guid)),
    }
}

/// ra:62-70: {"InactiveState": the load's guid} updated with `state_guids`.
pub fn ability_equippable(load: &AbilityLoad, state_guids: &[(String, u32)], extra_machines: Vec<ExtraMachine>) -> AbilityEquippable {
    let mut guids = vec![(INACTIVE_STATE.to_string(), load.inactive_guid)];
    for (name, guid) in state_guids {
        put_guid(&mut guids, name, *guid);
    }
    AbilityEquippable {
        key: load.class_name,
        class_name: load.class_name,
        channel: load.channel,
        actor_guid: load.actor_guid,
        slot: load.slot,
        state_machine_guid: load.state_machine_guid,
        state_guids: guids,
        extra_machines,
        mirror_states: true,
    }
}

/// ruh:1102-1118: every other machine of the item that has a starting state
/// gets a guid, then one per state (machine before its states).
pub fn plan_extra_machines(main_machine: u64, item_machines: &[ItemMachine], allocator: &mut AbilityAllocator) -> Vec<ExtraMachine> {
    let mut extra = Vec::new();
    for machine in item_machines {
        let Some(starting) = machine.starting.as_deref().filter(|state| !state.is_empty()) else { continue };
        if machine.address == main_machine {
            continue;
        }
        let guid = allocator.dynamic_guid();
        let mut state_guids = Vec::new();
        for state in &machine.state_names {
            let state_guid = allocator.dynamic_guid();
            put_guid(&mut state_guids, state, state_guid);
        }
        extra.push(ExtraMachine { name: machine.name.clone(), guid, starting_state: starting.to_string(), state_guids });
    }
    extra
}

/// ruh:1158-1165 once the slot holds the ability: extra machines first, then the
/// main machine's states except InactiveState, in SM+0x160 order.
pub fn register_ability(load: &AbilityLoad, allocator: &mut AbilityAllocator, slot: &SlotMachine, item_machines: &[ItemMachine]) -> AbilityEquippable {
    let names: Vec<&String> = slot.state_names.iter().filter(|name| name.as_str() != INACTIVE_STATE).collect();
    let extra = plan_extra_machines(slot.state_machine, item_machines, allocator);
    let mut state_guids = Vec::with_capacity(names.len());
    for name in names {
        let guid = allocator.dynamic_guid();
        put_guid(&mut state_guids, name, guid);
    }
    ability_equippable(load, &state_guids, extra)
}

/// ruh:1120-1128 for one extra machine: open it (exports), then its
/// AuthCurrentState = the starting state, exported. the caller then seeds the
/// mirror state "<key>/<name>" (auth and exported = starting, active).
pub fn extra_machine_sends(equippable: &AbilityEquippable, extra: &ExtraMachine) -> RepResult<[ChannelSend; 2]> {
    let open = machine_open_body(equippable.actor_guid, extra.guid, &extra.name)?;
    let starting_guid = extra.state_guid(&extra.starting_state).ok_or_else(|| invalid("extra machine has no guid for its starting state"))?;
    let state = auth_state_body(equippable.actor_guid, extra.guid, starting_guid, &extra.starting_state, true)?;
    Ok([ChannelSend::update(equippable.channel, open, true), ChannelSend::update(equippable.channel, state, true)])
}

// ---------------------------------------------------------------- jobs

/// ruh:2073: job k starts 0.25 s after job k - 1.
pub const JOB_STAGGER: f64 = 0.25;
/// ruh:1142: setup steps are 0.4 s apart.
pub const JOB_STEP_DELAY: f64 = 0.4;
/// ruh:1156: a registration that finds no ability retries after 0.3 s...
pub const REGISTER_RETRY_DELAY: f64 = 0.3;
/// ruh:1152: ...and gives up after this many misses.
pub const REGISTER_MAX_TRIES: u32 = 20;
/// ruh:1153, 1170: the stage of a finished (or abandoned) job.
pub const JOB_DONE: u8 = 7;
/// ruh:1138-1139: stages below this send ability_setup_bodies[stage].
pub const SETUP_STAGES: u8 = 3;

/// ruh:2071-2074
#[derive(Debug, Clone, PartialEq)]
pub struct AbilityJob {
    pub load: AbilityLoad,
    pub stage: u8,
    pub next: f64,
    pub registered_tries: u32,
}

/// the live reads registration needs (python's shared EquippableStateReader).
/// an Err is a failed read (python logs it and carries on).
pub trait AbilityOracle {
    /// read_slot_machines(reader, [slot]).get(slot)
    fn slot_machine(&mut self, slot: u32) -> Result<Option<SlotMachine>, String>;
    /// read_item_machines(reader, item)
    fn item_machines(&mut self, item: u64) -> Result<Vec<ItemMachine>, String>;
}

/// what one datagram's job pass did. for every ready ability the event
/// ability_ready gets [class, state count, [extra machine names]].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AbilityStep {
    pub sends: Vec<ChannelSend>,
    pub ready: Vec<AbilityEquippable>,
    /// event ability_register_error (python keeps the last)
    pub register_errors: Vec<String>,
    /// event extra_machines_error (python keeps the last)
    pub extra_machine_errors: Vec<String>,
    /// event ability_register_gave_up: class names
    pub gave_up: Vec<&'static str>,
}

/// per connection: the four ability jobs and their allocator (ruh:2068-2078).
#[derive(Debug, Clone, PartialEq)]
pub struct AbilityJobs {
    pub allocator: AbilityAllocator,
    pub jobs: Vec<AbilityJob>,
}

impl AbilityJobs {
    /// ruh:2069-2074: plan all four loads at once (fixed guids 100000-100014),
    /// staggered 0.25 s apart from `now`.
    pub fn new(now: f64) -> RepResult<Self> {
        let mut allocator = AbilityAllocator::default();
        let mut jobs = Vec::with_capacity(ABILITY_INDICES.len());
        for (order, &index) in ABILITY_INDICES.iter().enumerate() {
            let load = plan_ability_load(index, &mut allocator)?;
            jobs.push(AbilityJob { load, stage: 0, next: now + JOB_STAGGER * order as f64, registered_tries: 0 });
        }
        Ok(Self { allocator, jobs })
    }

    pub fn done(&self) -> bool {
        self.jobs.iter().all(|job| job.stage >= JOB_DONE)
    }

    /// ruh:2075-2078 -> advance_ability_job (ruh:1130-1170) for every due job, in order.
    pub fn step(&mut self, now: f64, oracle: &mut dyn AbilityOracle) -> RepResult<AbilityStep> {
        let mut step = AbilityStep::default();
        for job in &mut self.jobs {
            if job.stage >= JOB_DONE || now < job.next {
                continue;
            }
            if job.stage < SETUP_STAGES {
                // ability_setup_bodies(load)[stage]
                let (body, has_exports) = match job.stage {
                    0 => (ability_role_body(&job.load)?, false),
                    1 => (ability_state_machine_open_body(&job.load)?, true),
                    _ => (ability_initial_state_body(&job.load)?, true),
                };
                step.sends.push(ChannelSend::update(job.load.channel, body, has_exports));
                job.stage += 1;
                job.next = now + JOB_STEP_DELAY;
                continue;
            }
            let machine = match oracle.slot_machine(job.load.slot) {
                Ok(machine) => machine,
                Err(error) => {
                    step.register_errors.push(error);
                    None
                }
            };
            let Some(machine) = machine.filter(|machine| machine.class_name.as_deref() == Some(job.load.class_name)) else {
                job.registered_tries += 1;
                if job.registered_tries > REGISTER_MAX_TRIES {
                    job.stage = JOB_DONE;
                    step.gave_up.push(job.load.class_name);
                } else {
                    job.next = now + REGISTER_RETRY_DELAY;
                }
                continue;
            };
            let item_machines = match oracle.item_machines(machine.item) {
                Ok(machines) => machines,
                Err(error) => {
                    step.extra_machine_errors.push(error);
                    Vec::new()
                }
            };
            let equippable = register_ability(&job.load, &mut self.allocator, &machine, &item_machines);
            for extra in &equippable.extra_machines {
                step.sends.extend(extra_machine_sends(&equippable, extra)?);
            }
            step.ready.push(equippable);
            job.stage = JOB_DONE;
        }
        Ok(step)
    }
}
