//! LiveClient: one read-only view of the running client that owns the process, the
//! FName pool and every cache the python kept (state names by machine address,
//! class names by object, firing rng components). the range systems call its
//! methods; spec 09. GObjects is re-decoded on every walk like the python
//! (spec 09 §3.1), so it is not cached here.

use std::collections::HashMap;
use std::time::Instant;

use crate::equippable::{InputReplication, ItemMachine, ResourceMap, ResourceSlot, SlotMachine};
use crate::moves::SavedMoves;
use crate::objects::NamePool;
use crate::probes::{HudState, ProjectileSnapshot};
use crate::rpc_index::{CLIENT_SET_HUD_RVA, RpcActor, RpcIndex, resolve_rpc_index};
use crate::tags::TagNetIndex;
use crate::world::{Camera, HeaderRule, WorldChain, at, require_object_array};
use crate::{Memory, OracleError, Process};

/// a machine's states in live TMap order (python dict order: a repeated name keeps
/// its first position and takes the last pointer). state GUIDs are allocated in this
/// order, so it is part of the wire (spec 09 §4.4).
pub type StateNames = Vec<(String, u64)>;

/// the process-scope caches of EquippableStateReader (RES:97-98) and the responder
/// (H:944). python never invalidates them; see clear().
#[derive(Debug, Default, Clone)]
pub struct ReaderCaches {
    /// RES:97 `_names`: state machine -> its states. an empty TMap is cached too
    /// unless LiveClient::keep_empty_state_names is off (Q4).
    pub state_names: HashMap<u64, StateNames>,
    /// RES:98 `_class_names`: object -> name. items and projectile actors store their
    /// class name, machines and resource components their own name, in one map.
    pub class_names: HashMap<u64, String>,
    /// H:944 `firing_rng_components`: (state machine, equippable key) -> rng component.
    pub firing_rng: HashMap<(u64, String), u64>,
}

impl ReaderCaches {
    /// address caches go stale when the client reuses memory (spec 09 Q5). python
    /// lived one 3600 s responder run; a long-lived server clears per session.
    pub fn clear(&mut self) {
        self.state_names.clear();
        self.class_names.clear();
        self.firing_rng.clear();
    }
}

/// derive_name_key (FNP:43-64): the strict header rule, the engine's object index
/// check, then the 4-byte key validated against the full "AresGameEngine".
pub fn derive_names(memory: &impl Memory) -> Result<NamePool, OracleError> {
    let objects = require_object_array(memory, HeaderRule::NameKey)?;
    NamePool::new(memory, &objects)
}

pub struct LiveClient<M: Memory> {
    pub memory: M,
    pub names: NamePool,
    pub caches: ReaderCaches,
    /// python caches a machine's empty TMap forever, so a machine read before the
    /// client fills it never mirrors (spec 09 Q4). true = python parity; false
    /// re-reads until it has states (the spec's safer option for a 128 Hz oracle).
    pub keep_empty_state_names: bool,
}

impl<M: Memory> LiveClient<M> {
    /// EquippableStateReader.__init__ (RES:93-98). the responder retries this on
    /// every authority pass until it succeeds (H:1591-1598).
    pub fn new(memory: M) -> Result<Self, OracleError> {
        let names = derive_names(&memory)?;
        Ok(Self::with_names(memory, names))
    }

    pub fn with_names(memory: M, names: NamePool) -> Self {
        Self { memory, names, caches: ReaderCaches::default(), keep_empty_state_names: true }
    }

    pub fn image_base(&self) -> u64 {
        self.memory.image_base()
    }

    /// decode_name of a comparison index (FNP:67-71).
    pub fn name(&mut self, index: u32) -> Result<String, OracleError> {
        self.names.name(&self.memory, index)
    }

    /// `_class_names.get(object)` or decode it and remember it: `own` decodes the
    /// object's own name (+0x18), otherwise its class's (+0x10 -> +0x18).
    pub(crate) fn cached_name(&mut self, object: u64, own: bool) -> Result<String, OracleError> {
        if let Some(name) = self.caches.class_names.get(&object) {
            return Ok(name.clone());
        }
        let named = if own { object } else { self.memory.u64(at(object, 0x10))? };
        let name = self.names.name(&self.memory, self.memory.u32(at(named, 0x18))?)?;
        self.caches.class_names.insert(object, name.clone());
        Ok(name)
    }

    // ------------------------------------------------------------ world chain (spec 09 §3.3)

    pub fn world_chain(&self) -> Result<WorldChain, OracleError> {
        crate::world::read_world_chain(&self.memory)
    }

    pub fn find_world(&self) -> Result<u64, OracleError> {
        crate::world::find_world(&self.memory)
    }

    pub fn find_controller(&self) -> Result<u64, OracleError> {
        crate::world::find_controller(&self.memory)
    }

    pub fn find_pawn(&self) -> Result<u64, OracleError> {
        crate::world::find_pawn(&self.memory)
    }

    /// R9 camera POV: controller +0x528 -> +0x17C0 six doubles.
    pub fn camera(&self) -> Result<Option<Camera>, OracleError> {
        crate::world::read_camera(&self.memory)
    }

    // ------------------------------------------------------------ handle-only reads

    /// R11: generated input-event bounds; read after the datagram arrived (§4.11).
    pub fn pending_input_bounds(&self) -> Result<Option<crate::input::InputBounds>, OracleError> {
        crate::input::read_pending_input_bounds(&self.memory)
    }

    /// R17: the saved-move numbers for the move ack.
    pub fn saved_moves(&self) -> Result<Option<crate::moves::SavedMoves>, OracleError> {
        crate::moves::read_saved_moves(&self.memory)
    }

    /// R15 with any thunk: the field index of the function whose ExecFunction is `thunk`.
    pub fn resolve_rpc(&self, thunk: u64, actor: RpcActor) -> Result<Option<RpcIndex>, OracleError> {
        resolve_rpc_index(&self.memory, thunk, actor)
    }

    /// R15 production call: ClientSetHUD on the controller (H:3784-3786).
    pub fn client_set_hud_index(&self) -> Result<Option<RpcIndex>, OracleError> {
        resolve_rpc_index(&self.memory, at(self.image_base(), CLIENT_SET_HUD_RVA), RpcActor::Controller)
    }

    /// R12: the pawn's ultimate point requirement (H:1835-1837).
    pub fn ult_required(&self) -> Result<i32, OracleError> {
        crate::probes::read_ult_required(&self.memory)
    }

    /// R13: bottom-HUD widget collapsed and the cinematic flag (H:1999-2004).
    pub fn hud_state(&self) -> Result<crate::probes::HudState, OracleError> {
        crate::probes::read_hud_state(&self.memory)
    }

    /// R16: gameplay-tag net indices. the responder used a fresh reader (same key)
    /// for it (H:4054-4059); the key is per run, so ours is reused.
    pub fn tag_net_indices(&mut self) -> Result<TagNetIndex, OracleError> {
        crate::tags::predict_net_indices(&self.memory, &mut self.names)
    }
}

/// what a snapshot reads beyond its always-read fields (spec 09 §7.1-7.2). list a
/// slot only when the python would read it (its mirror is due or a registration
/// stage is), or the empty-TMap race of Q4 gets more likely.
#[derive(Debug, Clone, Default)]
pub struct WatchList {
    /// R4: slots in mirror order (pistol 1, melee 2, C 3, Q 4, E 5, X 9, then purchases)
    pub slots: Vec<u32>,
    /// R7: per slot, the resources its equippable declares
    pub resources: Vec<(u32, Vec<ResourceSlot<'static>>)>,
    /// R6: slots whose item machine array is read (equippables with extra machines)
    pub item_machine_slots: Vec<u32>,
    /// R8: (slot, equippable key), read only while that slot's item is current
    pub firing: Vec<(u32, String)>,
    /// R14: our projectile channels in flight
    pub projectile_channels: Vec<u32>,
}

/// one pass of reads (spec 09 §7.2). each field fails on its own like the python's
/// separate trys, except the slot list, which fails as one unit (RES:132-164).
#[derive(Debug)]
pub struct Snapshot {
    pub taken_at: Instant,
    pub read_us: u64,
    pub chain: Result<WorldChain, OracleError>,
    pub slots: Result<Vec<SlotMachine>, OracleError>,
    /// slot -> resource amounts (python: `{}` + resource_mirror_error on failure)
    pub resources: Vec<(u32, Result<ResourceMap, OracleError>)>,
    /// slot -> machines (python: `{}` + machine_mirror_error on failure)
    pub item_machines: Vec<(u32, Result<Vec<ItemMachine>, OracleError>)>,
    /// slot -> seed; the responder maps every failure to None (H:963-964)
    pub firing_seeds: Vec<(u32, Option<u32>)>,
    pub camera: Result<Option<Camera>, OracleError>,
    pub input_replication: Result<Option<InputReplication>, OracleError>,
    pub ult_required: Result<i32, OracleError>,
    pub hud: Result<HudState, OracleError>,
    pub projectiles: Vec<(u32, Result<Option<ProjectileSnapshot>, OracleError>)>,
    /// a stale value only lowers `newest`, which the strictly-increasing ack gate tolerates (§7.2)
    pub saved_moves: Result<Option<SavedMoves>, OracleError>,
}

impl<M: Memory> LiveClient<M> {
    /// every snapshot field once, in the order one python authority pass reads them
    /// (§1: mirror R4 -> R8 -> R7 -> R6, then R9, R10, R12, R13, R14, and R17).
    /// R11 (input bounds), R15 and R16 are on-demand queries, never snapshot fields.
    pub fn snapshot(&mut self, watch: &WatchList) -> Snapshot {
        let taken_at = Instant::now();
        let chain = self.world_chain();
        let slots = self.slot_machines(&watch.slots);
        let mut resources = Vec::new();
        let mut item_machines = Vec::new();
        let mut firing_seeds = Vec::new();
        if let Ok(machines) = &slots {
            for machine in machines {
                for (_, key) in watch.firing.iter().filter(|(slot, _)| *slot == machine.slot) {
                    if machine.is_current {
                        firing_seeds.push((machine.slot, self.firing_seed(machine.state_machine, key).unwrap_or(None)));
                    }
                }
                for (_, declared) in watch.resources.iter().filter(|(slot, _)| *slot == machine.slot) {
                    resources.push((machine.slot, self.resource_amounts(machine.item, declared)));
                }
                if watch.item_machine_slots.contains(&machine.slot) {
                    item_machines.push((machine.slot, self.item_machines(machine.item)));
                }
            }
        }
        let camera = self.camera();
        let input_replication = self.input_replication();
        let ult_required = self.ult_required();
        let hud = self.hud_state();
        let projectiles = watch.projectile_channels.iter().map(|&channel| (channel, self.projectile(channel))).collect();
        let saved_moves = self.saved_moves();
        Snapshot {
            taken_at,
            read_us: taken_at.elapsed().as_micros() as u64,
            chain,
            slots,
            resources,
            item_machines,
            firing_seeds,
            camera,
            input_replication,
            ult_required,
            hud,
            projectiles,
            saved_moves,
        }
    }
}

impl LiveClient<Process> {
    /// a read-only handle (VM_READ | QUERY_LIMITED, RRI:36) plus the name key.
    pub fn open(pid: u32, image_base: u64) -> Result<Self, OracleError> {
        Self::new(Process::open(pid, image_base)?)
    }

    pub fn pid(&self) -> u32 {
        self.memory.pid
    }

    /// R18 liveness on the reader's own handle.
    pub fn alive(&self) -> bool {
        self.memory.is_alive()
    }

    /// R1: the ares handler and both session keys (CLK:42-82). slow (a memory scan).
    pub fn handler_keys(&self, max_seconds: f64) -> Result<crate::keys::HandlerKeys, OracleError> {
        crate::keys::read_handler_keys(self.memory.pid, self.memory.image_base, max_seconds)
    }
}

/// R18 client_process_alive (H:148-163): OpenProcess(SYNCHRONIZE) fails -> dead,
/// else alive iff WaitForSingleObject(h, 0) == WAIT_TIMEOUT. no memory access.
#[cfg(windows)]
pub fn client_process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return false;
        }
        let alive = WaitForSingleObject(handle, 0) == WAIT_TIMEOUT;
        CloseHandle(handle);
        alive
    }
}

#[cfg(not(windows))]
pub fn client_process_alive(_pid: u32) -> bool {
    false
}
