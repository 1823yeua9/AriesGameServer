//! the range's live reads served by the oracle (ReadProcessMemory, read only).
//! python opened one EquippableStateReader lazily on the first authority pass
//! and retried every pass after a failure; we retry at most once a second.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use aries_oracle::Process;
use aries_oracle::client::LiveClient as Oracle;
use aries_oracle::equippable::ResourceSlot;
use aries_range::{Camera, HudWidget, InputReplication, LiveClient, LiveItemMachine, ResourceAmounts, SavedMoves, SlotMachine};
use aries_world::equippables::ResourceSubobject;
use log::{info, warn};

const REOPEN_AFTER: Duration = Duration::from_secs(1);

pub struct OracleLive {
    pid: u32,
    image_base: u64,
    oracle: Option<Oracle<Process>>,
    last_attempt: Option<Instant>,
}

impl OracleLive {
    pub fn new(pid: u32, image_base: u64) -> Self {
        Self { pid, image_base, oracle: None, last_attempt: None }
    }

    /// the reader, opened on first use (and re-opened after a failed open).
    fn oracle(&mut self) -> Result<&mut Oracle<Process>, String> {
        if self.oracle.is_none() {
            if self.last_attempt.is_some_and(|t| t.elapsed() < REOPEN_AFTER) {
                return Err("live client reader not open".into());
            }
            self.last_attempt = Some(Instant::now());
            match Oracle::open(self.pid, self.image_base) {
                Ok(oracle) => {
                    info!(target: "oracle", "reader attached to pid {}", self.pid);
                    self.oracle = Some(oracle);
                }
                Err(error) => {
                    warn!(target: "oracle", "reader open failed: {error}");
                    return Err(error.to_string());
                }
            }
        }
        Ok(self.oracle.as_mut().expect("opened above"))
    }
}

fn text(error: aries_oracle::OracleError) -> String {
    error.to_string()
}

impl LiveClient for OracleLive {
    fn attached(&mut self) -> bool {
        self.oracle().is_ok()
    }

    fn saved_moves(&mut self) -> Result<Option<SavedMoves>, String> {
        let moves = self.oracle()?.saved_moves().map_err(text)?;
        Ok(moves.map(|m| SavedMoves { newest: m.newest, count: m.count, oldest: Some(m.oldest), last_acked: m.last_acked, timestamp: m.newest_timestamp() }))
    }

    fn latest_generated_input_event(&mut self) -> Result<Option<u32>, String> {
        Ok(self.oracle()?.pending_input_bounds().map_err(text)?.map(|bounds| bounds.latest))
    }

    fn input_replication(&mut self) -> Result<Option<InputReplication>, String> {
        let replication = self.oracle()?.input_replication().map_err(text)?;
        Ok(replication.map(|r| InputReplication {
            stopped: r.stopped,
            requested_correction: r.requested_correction,
            inventory_correction: r.inventory_correction,
            current_equippable_class: r.current_equippable_class,
        }))
    }

    fn ultimate_points_requirement(&mut self) -> Result<Option<i32>, String> {
        Ok(Some(self.oracle()?.ult_required().map_err(text)?))
    }

    fn hud_widget(&mut self) -> Result<Option<HudWidget>, String> {
        let hud = self.oracle()?.hud_state().map_err(text)?;
        Ok(Some(HudWidget { collapsed: hud.collapsed, cinematic: hud.cinematic }))
    }

    fn slot_machines(&mut self, slots: &[u32]) -> Result<Vec<SlotMachine>, String> {
        let machines = self.oracle()?.slot_machines(slots).map_err(text)?;
        Ok(machines
            .into_iter()
            .map(|m| SlotMachine { slot: m.slot, item: m.item, class_name: m.class_name, state_machine: m.state_machine, predicted: m.predicted, is_current: m.is_current })
            .collect())
    }

    fn state_names(&mut self, state_machine: u64) -> Result<Vec<String>, String> {
        Ok(self.oracle()?.state_names(state_machine).map_err(text)?.into_iter().map(|(name, _)| name).collect())
    }

    fn item_machines(&mut self, item: u64) -> Result<Vec<LiveItemMachine>, String> {
        let machines = self.oracle()?.item_machines(item).map_err(text)?;
        Ok(machines
            .into_iter()
            .map(|m| LiveItemMachine { name: m.name, address: m.address, predicted: m.predicted, starting: m.starting, state_names: m.state_names })
            .collect())
    }

    fn resource_amounts(&mut self, item: u64, resources: &[ResourceSubobject]) -> Result<BTreeMap<String, ResourceAmounts>, String> {
        let slots: Vec<ResourceSlot> = resources.iter().map(|r| ResourceSlot { name: &r.name, item_offset: r.item_offset as u64, default_offset: r.default_offset as u64 }).collect();
        let amounts = self.oracle()?.resource_amounts(item, &slots).map_err(text)?;
        Ok(amounts
            .into_iter()
            .map(|(name, a)| (name, ResourceAmounts { predicted: a.predicted as i64, authoritative: a.authoritative as i64, default_amount: a.default_amount as i64 }))
            .collect())
    }

    fn firing_seed(&mut self, state_machine: u64, key: &str) -> Option<u32> {
        self.oracle().ok()?.firing_seed(state_machine, key).ok().flatten()
    }

    fn camera(&mut self) -> Result<Option<Camera>, String> {
        let camera = self.oracle()?.camera().map_err(text)?;
        Ok(camera.map(|c| Camera { location: c.location, rotation: c.rotation }))
    }

    fn client_set_hud_index(&mut self) -> Result<Option<(u32, u32)>, String> {
        let index = self.oracle()?.client_set_hud_index().map_err(text)?;
        Ok(index.map(|i| (i.field_index, i.class_max_index as u32)))
    }
}
