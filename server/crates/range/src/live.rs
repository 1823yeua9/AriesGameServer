//! what the range needs from the running client. the oracle crate's
//! LiveClient (ReadProcessMemory now, the internal dll later) is adapted to
//! this; tests use scripted values. every read may fail: the python treated a
//! failed read as "no data" and sent nothing for that step.

use std::collections::BTreeMap;

use aries_world::equippables::ResourceSubobject;

/// read_saved_moves (read_move_number.py:45): only newest and count drive sends.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SavedMoves {
    /// the newest non-null saved move's number among the first min(count, 256) slots
    pub newest: u32,
    /// raw saved-move count (the placement-correction gate uses count > 0)
    pub count: u32,
    pub oldest: Option<u32>,
    pub last_acked: Option<u32>,
    pub timestamp: Option<f32>,
    /// saved move +592 of slot min(count, 256) - 1 (rmn:145-146); diagnostics only
    pub newest_location: Option<[f64; 3]>,
}

/// read_input_replication (read_equippable_state.py:247-268).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InputReplication {
    /// input component +0x12C: replication stopped (>256 unacked or a stale frame)
    pub stopped: bool,
    /// input component +0x130: the correction index it waits for
    pub requested_correction: u32,
    /// inventory +0x2F0
    pub inventory_correction: u32,
    /// class of the desired (+0x260) or current (+0x278) equippable
    pub current_equippable_class: Option<String>,
}

/// the bottom hud widget and load-in cinematic flag (hud refresh, python 1999-2004).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HudWidget {
    pub collapsed: bool,
    pub cinematic: bool,
}

/// one inventory slot's item and its main state machine (read_slot_machines,
/// read_equippable_state.py:132-164).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SlotMachine {
    pub slot: u32,
    pub item: u64,
    pub class_name: String,
    pub state_machine: u64,
    /// SM+0x1B0 state name
    pub predicted: Option<String>,
    /// the inventory's current equippable is this item
    pub is_current: bool,
}

/// one machine of an item's machine array (read_item_machines, :177-202).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveItemMachine {
    pub name: String,
    pub address: u64,
    pub predicted: Option<String>,
    /// SM+0x1E8; none or empty means no starting state
    pub starting: Option<String>,
    /// state names in client TMap order
    pub state_names: Vec<String>,
}

impl LiveItemMachine {
    /// what plan_extra_machines needs
    pub fn plan_view(&self) -> aries_world::loader::ItemMachine {
        aries_world::loader::ItemMachine { name: self.name.clone(), address: self.address, starting: self.starting.clone(), state_names: self.state_names.clone() }
    }
}

/// one resource component's amounts (read_resource_amounts, :212-236).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResourceAmounts {
    pub predicted: i64,
    pub authoritative: i64,
    pub default_amount: i64,
}

/// the player camera (read_camera, :66-74).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Camera {
    pub location: [f64; 3],
    /// pitch, yaw, roll in degrees
    pub rotation: [f64; 3],
}

pub trait LiveClient {
    /// python `state_reader is not None`: the equippable reads can be attempted
    fn attached(&mut self) -> bool {
        true
    }
    /// why the reader is not attached (python equippable_reader_error)
    fn attach_error(&mut self) -> Option<String> {
        None
    }
    /// a new range entry: address caches from the last one are stale
    fn new_session(&mut self) {}
    /// none when the pawn or its prediction data is not there yet
    fn saved_moves(&mut self) -> Result<Option<SavedMoves>, String>;
    /// read_pending_input_bounds: (cursor + count) of the pawn input component
    fn latest_generated_input_event(&mut self) -> Result<Option<u32>, String>;
    fn input_replication(&mut self) -> Result<Option<InputReplication>, String> {
        Ok(None)
    }
    /// i32 [pawn + 0x14F4]
    fn ultimate_points_requirement(&mut self) -> Result<Option<i32>, String> {
        Ok(None)
    }
    fn hud_widget(&mut self) -> Result<Option<HudWidget>, String> {
        Ok(None)
    }
    /// the machines of these slots; a slot with no item or machine is left out
    fn slot_machines(&mut self, _slots: &[u32]) -> Result<Vec<SlotMachine>, String> {
        Ok(Vec::new())
    }
    /// a state machine's state names in client TMap order (python reader._names[sm])
    fn state_names(&mut self, _state_machine: u64) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }
    fn item_machines(&mut self, _item: u64) -> Result<Vec<LiveItemMachine>, String> {
        Ok(Vec::new())
    }
    /// by resource name; a component that is missing or misnamed is left out
    fn resource_amounts(&mut self, _item: u64, _resources: &[ResourceSubobject]) -> Result<BTreeMap<String, ResourceAmounts>, String> {
        Ok(BTreeMap::new())
    }
    /// the firing state's networked rng seed; none on any failure (python 946-964)
    fn firing_seed(&mut self, _state_machine: u64, _key: &str) -> Option<u32> {
        None
    }
    fn camera(&mut self) -> Result<Option<Camera>, String> {
        Ok(None)
    }
    /// ClientSetHUD's (field index, class max) in the live controller class cache
    fn client_set_hud_index(&mut self) -> Result<Option<(u32, u32)>, String> {
        Ok(None)
    }
    /// our projectile channel's actor on the client (cloudburst flights)
    fn projectile(&mut self, _channel: u32) -> Result<Option<aries_world::projectiles::ProjectileSnapshot>, String> {
        Ok(None)
    }
    /// the client's gameplay tag net indices (ability tuning opens)
    fn tag_net_indices(&mut self) -> Result<TagIndices, String> {
        Err("no live client".into())
    }
}

/// predict_net_indices (gameplay_tag_net_index.py) as the ability opens use it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TagIndices {
    pub indices: std::collections::HashMap<String, u32>,
    pub true_bit_num: u32,
    pub first_bit_segment: i32,
    pub constructed: bool,
}

/// no client to read (tests, or the oracle failed to attach): every read is "no data".
pub struct NoLiveClient;

impl LiveClient for NoLiveClient {
    fn attached(&mut self) -> bool {
        false
    }

    fn saved_moves(&mut self) -> Result<Option<SavedMoves>, String> {
        Ok(None)
    }

    fn latest_generated_input_event(&mut self) -> Result<Option<u32>, String> {
        Ok(None)
    }
}
