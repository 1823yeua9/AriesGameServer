//! loading a bought gun or armor onto the client (range_gun_loader.py, spec 07 6).
//!
//! one GunLoad is an allocated identity (channel, netguids) for a store class.
//! its setup is the proven classic chain made generic, each step reliable on the
//! gun's own channel: +0 open (archetype export, the skin when one resolves, the
//! forced attachments pre-exported on ch8) and +1 Owner = pawn, +2 RemoteRole,
//! +3 the EquippableStateMachine export, +4 AuthCurrentState = InactiveState.
//! then ch8 slot Contents and an inventory correction naming it current; after
//! that the equippable mirror confirms its states and ammo.

use aries_bits::{BitWriter, Bits};
use aries_rep::block::{Target, append_rep_block, dynamic_actor_reference};
use aries_rep::bodies::channel_actor_owner_block;
use aries_rep::{RepError, RepResult};

use crate::attachments::{ATTACHMENT_ASSET_IDS_HANDLE, ATTACHMENT_ASSETS_HANDLE, GUN_OPEN_BODY_GUARD_BITS, append_guid_array_property, append_object_array_property, forced_attachments};
use crate::cosmetics::{COSMETIC_POINTER_HANDLES, COSMETIC_UUID_HANDLES, CosmeticActorLayout, RANGE_LEVEL_GUID, build_catalog_cosmetic_actor_open};
use crate::equippables::{
    EquippableActor, ExtraStateMachine, MAGAZINE_AMMO_OFFSET, PAWN_ACTOR_GUID, RESERVE_AMMO_OFFSET, ResourceSubobject, StateGuids, build_auth_state_body, build_machine_open_body, remote_role_body,
};
use crate::skins::{CosmeticKind, EquippedCdoPaths, SkinError, SkinInventory, catalog_cdo_path, resolve_equipped_cdo_paths};
use crate::store::{StoreItem, split_class_path};

/// ch 9-18 are the starting items and teams; codex reserved ch19 (range_gun_loader.py:36-38).
pub const FIRST_PURCHASE_CHANNEL: u32 = 20;
/// session netguids below 520 are taken: starting setup, cosmetics, knife and
/// ammo exports, codex's acr probe 512-515 (range_gun_loader.py:39-41).
pub const FIRST_PURCHASE_GUID: u32 = 520;
/// reliable +0..+4 on the gun channel (range_gun_loader.py:42).
pub const SETUP_BUNCH_COUNT: u32 = 5;

/// per-connection channel and netguid allocation for bought items
/// (range_gun_loader.py:45-70). the ability allocator is the same type started
/// at channel 0, guid 100000 (range_abilities.py:39-40).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PurchaseAllocator {
    pub next_channel: u32,
    pub next_guid: u32,
}

impl Default for PurchaseAllocator {
    fn default() -> Self {
        Self::new(FIRST_PURCHASE_CHANNEL, FIRST_PURCHASE_GUID)
    }
}

impl PurchaseAllocator {
    pub fn new(next_channel: u32, next_guid: u32) -> Self {
        Self { next_channel, next_guid }
    }

    pub fn channel(&mut self) -> u32 {
        let channel = self.next_channel;
        self.next_channel += 1;
        channel
    }

    /// even netguid for a spawned actor or subobject.
    pub fn dynamic_guid(&mut self) -> u32 {
        if self.next_guid & 1 != 0 {
            self.next_guid += 1;
        }
        let guid = self.next_guid;
        self.next_guid += 2;
        guid
    }

    /// two odd netguids: an object and its package.
    pub fn static_pair(&mut self) -> (u32, u32) {
        if self.next_guid & 1 == 0 {
            self.next_guid += 1;
        }
        let pair = (self.next_guid, self.next_guid + 2);
        self.next_guid += 4;
        pair
    }
}

/// a planned gun (range_gun_loader.py:73-91).
#[derive(Debug, Clone, PartialEq)]
pub struct GunLoad {
    pub item: &'static StoreItem,
    pub package: String,
    pub archetype: String,
    pub channel: u32,
    pub actor_guid: u32,
    pub archetype_guid: u32,
    pub package_guid: u32,
    pub state_machine_guid: u32,
    pub magazine_guid: u32,
    pub reserve_guid: u32,
    /// skin, level, chroma (object, package): allocated even when unskinned
    pub cosmetic_guid_pairs: [(u32, u32); 3],
    /// one (object, package) per forced attachment, in table order
    pub attachment_guid_pairs: Vec<(u32, u32)>,
    /// main-machine state guids, filled at stage 3 (InactiveState) and stage 5
    pub state_guids: StateGuids,
}

impl GunLoad {
    /// "<class>@<channel>", the mirror key (range_gun_loader.py:89-91).
    pub fn key(&self) -> String {
        format!("{}@{}", self.item.class_name, self.channel)
    }
}

/// a non-gun store item (armor): an actor in its slot, no state machine
/// (range_gun_loader.py:94-103).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemLoad {
    pub item: &'static StoreItem,
    pub package: String,
    pub archetype: String,
    pub channel: u32,
    pub actor_guid: u32,
    pub archetype_guid: u32,
    pub package_guid: u32,
}

fn invalid(message: impl Into<String>) -> RepError {
    RepError::Invalid(message.into())
}

/// python plan_item_load (range_gun_loader.py:106-113): channel, actor, then
/// the (archetype, package) pair.
pub fn plan_item_load(class_path: &str, item: &'static StoreItem, allocator: &mut PurchaseAllocator) -> RepResult<ItemLoad> {
    let path = split_class_path(class_path).map_err(|error| invalid(error.to_string()))?;
    if path.class_name != item.class_name || item.is_gun {
        return Err(invalid("store item does not match the requested non-gun class"));
    }
    let channel = allocator.channel();
    let actor_guid = allocator.dynamic_guid();
    let (archetype_guid, package_guid) = allocator.static_pair();
    Ok(ItemLoad { item, package: path.package, archetype: path.archetype, channel, actor_guid, archetype_guid, package_guid })
}

/// python item_open_body (range_gun_loader.py:116-120): the armor actor open.
pub fn item_open_body(load: &ItemLoad) -> RepResult<Bits> {
    Ok(dynamic_actor_reference((load.archetype.as_str(), load.package.as_str()), load.actor_guid, load.archetype_guid, load.package_guid, RANGE_LEVEL_GUID, None)?)
}

/// python plan_gun_load (range_gun_loader.py:123-138). allocation order:
/// channel; actor; (archetype, package); state machine; magazine; reserve; three
/// cosmetic pairs always; one pair per forced attachment. fresh pairs every
/// purchase: the pre-export and the open must agree.
pub fn plan_gun_load(class_path: &str, item: &'static StoreItem, allocator: &mut PurchaseAllocator) -> RepResult<GunLoad> {
    let path = split_class_path(class_path).map_err(|error| invalid(error.to_string()))?;
    if path.class_name != item.class_name || !item.is_gun {
        return Err(invalid("store item does not match the requested gun class"));
    }
    let channel = allocator.channel();
    let actor_guid = allocator.dynamic_guid();
    let (archetype_guid, package_guid) = allocator.static_pair();
    let state_machine_guid = allocator.dynamic_guid();
    let magazine_guid = allocator.dynamic_guid();
    let reserve_guid = allocator.dynamic_guid();
    let cosmetic_guid_pairs = [allocator.static_pair(), allocator.static_pair(), allocator.static_pair()];
    let attachment_guid_pairs = forced_attachments(item.class_name).iter().map(|_| allocator.static_pair()).collect();
    Ok(GunLoad {
        item,
        package: path.package,
        archetype: path.archetype,
        channel,
        actor_guid,
        archetype_guid,
        package_guid,
        state_machine_guid,
        magazine_guid,
        reserve_guid,
        cosmetic_guid_pairs,
        attachment_guid_pairs,
        state_guids: StateGuids::new(),
    })
}

/// python gun_attachments (range_gun_loader.py:141-147): (object netguid, uuid
/// words) per forced attachment, in send order.
pub fn gun_attachments(load: &GunLoad) -> RepResult<Vec<(u32, [u32; 4])>> {
    let assets = forced_attachments(load.item.class_name);
    if assets.len() != load.attachment_guid_pairs.len() {
        return Err(invalid("gun load was planned without its attachment NetGUIDs"));
    }
    Ok(load.attachment_guid_pairs.iter().zip(assets).map(|(&(object_guid, _), asset)| (object_guid, asset.uuid_words)).collect())
}

/// the measured AAresEquippable cosmetic layout for this gun (range_gun_loader.py:157-164).
pub fn gun_cosmetic_layout(load: &GunLoad) -> CosmeticActorLayout {
    CosmeticActorLayout {
        weapon_id: load.item.weapon_id.to_string(),
        channel_index: load.channel,
        actor_net_guid: load.actor_guid,
        archetype_name: load.archetype.clone(),
        archetype_package: load.package.clone(),
        archetype_net_guid: load.archetype_guid,
        package_net_guid: load.package_guid,
        level_net_guid: RANGE_LEVEL_GUID,
        pointer_handles: COSMETIC_POINTER_HANDLES,
        uuid_handles: COSMETIC_UUID_HANDLES,
        asset_guid_pairs: load.cosmetic_guid_pairs,
    }
}

/// python gun_open_body (range_gun_loader.py:150-186): +0, the actor open with
/// the skin (when given) and the gun's attachments. the attachment cdos must
/// already be exported.
pub fn gun_open_body(load: &GunLoad, cosmetic_cdo_paths: Option<&EquippedCdoPaths>) -> RepResult<Bits> {
    let attachments = gun_attachments(load)?;
    if let Some(paths) = cosmetic_cdo_paths {
        return build_catalog_cosmetic_actor_open(paths, &gun_cosmetic_layout(load), &attachments);
    }
    let body = dynamic_actor_reference((load.archetype.as_str(), load.package.as_str()), load.actor_guid, load.archetype_guid, load.package_guid, RANGE_LEVEL_GUID, None)?;
    if attachments.is_empty() {
        return Ok(body);
    }
    let mut rep = BitWriter::new();
    rep.append_bits(0, 1);
    let guids: Vec<u32> = attachments.iter().map(|&(guid, _)| guid).collect();
    let words: Vec<[u32; 4]> = attachments.iter().map(|&(_, words)| words).collect();
    append_object_array_property(&mut rep, ATTACHMENT_ASSETS_HANDLE, &guids);
    append_guid_array_property(&mut rep, ATTACHMENT_ASSET_IDS_HANDLE, &words);
    rep.append_packed_uint(0);
    let mut w = BitWriter::new();
    w.append_bits_value(&body);
    append_rep_block(&mut w, Target::Actor, &rep.finish()?, load.actor_guid)?;
    let body = w.finish()?;
    if body.bit_count > GUN_OPEN_BODY_GUARD_BITS {
        return Err(invalid("gun actor-open bunch exceeds protocol size limit"));
    }
    Ok(body)
}

/// python gun_owner_body (range_gun_loader.py:189-191): +1, AActor.Owner = the pawn.
pub fn gun_owner_body(load: &GunLoad) -> RepResult<Bits> {
    channel_actor_owner_block(load.actor_guid, PAWN_ACTOR_GUID)
}

/// python gun_role_body (range_gun_loader.py:194-203): +2, RemoteRole = AutonomousProxy.
pub fn gun_role_body(load: &GunLoad) -> RepResult<Bits> {
    remote_role_body(load.actor_guid)
}

/// python gun_state_machine_open_body (range_gun_loader.py:206-221): +3, the
/// EquippableStateMachine default subobject with an empty layout.
pub fn gun_state_machine_open_body(load: &GunLoad) -> RepResult<Bits> {
    build_machine_open_body(load.actor_guid, load.state_machine_guid, "EquippableStateMachine")
}

/// python gun_equippable (range_gun_loader.py:224-240): the mirror's view of the
/// gun. InactiveState is allocated first (stage 3); at stage 5 every live state
/// name not yet known gets the next dynamic guid, in live TMap order.
pub fn gun_equippable<S: AsRef<str>>(load: &mut GunLoad, mut allocator: Option<&mut PurchaseAllocator>, state_names: &[S], extra_machines: Vec<ExtraStateMachine>) -> RepResult<EquippableActor> {
    if !load.state_guids.contains("InactiveState") {
        let Some(allocator) = allocator.as_deref_mut() else {
            return Err(invalid("an allocator is needed to name the first state"));
        };
        load.state_guids.insert("InactiveState", allocator.dynamic_guid());
    }
    for name in state_names {
        let name = name.as_ref();
        if !load.state_guids.contains(name)
            && let Some(allocator) = allocator.as_deref_mut()
        {
            load.state_guids.insert(name, allocator.dynamic_guid());
        }
    }
    Ok(EquippableActor::new(load.key(), load.item.class_name, load.channel, load.actor_guid, load.item.slot, load.state_machine_guid, load.state_guids.clone())?
        .with_resources(vec![
            ResourceSubobject::ammo("MagazineAmmo", load.magazine_guid, MAGAZINE_AMMO_OFFSET),
            ResourceSubobject::ammo("ReserveAmmo", load.reserve_guid, RESERVE_AMMO_OFFSET),
        ])
        .with_extra_machines(extra_machines))
}

/// python gun_initial_state_body (range_gun_loader.py:243-245): +4,
/// AuthCurrentState = InactiveState (exported); allocates its guid.
pub fn gun_initial_state_body(load: &mut GunLoad, allocator: &mut PurchaseAllocator) -> RepResult<Bits> {
    let equippable = gun_equippable::<&str>(load, Some(allocator), &[], Vec::new())?;
    build_auth_state_body(&equippable, "InactiveState", true)
}

/// one machine of a live item's machine array (item+0xE30), as the oracle reads
/// it (read_equippable_state.read_item_machines).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemMachine {
    pub name: String,
    pub address: u64,
    /// SM+0x1E8; empty or None means no starting state
    pub starting: Option<String>,
    /// SM+0x160 names in live TMap order
    pub state_names: Vec<String>,
}

/// stage-5 plan_extra_machines (range_udp_handshake.py:1102-1118): every machine
/// other than the main one that has a starting state gets the next dynamic
/// guid, then one per state name in TMap order (a repeated name keeps its first
/// place and the last guid, like the python dict comprehension).
pub fn plan_extra_machines(allocator: &mut PurchaseAllocator, item_machines: &[ItemMachine], main_state_machine: u64) -> Vec<ExtraStateMachine> {
    item_machines
        .iter()
        .filter(|machine| machine.address != main_state_machine)
        .filter_map(|machine| {
            let starting = machine.starting.as_deref().filter(|starting| !starting.is_empty())?;
            let guid = allocator.dynamic_guid();
            let state_guids = machine.state_names.iter().map(|name| (name.clone(), allocator.dynamic_guid())).collect();
            Some(ExtraStateMachine { name: machine.name.clone(), guid, starting_state: starting.to_string(), state_guids })
        })
        .collect()
}

/// python select_saved_cdo_paths (range_gun_loader.py:276-285): the owned and
/// equipped skin, None when unskinned or any part is not packaged.
pub fn select_saved_cdo_paths(inventory: &SkinInventory, weapon_id: &str) -> Result<Option<EquippedCdoPaths>, SkinError> {
    Ok(resolve_equipped_cdo_paths(inventory, weapon_id)?.filter(EquippedCdoPaths::all_packaged))
}

/// python standard_cdo_paths (range_gun_loader.py:288-305): the weapon's first
/// "_Standard" catalog entry in file order with its default level and chroma.
/// unskinned guns have no first-person cosmetic mesh, so sights would have
/// nothing to attach to.
pub fn standard_cdo_paths(inventory: &SkinInventory, weapon_id: &str) -> Result<Option<EquippedCdoPaths>, SkinError> {
    let weapon_id = weapon_id.to_lowercase();
    let Some(entry) = inventory.catalog.entries().iter().find(|entry| entry.weapon_id.to_lowercase() == weapon_id && entry.name.ends_with("_Standard")) else {
        return Ok(None);
    };
    let paths = EquippedCdoPaths {
        skin: catalog_cdo_path(entry, CosmeticKind::Skin, &entry.skin_id)?,
        level: catalog_cdo_path(entry, CosmeticKind::Level, &entry.skin_level_id)?,
        chroma: catalog_cdo_path(entry, CosmeticKind::Chroma, &entry.chroma_id)?,
        weapon_id,
    };
    Ok(Some(paths).filter(EquippedCdoPaths::all_packaged))
}

/// the stage-0b open a bought gun gets (range_udp_handshake.py:1226-1252).
#[derive(Debug, Clone, PartialEq)]
pub struct GunOpen {
    pub body: Bits,
    /// the cosmetic that went out (event purchase_opened's skin id), None = unskinned
    pub cosmetics: Option<EquippedCdoPaths>,
    /// every selection or build error on the way (event purchase_cosmetic_error, last wins)
    pub errors: Vec<String>,
}

/// python's candidate loop (range_udp_handshake.py:1228-1248): the saved skin,
/// else the Standard skin, else unskinned; the first that builds wins. pass the
/// two selections (select_saved_cdo_paths, standard_cdo_paths) as they came out.
pub fn gun_open_with_fallback(load: &GunLoad, saved: Result<Option<EquippedCdoPaths>, SkinError>, standard: Result<Option<EquippedCdoPaths>, SkinError>) -> RepResult<GunOpen> {
    let mut errors = Vec::new();
    let mut candidates = Vec::new();
    for found in [saved, standard] {
        match found {
            Ok(Some(paths)) => candidates.push(paths),
            Ok(None) => {}
            Err(error) => errors.push(error.to_string()),
        }
    }
    for paths in candidates {
        match gun_open_body(load, Some(&paths)) {
            Ok(body) => return Ok(GunOpen { body, cosmetics: Some(paths), errors }),
            Err(error) => errors.push(error.to_string()),
        }
    }
    let body = gun_open_body(load, None)?;
    Ok(GunOpen { body, cosmetics: None, errors })
}
