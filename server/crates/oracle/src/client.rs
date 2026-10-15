//! LiveClient: one read-only view of the running client that owns the process, the
//! FName pool and every cache the python kept (state names by machine address,
//! class names by object, firing rng components). the range systems call its
//! methods; spec 09. GObjects is re-decoded on every walk like the python
//! (spec 09 §3.1), so it is not cached here.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::equippable::{InputReplication, ItemMachine, ResourceMap, ResourceSlot, SlotMachine};
use crate::moves::SavedMoves;
use crate::objects::NamePool;
use crate::probes::{HudState, ProjectileSnapshot};
use crate::rpc_index::{CLIENT_SET_HUD_RVA, RpcActor, RpcIndex, resolve_rpc_index};
use crate::tags::TagNetIndex;
use crate::world::{ArrayHeader, Camera, WorldChain, at, name_key_engine};
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
    pub last_shots: HashMap<(u64, String), (u64, u32, u32)>,
}

impl ReaderCaches {
    /// address caches go stale when the client reuses memory (spec 09 Q5). python
    /// lived one 3600 s responder run; a long-lived server clears per session.
    pub fn clear(&mut self) {
        self.state_names.clear();
        self.class_names.clear();
        self.firing_rng.clear();
        self.last_shots.clear();
    }
}

/// derive_name_key (FNP:43-64): the strict header rule, the engine's chunk only, the
/// engine's object index check, then the 4-byte key validated against the full
/// "AresGameEngine".
pub fn derive_names(memory: &impl Memory) -> Result<NamePool, OracleError> {
    NamePool::from_engine(memory, name_key_engine(memory)?)
}

pub struct LiveClient<M: Memory> {
    pub memory: M,
    pub names: NamePool,
    pub caches: ReaderCaches,
    /// python caches a machine's empty TMap forever, so a machine read before the
    /// client fills it never mirrors (spec 09 Q4). true = python parity; false
    /// re-reads until it has states (the spec's safer option for a 128 Hz oracle).
    pub keep_empty_state_names: bool,
    /// how long a walked world chain is reused. None (python): every read walks
    /// GObjects -> engine -> world -> controller -> pawn again, ~2/3 of all reads
    pub chain_ttl: Option<Duration>,
    chain: Option<(Instant, Option<LiveChain>)>,
}

/// a complete chain (controller, pawn and inventory all set) and its header.
#[derive(Debug, Clone, Copy)]
pub struct LiveChain {
    pub chain: WorldChain,
    pub header: ArrayHeader,
}

impl<M: Memory> LiveClient<M> {
    /// EquippableStateReader.__init__ (RES:93-98). the responder retries this on
    /// every authority pass until it succeeds (H:1591-1598).
    pub fn new(memory: M) -> Result<Self, OracleError> {
        let names = derive_names(&memory)?;
        Ok(Self::with_names(memory, names))
    }

    pub fn with_names(memory: M, names: NamePool) -> Self {
        Self { memory, names, caches: ReaderCaches::default(), keep_empty_state_names: true, chain_ttl: None, chain: None }
    }

    /// the cached chain, re-walked once it is older than chain_ttl. None without a
    /// ttl or without a complete chain (no pawn yet, loading): callers then walk
    /// the python way, which also gives the python's exact error or None.
    pub fn live_chain(&mut self) -> Option<LiveChain> {
        let ttl = self.chain_ttl?;
        if let Some((at, chain)) = self.chain
            && at.elapsed() < ttl
        {
            return chain;
        }
        let walked = crate::world::read_world_chain_with_header(&self.memory)
            .ok()
            .filter(|(chain, _)| chain.controller != 0 && chain.pawn != 0 && chain.inventory != 0)
            .map(|(chain, header)| LiveChain { chain, header });
        self.chain = Some((Instant::now(), walked));
        walked
    }

    /// drops the cached chain (a new range entry, or a read through it failed).
    pub fn forget_chain(&mut self) {
        self.chain = None;
    }

    /// `cached` with the live chain when there is one, else `walk`. a cached read
    /// that fails may be a stale pointer: forget the chain and walk instead.
    pub(crate) fn with_chain<T>(
        &mut self,
        cached: impl FnOnce(&mut Self, LiveChain) -> Result<T, OracleError>,
        walk: impl FnOnce(&mut Self) -> Result<T, OracleError>,
    ) -> Result<T, OracleError> {
        if let Some(chain) = self.live_chain() {
            if let Ok(value) = cached(self, chain) {
                return Ok(value);
            }
            self.forget_chain();
        }
        walk(self)
    }

    pub fn image_base(&self) -> u64 {
        self.memory.image_base()
    }

    /// decode_name of a comparison index (FNP:67-71).
    pub fn name(&mut self, index: u32) -> Result<String, OracleError> {
        self.names.name(&self.memory, index)
    }

    /// `reader._names.get(sm, {})` with no read: what registration takes after its
    /// slot read (H:1159-1160 abilities, H:1301 purchases) and what the firing seed
    /// searches (H:953). None when the machine was never read.
    pub fn cached_state_names(&self, state_machine: u64) -> Option<&StateNames> {
        self.caches.state_names.get(&state_machine)
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
    pub fn camera(&mut self) -> Result<Option<Camera>, OracleError> {
        self.with_chain(|client, live| crate::world::camera_of(&client.memory, live.chain.controller), |client| crate::world::read_camera(&client.memory))
    }

    // ------------------------------------------------------------ handle-only reads

    /// R11: generated input-event bounds; read after the datagram arrived (§4.11).
    /// the chain is only pointers; the bounds themselves are always read fresh.
    pub fn pending_input_bounds(&mut self) -> Result<Option<crate::input::InputBounds>, OracleError> {
        self.with_chain(
            |client, live| crate::input::bounds_of(&client.memory, client.memory.u64(at(live.chain.pawn, crate::input::PAWN_INPUT))?),
            |client| crate::input::read_pending_input_bounds(&client.memory),
        )
    }

    /// R17: the saved-move numbers for the move ack.
    pub fn saved_moves(&mut self) -> Result<Option<crate::moves::SavedMoves>, OracleError> {
        self.with_chain(
            |client, live| {
                if !crate::world::HeaderRule::Resolve.accepts(&live.header) {
                    return Ok(None);
                }
                crate::moves::moves_of(&client.memory, live.chain.controller, live.chain.pawn)
            },
            |client| crate::moves::read_saved_moves(&client.memory),
        )
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
    pub fn ult_required(&mut self) -> Result<i32, OracleError> {
        self.with_chain(|client, live| crate::probes::ult_of(&client.memory, live.chain.pawn), |client| crate::probes::read_ult_required(&client.memory))
    }

    /// R13: bottom-HUD widget collapsed and the cinematic flag (H:1999-2004).
    pub fn hud_state(&mut self) -> Result<crate::probes::HudState, OracleError> {
        self.with_chain(|client, live| crate::probes::hud_state_of(&client.memory, live.chain.controller), |client| crate::probes::read_hud_state(&client.memory))
    }

    /// R16: gameplay-tag net indices. like the responder (H:4054-4059) it derives a
    /// fresh name key for the read instead of reusing ours, so it fails the same way.
    pub fn tag_net_indices(&self) -> Result<TagNetIndex, OracleError> {
        crate::tags::read_tag_net_indices(&self.memory)
    }
}

/// one entry of mirror_ready_equippables (H:1316-1331) and what the mirror pass
/// reads for it (H:1615-1700).
#[derive(Debug, Clone)]
pub struct WatchedEquippable {
    /// equippable.key, part of the firing rng cache key (H:949)
    pub key: String,
    pub slot: u32,
    /// the slot's item must have this class or nothing more is read for it (H:1617)
    pub class_name: String,
    /// R7, read when not empty (H:1635-1645)
    pub resources: Vec<ResourceSlot<'static>>,
    /// R8 with range_bots on: the seed, while the item is current and has resources (H:1624)
    pub firing: bool,
    /// R6 for an equippable with mirror_states and extra machines (H:1667-1676)
    pub item_machines: bool,
}

/// what a snapshot reads beyond its always-read fields (spec 09 §7.1-7.2). list an
/// equippable only while the python would read it (its mirror is due or a
/// registration stage is), or the empty-TMap race of Q4 gets more likely.
#[derive(Debug, Clone, Default)]
pub struct WatchList {
    /// in mirror order: pistol 1, melee 2, C 3, Q 4, E 5, X 9, then purchases
    pub equippables: Vec<WatchedEquippable>,
    /// R14: our projectile channels in flight
    pub projectile_channels: Vec<u32>,
}

/// one equippable's reads in a mirror pass. there is one only for an equippable whose
/// slot machine was found with the expected class (H:1616-1618).
#[derive(Debug)]
pub struct EquippableReads {
    pub key: String,
    pub slot: u32,
    /// None when not read; Some(None) where the responder got None, every failure
    /// included (H:963-964)
    pub firing_seed: Option<Option<u32>>,
    /// None when not read (python: `{}` + resource_mirror_error on failure)
    pub resources: Option<Result<ResourceMap, OracleError>>,
    /// None when not read (python: `{}` + machine_mirror_error on failure)
    pub item_machines: Option<Result<Vec<ItemMachine>, OracleError>>,
}

/// one pass of reads (spec 09 §7.2). each field fails on its own like the python's
/// separate trys, except the slot list, which fails as one unit (RES:132-164).
#[derive(Debug)]
pub struct Snapshot {
    pub taken_at: Instant,
    pub read_us: u64,
    pub chain: Result<WorldChain, OracleError>,
    /// R4 for the watched slots; with nothing watched the python skips the read (H:1603)
    pub slots: Result<Vec<SlotMachine>, OracleError>,
    /// per watched equippable in watch order, the ones whose class matched
    pub equippables: Vec<EquippableReads>,
    pub camera: Result<Option<Camera>, OracleError>,
    pub input_replication: Result<Option<InputReplication>, OracleError>,
    pub ult_required: Result<i32, OracleError>,
    pub hud: Result<HudState, OracleError>,
    pub projectiles: Vec<(u32, Result<Option<ProjectileSnapshot>, OracleError>)>,
    /// a stale value only lowers `newest`, which the strictly-increasing ack gate tolerates (§7.2)
    pub saved_moves: Result<Option<SavedMoves>, OracleError>,
}

impl<M: Memory> LiveClient<M> {
    /// the mirror pass's reads (H:1602-1700): one slot read for every watched slot,
    /// then per equippable, skipping it without a machine or with another class:
    /// firing seed, resources, item machines, in that order.
    fn mirror_reads(&mut self, watch: &WatchList) -> (Result<Vec<SlotMachine>, OracleError>, Vec<EquippableReads>) {
        if watch.equippables.is_empty() {
            return (Ok(Vec::new()), Vec::new());
        }
        let slots: Vec<u32> = watch.equippables.iter().map(|equippable| equippable.slot).collect();
        let machines = match self.slot_machines(&slots) {
            Ok(machines) => machines,
            Err(error) => return (Err(error), Vec::new()),
        };
        let mut reads = Vec::new();
        for equippable in &watch.equippables {
            let Some(machine) = machines.iter().find(|machine| machine.slot == equippable.slot) else {
                continue;
            };
            if machine.class_name != equippable.class_name {
                continue;
            }
            let has_resources = !equippable.resources.is_empty();
            let firing_seed = (equippable.firing && machine.is_current && has_resources)
                .then(|| self.firing_seed(machine.state_machine, &equippable.key).unwrap_or(None));
            let resources = has_resources.then(|| self.resource_amounts(machine.item, &equippable.resources));
            let item_machines = equippable.item_machines.then(|| self.item_machines(machine.item));
            reads.push(EquippableReads { key: equippable.key.clone(), slot: equippable.slot, firing_seed, resources, item_machines });
        }
        (Ok(machines), reads)
    }

    /// every snapshot field once, in the order one python authority pass reads them
    /// (§1: the mirror R4 -> R8 -> R7 -> R6, then R9, R10, R12, R13, R14, and R17).
    /// R11 (input bounds), R15 and R16 are on-demand queries, never snapshot fields.
    pub fn snapshot(&mut self, watch: &WatchList) -> Snapshot {
        let taken_at = Instant::now();
        let chain = match self.live_chain() {
            // the cached pointers are stable; the current equippable is not
            Some(live) => self.memory.u64(at(live.chain.inventory, 0x278)).map(|current_equippable| WorldChain { current_equippable, ..live.chain }),
            None => self.world_chain(),
        };
        let (slots, equippables) = self.mirror_reads(watch);
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
            equippables,
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

    /// R18 the python way (H:148-163): a fresh SYNCHRONIZE handle and a zero wait,
    /// not the exit code (a process that exits with 259 would look alive forever).
    pub fn alive(&self) -> bool {
        client_process_alive(self.memory.pid)
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
