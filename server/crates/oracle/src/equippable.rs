//! equippable state machines, slot machines, item machines, resources, input
//! replication, firing seed (spec 09 R4-R8, R10). read_equippable_state.py and
//! the responder's read_firing_seed (H:946-964).

use std::collections::BTreeMap;

use crate::client::{LiveClient, StateNames};
use crate::world::{at, find_pawn};
use crate::{Memory, OracleError};

/// UStateMachineComponent fields (RES:3-7, 78-79, 200).
const SM_STATES: u64 = 0x160;
const SM_STATE_COUNT: u64 = 0x168;
const SM_PREDICTED: u64 = 0x1B0;
const SM_AUTHORITATIVE: u64 = 0x1C8;
const SM_STARTING: u64 = 0x1E8;
/// TMap element stride, the state object first (RES:5-6, 84).
const STATE_STRIDE: u64 = 24;
/// pawn +0xBF8 inventory, inventory +0x278 current, +0x260 desired, +0x1B8 slots (RES:137-143, 256-259)
const PAWN_INVENTORY: u64 = 0xBF8;
const PAWN_INPUT: u64 = 0xB90;
const INVENTORY_CURRENT: u64 = 0x278;
const INVENTORY_DESIRED: u64 = 0x260;
const INVENTORY_SLOTS: u64 = 0x1B8;
const INVENTORY_CORRECTION: u64 = 0x2F0;
const SLOT_CONTENTS: u64 = 0x38;
/// item +0xE00 EquippableStateMachine, +0xE30/+0xE38 machine array (RES:153, 180-181)
const ITEM_STATE_MACHINE: u64 = 0xE00;
const ITEM_MACHINES: u64 = 0xE30;
const ITEM_MACHINE_COUNT: u64 = 0xE38;
/// FiringState +0x4A0 rng component, rng +0xEC networked seed (H:947, 958, 962)
const FIRING_RNG: u64 = 0x4A0;
const RNG_SEED: u64 = 0xEC;

/// one inventory slot's item and its main machine (RES:121-129).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotMachine {
    pub slot: u32,
    pub item: u64,
    pub class_name: String,
    pub state_machine: u64,
    pub predicted: Option<String>,
    pub authoritative: Option<String>,
    pub is_current: bool,
}

/// one machine of an item's machine array (RES:167-174).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemMachine {
    /// default-subobject name, e.g. "ZoomStateMachine"
    pub name: String,
    pub address: u64,
    pub predicted: Option<String>,
    pub authoritative: Option<String>,
    pub starting: Option<String>,
    pub state_names: Vec<String>,
}

/// a named resource component on an item (range_equippables.ResourceSubobject, RE:22-37).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceSlot<'a> {
    pub name: &'a str,
    pub item_offset: u64,
    /// 0x12C UAmmoComponent.Ammo, 0x188 MaxCharges for ability charges (RE:36)
    pub default_offset: u64,
}

/// BasePistol: +0x1208 MagazineAmmo, +0x1200 ReserveAmmo (RE:326-327); bought guns use the same (range_gun_loader.py:238-239).
pub const GUN_RESOURCES: [ResourceSlot<'static>; 2] = [
    ResourceSlot { name: "MagazineAmmo", item_offset: 0x1208, default_offset: 0x12C },
    ResourceSlot { name: "ReserveAmmo", item_offset: 0x1200, default_offset: 0x12C },
];
/// Cloudburst C (RE:272)
pub const SMOKE_RESOURCES: [ResourceSlot<'static>; 1] = [ResourceSlot { name: "EquipmentCharge", item_offset: 0x1240, default_offset: 0x188 }];
/// Updraft Q (RE:273)
pub const UPDRAFT_RESOURCES: [ResourceSlot<'static>; 1] = [ResourceSlot { name: "EquipmentCharge", item_offset: 0x12A0, default_offset: 0x188 }];
/// Tailwind E (RE:274)
pub const TAILWIND_RESOURCES: [ResourceSlot<'static>; 1] = [ResourceSlot { name: "SignatureAbilityResource", item_offset: 0x1290, default_offset: 0x188 }];

/// RES:205-209
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceAmounts {
    /// UResourceComponent.PredictedResourceAmount +0x10C
    pub predicted: i32,
    /// AuthResourceAmount +0x110
    pub authoritative: i32,
    /// the class default fill at the resource's default offset
    pub default_amount: i32,
}

/// read_resource_amounts' result: resource name -> amounts
pub type ResourceMap = BTreeMap<String, ResourceAmounts>;

/// RES:239-244
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputReplication {
    /// input component +0x12C
    pub stopped: bool,
    /// input component +0x130, the correction index recorded when it stopped
    pub requested_correction: u32,
    /// inventory +0x2F0 CorrectionIndex
    pub inventory_correction: u32,
    /// class of the desired (+0x260) or else current (+0x278) equippable
    pub current_equippable_class: Option<String>,
}

/// `{address: name for name, address in states.items()}.get(pointer)`: the last
/// name holding that pointer.
pub fn state_name_of(states: &StateNames, pointer: u64) -> Option<String> {
    states.iter().rev().find(|(_, address)| *address == pointer).map(|(name, _)| name.clone())
}

/// python `dict[key] = value`: replace in place or append.
fn dict_set<V>(entries: &mut Vec<(String, V)>, key: String, value: V) {
    match entries.iter_mut().find(|(k, _)| *k == key) {
        Some(entry) => entry.1 = value,
        None => entries.push((key, value)),
    }
}

impl<M: Memory> LiveClient<M> {
    /// R5 state_names (RES:77-87), uncached: TMap index order, nulls skipped.
    pub fn read_state_names(&mut self, state_machine: u64) -> Result<StateNames, OracleError> {
        let data = self.memory.u64(at(state_machine, SM_STATES))?;
        let count = self.memory.u32(at(state_machine, SM_STATE_COUNT))?;
        let mut names = StateNames::new();
        if data == 0 || !(1..=64).contains(&count) {
            return Ok(names);
        }
        for index in 0..count as u64 {
            let state = self.memory.u64(at(data, STATE_STRIDE * index))?;
            if state != 0 {
                let name = self.names.name(&self.memory, self.memory.u32(at(state, 0x18))?)?;
                dict_set(&mut names, name, state);
            }
        }
        Ok(names)
    }

    /// R5 through the `_names` cache (RES:110-111, 156-157). the first read is kept
    /// forever, an empty one too unless keep_empty_state_names is off (spec 09 Q4).
    pub fn state_names(&mut self, state_machine: u64) -> Result<StateNames, OracleError> {
        if let Some(names) = self.caches.state_names.get(&state_machine) {
            return Ok(names.clone());
        }
        let names = self.read_state_names(state_machine)?;
        if !names.is_empty() || self.keep_empty_state_names {
            self.caches.state_names.insert(state_machine, names.clone());
        }
        Ok(names)
    }

    fn state_at(&self, states: &StateNames, address: u64) -> Result<Option<String>, OracleError> {
        Ok(state_name_of(states, self.memory.u64(address)?))
    }

    /// R4 read_slot_machines (RES:132-164): result in first-seen slot order. any
    /// failure fails every slot of the call, like the python.
    pub fn slot_machines(&mut self, slots: &[u32]) -> Result<Vec<SlotMachine>, OracleError> {
        let pawn = find_pawn(&self.memory)?;
        let inventory = if pawn == 0 { 0 } else { self.memory.u64(at(pawn, PAWN_INVENTORY))? };
        let mut result: Vec<SlotMachine> = Vec::new();
        if inventory == 0 {
            return Ok(result);
        }
        let current = self.memory.u64(at(inventory, INVENTORY_CURRENT))?;
        for &slot in slots {
            let item_slot = self.memory.u64(at(inventory, INVENTORY_SLOTS + 8 * slot as u64))?;
            let item = if item_slot == 0 { 0 } else { self.memory.u64(at(item_slot, SLOT_CONTENTS))? };
            if item == 0 {
                continue;
            }
            let class_name = self.cached_name(item, false)?;
            let state_machine = self.memory.u64(at(item, ITEM_STATE_MACHINE))?;
            if state_machine == 0 {
                continue;
            }
            let states = self.state_names(state_machine)?;
            let machine = SlotMachine {
                slot,
                item,
                class_name,
                state_machine,
                predicted: self.state_at(&states, at(state_machine, SM_PREDICTED))?,
                authoritative: self.state_at(&states, at(state_machine, SM_AUTHORITATIVE))?,
                is_current: item == current,
            };
            match result.iter_mut().find(|m| m.slot == slot) {
                Some(existing) => *existing = machine,
                None => result.push(machine),
            }
        }
        Ok(result)
    }

    /// R6 read_item_machines (RES:177-202): every machine in item +0xE30 (1..=8).
    pub fn item_machines(&mut self, item: u64) -> Result<Vec<ItemMachine>, OracleError> {
        let array = self.memory.u64(at(item, ITEM_MACHINES))?;
        let count = self.memory.u32(at(item, ITEM_MACHINE_COUNT))?;
        let mut machines = Vec::new();
        if array == 0 || !(1..=8).contains(&count) {
            return Ok(machines);
        }
        for index in 0..count as u64 {
            let machine = self.memory.u64(at(array, 8 * index))?;
            if machine == 0 {
                continue;
            }
            let name = self.cached_name(machine, true)?;
            let states = self.state_names(machine)?;
            machines.push(ItemMachine {
                name,
                address: machine,
                predicted: self.state_at(&states, at(machine, SM_PREDICTED))?,
                authoritative: self.state_at(&states, at(machine, SM_AUTHORITATIVE))?,
                starting: self.state_at(&states, at(machine, SM_STARTING))?,
                state_names: states.into_iter().map(|(name, _)| name).collect(),
            });
        }
        Ok(machines)
    }

    /// R7 read_resource_amounts (RES:212-236): only components whose own name matches.
    pub fn resource_amounts(&mut self, item: u64, resources: &[ResourceSlot<'_>]) -> Result<ResourceMap, OracleError> {
        let mut result = BTreeMap::new();
        for resource in resources {
            let component = self.memory.u64(at(item, resource.item_offset))?;
            if component == 0 {
                continue;
            }
            if self.cached_name(component, true)? != resource.name {
                continue;
            }
            let pair: [u8; 8] = self.memory.read(at(component, 0x10C))?;
            let default_amount = self.memory.i32(at(component, resource.default_offset))?;
            result.insert(
                resource.name.to_string(),
                ResourceAmounts {
                    predicted: i32::from_le_bytes(pair[0..4].try_into().unwrap()),
                    authoritative: i32::from_le_bytes(pair[4..8].try_into().unwrap()),
                    default_amount,
                },
            );
        }
        Ok(result)
    }

    /// R8 read_firing_seed (H:946-964). uses only the cached state names of the
    /// machine; the rng pointer is cached per (machine, key) before the seed read.
    /// Ok(None) where the python returns None; the python also maps Err to None.
    pub fn firing_seed(&mut self, state_machine: u64, equippable_key: &str) -> Result<Option<u32>, OracleError> {
        let key = (state_machine, equippable_key.to_string());
        let rng = match self.caches.firing_rng.get(&key) {
            Some(&rng) => rng,
            None => {
                let states = self.caches.state_names.get(&state_machine);
                let firing = states.and_then(|states| {
                    let exact = states.iter().find(|(name, _)| name == "FiringState").map(|(_, address)| *address);
                    exact.or_else(|| states.iter().find(|(name, _)| name.starts_with("Firing")).map(|(_, address)| *address))
                });
                let Some(firing) = firing.filter(|&address| address != 0) else {
                    return Ok(None);
                };
                let rng = self.memory.u64(at(firing, FIRING_RNG))?;
                if rng == 0 {
                    return Ok(None);
                }
                self.caches.firing_rng.insert(key, rng);
                rng
            }
        };
        Ok(Some(self.memory.u32(at(rng, RNG_SEED))?))
    }

    /// R10 read_input_replication (RES:247-268). the class name is not cached.
    pub fn input_replication(&mut self) -> Result<Option<InputReplication>, OracleError> {
        let pawn = find_pawn(&self.memory)?;
        let component = if pawn == 0 { 0 } else { self.memory.u64(at(pawn, PAWN_INPUT))? };
        let inventory = if pawn == 0 { 0 } else { self.memory.u64(at(pawn, PAWN_INVENTORY))? };
        if component == 0 || inventory == 0 {
            return Ok(None);
        }
        // a resume correction force-sets the desired equippable, so it goes first (RES:256-259)
        let mut equippable = self.memory.u64(at(inventory, INVENTORY_DESIRED))?;
        if equippable == 0 {
            equippable = self.memory.u64(at(inventory, INVENTORY_CURRENT))?;
        }
        let current_equippable_class = if equippable == 0 {
            None
        } else {
            let class = self.memory.u64(at(equippable, 0x10))?;
            Some(self.names.name(&self.memory, self.memory.u32(at(class, 0x18))?)?)
        };
        Ok(Some(InputReplication {
            stopped: self.memory.u8(at(component, 0x12C))? != 0,
            requested_correction: self.memory.u32(at(component, 0x130))?,
            inventory_correction: self.memory.u32(at(inventory, INVENTORY_CORRECTION))?,
            current_equippable_class,
        }))
    }
}
