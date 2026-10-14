//! store purchases (python 1847-1907, 1172-1314): a ServerPurchase rpc on the
//! controller channel plans a gun or armor load, then a timed job opens it on
//! its own channel, slots it, makes it current and hands it to the mirror.
//! docs/port/05_authority_loop.md 6.9.

use std::collections::{HashMap, HashSet};

use aries_bits::Bits;
use aries_rep::decode::{SERVER_PURCHASE_FIELD, decode_purchase_requests, take_new_purchases};
use aries_world::attachments::{attachment_pre_export_body, forced_attachments};
use aries_world::equippables::{ARMOR_SHIELD_FALLBACK, PAWN_ACTOR_GUID, armor_shield_value, build_shield_section_body};
use aries_world::loader::{
    GunLoad, ItemLoad, PurchaseAllocator, gun_equippable, gun_initial_state_body, gun_open_with_fallback, gun_owner_body, gun_role_body, gun_state_machine_open_body, item_open_body,
    plan_extra_machines, plan_gun_load, plan_item_load, select_saved_cdo_paths, standard_cdo_paths,
};
use aries_world::store::{resolve_class_path, store_item_for_path};
use serde_json::{Value, json};

use crate::authority::Env;
use crate::mirror::{extra_names, send_extra_machines, starting};
use crate::send::Reply;

pub const STAGE_DONE: u8 = 7;
/// python job["next"] spacings.
pub const STAGE_DELAY: f64 = 0.5;
pub const ATTACHMENT_DELAY: f64 = 0.2;
pub const REGISTER_RETRY: f64 = 0.3;
/// python registered_tries > 20 gives up.
pub const REGISTER_TRIES: u32 = 20;

#[derive(Debug, Clone)]
pub enum JobLoad {
    Gun(GunLoad),
    Armor(ItemLoad),
}

/// python purchase_jobs entries ({"load", "stage", "next", "registered_tries", ...}).
#[derive(Debug, Clone)]
pub struct PurchaseJob {
    pub load: JobLoad,
    pub stage: u8,
    pub next: f64,
    pub registered_tries: u32,
    pub attachments_exported: bool,
}

/// what a slot holds now (python purchased_slot_items values).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotItem {
    pub channel: u32,
    pub key: String,
    pub actor_guid: u32,
    pub class_name: String,
}

/// per-connection purchase state (python dicts keyed by source).
#[derive(Debug, Default, Clone)]
pub struct PurchaseBook {
    /// python purchase_sequences_seen (156/157/161 share it)
    pub sequences_seen: HashSet<u32>,
    pub allocator: Option<PurchaseAllocator>,
    /// never pruned: done jobs stay at stage 7 (quirk 11)
    pub jobs: Vec<PurchaseJob>,
    /// package guid -> package path, resolves "<guid:N>.Class" repeat purchases
    pub package_guids: HashMap<u32, String>,
    /// class -> last full path
    pub seen_paths: HashMap<String, String>,
    pub slot_items: HashMap<u32, SlotItem>,
    pub shield_section_exported: bool,
}

/// python 1851-1907. the gate (starting equippables done) is the caller's.
pub fn run_purchases(reply: &mut Reply, env: &mut Env, decoded: &Bits) {
    let requests = decode_purchase_requests(decoded);
    for request in take_new_purchases(requests, &mut reply.conn.authority.purchases.sequences_seen) {
        reply.event.push("received_purchases", json!([request.field, request.item_path, request.target_slot, request.purchasable_type]));
        let Some(item_path) = request.item_path.as_deref().filter(|p| !p.is_empty()) else {
            continue;
        };
        if request.field != SERVER_PURCHASE_FIELD {
            continue;
        }
        plan_purchase(reply, item_path);
    }
    // python iterates the live list; jobs are only appended above, never here
    let mut jobs = std::mem::take(&mut reply.conn.authority.purchases.jobs);
    for job in jobs.iter_mut() {
        if job.stage >= STAGE_DONE || reply.now < job.next {
            continue;
        }
        if let Err(error) = advance_job(reply, env, job) {
            reply.event.set("purchase_error", error);
            job.stage = STAGE_DONE;
        }
    }
    reply.conn.authority.purchases.jobs = jobs;
}

fn plan_purchase(reply: &mut Reply, item_path: &str) {
    let book = &mut reply.conn.authority.purchases;
    // repeat purchases name the package by the guid we exported ("<guid:525>.AssaultRifle_AK_C")
    let class_path = resolve_class_path(Some(item_path), &book.package_guids, &book.seen_paths);
    let item = class_path.as_deref().and_then(|path| store_item_for_path(path).ok().flatten());
    let (Some(class_path), Some(item)) = (class_path, item) else {
        reply.event.set("purchase_unsupported", item_path);
        return;
    };
    if class_path != item_path {
        reply.event.set("purchase_resolved_path", json!([item_path, class_path]));
    }
    book.seen_paths.insert(item.class_name.to_string(), class_path.clone());
    let allocator = book.allocator.get_or_insert_with(PurchaseAllocator::default);
    let planned = if item.is_gun {
        plan_gun_load(&class_path, item, allocator).map(|load| (load.package_guid, load.package.clone(), load.channel, load.actor_guid, JobLoad::Gun(load)))
    } else {
        plan_item_load(&class_path, item, allocator).map(|load| (load.package_guid, load.package.clone(), load.channel, load.actor_guid, JobLoad::Armor(load)))
    };
    match planned {
        Ok((package_guid, package, channel, actor_guid, load)) => {
            book.package_guids.insert(package_guid, package);
            book.jobs.push(PurchaseJob { load, stage: 0, next: reply.now, registered_tries: 0, attachments_exported: false });
            reply.event.set("purchase_planned", json!([item.display_name, channel, actor_guid]));
        }
        Err(error) => reply.event.set("purchase_error", error.to_string()),
    }
}

/// python advance_purchase_job (1172-1314): one timed step.
fn advance_job(reply: &mut Reply, env: &mut Env, job: &mut PurchaseJob) -> Result<(), String> {
    match &mut job.load {
        JobLoad::Armor(load) => {
            let load = load.clone();
            advance_armor(reply, job, &load)
        }
        JobLoad::Gun(_) => advance_gun(reply, env, job),
    }
}

fn send(reply: &mut Reply, channel: u32, body: aries_rep::RepResult<Bits>, exports: bool, open: bool) -> Result<usize, String> {
    let body = body.map_err(|e| e.to_string())?;
    reply.send_reliable(channel, &body, exports, open, false).map_err(|e| e.to_string())
}

/// armor: the item in ItemSlots[11] (the shop shows it owned) and the pawn's
/// ShieldDamageSection life (the hud shield). no correction, no replacement.
fn advance_armor(reply: &mut Reply, job: &mut PurchaseJob, load: &ItemLoad) -> Result<(), String> {
    match job.stage {
        0 => {
            reply.register_dynamic_channel(load.channel);
            send(reply, load.channel, item_open_body(load), true, true)?;
            send(reply, load.channel, aries_rep::bodies::channel_actor_owner_block(load.actor_guid, PAWN_ACTOR_GUID), false, false)?;
        }
        1 => {
            send(reply, 8, aries_rep::bodies::item_slot_contents_block(load.item.slot, load.actor_guid, PAWN_ACTOR_GUID), false, false)?;
        }
        _ => {
            let shield = armor_shield_value(load.item.class_name).unwrap_or(ARMOR_SHIELD_FALLBACK);
            let export = !reply.conn.authority.purchases.shield_section_exported;
            send(reply, 8, build_shield_section_body(shield, shield, export), export, false)?;
            reply.conn.authority.purchases.shield_section_exported = true;
            reply.event.set("purchase_shield", json!([load.item.display_name, shield]));
            job.stage = STAGE_DONE;
            return Ok(());
        }
    }
    job.stage += 1;
    job.next = reply.now + STAGE_DELAY;
    Ok(())
}

fn advance_gun(reply: &mut Reply, env: &mut Env, job: &mut PurchaseJob) -> Result<(), String> {
    let JobLoad::Gun(load) = &mut job.load else {
        unreachable!("gun job");
    };
    let now = reply.now;
    match job.stage {
        0 => {
            let attachments = forced_attachments(load.item.class_name);
            if !attachments.is_empty() && !job.attachments_exported {
                // the open references the sight/silencer cdos; an unresolved one crashes BeginPlay
                let body = attachment_pre_export_body(attachments, &load.attachment_guid_pairs).map_err(|e| e.to_string())?;
                let bits = body.bit_count;
                reply.send_reliable(8, &body, true, false, false).map_err(|e| e.to_string())?;
                job.attachments_exported = true;
                let mut exported: Vec<Value> = attachments.iter().map(|a| json!(a.cdo_name)).collect();
                exported.push(json!(bits));
                reply.event.set("purchase_attachments_exported", exported);
                job.next = now + ATTACHMENT_DELAY;
                return Ok(());
            }
            reply.register_dynamic_channel(load.channel);
            // saved skin, else the standard skin (sights need the first-person mesh), else none
            let inventory = env.skins.get();
            let saved = inventory.as_ref().map_err(Clone::clone).and_then(|inventory| select_saved_cdo_paths(inventory, load.item.weapon_id));
            let standard = inventory.as_ref().map_err(Clone::clone).and_then(|inventory| standard_cdo_paths(inventory, load.item.weapon_id));
            let open = gun_open_with_fallback(load, saved, standard).map_err(|e| e.to_string())?;
            if let Some(error) = open.errors.last() {
                reply.event.set("purchase_cosmetic_error", error.as_str());
            }
            reply.send_reliable(load.channel, &open.body, true, true, false).map_err(|e| e.to_string())?;
            send(reply, load.channel, gun_owner_body(load), false, false)?;
            let skin = open.cosmetics.as_ref().map(|c| Value::from(c.skin.asset_id.as_str())).unwrap_or(Value::Null);
            reply.event.set("purchase_opened", json!([load.item.display_name, load.channel, load.actor_guid, skin, open.body.bit_count, attachments.len()]));
        }
        1 => {
            send(reply, load.channel, gun_role_body(load), false, false)?;
        }
        2 => {
            send(reply, load.channel, gun_state_machine_open_body(load), true, false)?;
        }
        3 => {
            let allocator = reply.conn.authority.purchases.allocator.get_or_insert_with(PurchaseAllocator::default);
            let body = gun_initial_state_body(load, allocator);
            send(reply, load.channel, body, true, false)?;
        }
        4 => slot_gun(reply, load)?,
        5 => {
            register_gun(reply, env, job);
            return Ok(());
        }
        _ => {}
    }
    job.stage += 1;
    job.next = now + STAGE_DELAY;
    Ok(())
}

/// stage 4: slot Contents and an inventory correction naming it current; the
/// slot's previous item (or the starting classic) is closed and unmirrored.
fn slot_gun(reply: &mut Reply, load: &GunLoad) -> Result<(), String> {
    let slot = load.item.slot;
    send(reply, 8, aries_rep::bodies::item_slot_contents_block(slot, load.actor_guid, PAWN_ACTOR_GUID), false, false)?;
    let correction_index = reply.conn.correction_index + 1;
    send(reply, 8, aries_rep::bodies::inventory_correction_body(correction_index, Some(load.actor_guid)), false, false)?;
    reply.conn.correction_index = correction_index;
    let mut replaced = reply.conn.authority.purchases.slot_items.get(&slot).cloned();
    let pistol = &starting()[0];
    if replaced.is_none() && slot == pistol.slot && !reply.conn.authority.mirror.starting_pistol_replaced {
        replaced = Some(SlotItem { channel: pistol.channel, key: pistol.key.clone(), actor_guid: pistol.actor_guid, class_name: pistol.class_name.clone() });
        reply.conn.authority.mirror.starting_pistol_replaced = true;
    }
    if let Some(old) = replaced {
        reply.send_close(old.channel).map_err(|e| e.to_string())?;
        let authority = &mut reply.conn.authority;
        authority.mirror.purchased.retain(|entry| entry.key != old.key);
        if authority.purchased_class_guids.get(&old.class_name) == Some(&old.actor_guid) {
            authority.purchased_class_guids.remove(&old.class_name);
        }
        reply.event.set("purchase_replaced", json!([old.class_name, old.channel, old.actor_guid]));
    }
    let class_name = load.item.class_name.to_string();
    let authority = &mut reply.conn.authority;
    authority.purchases.slot_items.insert(slot, SlotItem { channel: load.channel, key: load.key(), actor_guid: load.actor_guid, class_name: class_name.clone() });
    authority.purchased_class_guids.insert(class_name, load.actor_guid);
    reply.event.set("purchase_slotted", json!([load.item.display_name, slot, correction_index]));
    Ok(())
}

/// stage 5: once the client's slot holds the gun's class, name every state its
/// machine has, open its extra machines and start mirroring it.
fn register_gun(reply: &mut Reply, env: &mut Env, job: &mut PurchaseJob) {
    let JobLoad::Gun(load) = &mut job.load else {
        unreachable!("gun job");
    };
    let slot = load.item.slot;
    let mut machine = None;
    if env.live.attached() {
        match env.live.slot_machines(&[slot]) {
            Ok(machines) => machine = machines.into_iter().find(|m| m.slot == slot),
            Err(error) => reply.event.set("purchase_register_error", error),
        }
    }
    let Some(machine) = machine.filter(|m| m.class_name == load.item.class_name) else {
        job.registered_tries += 1;
        if job.registered_tries > REGISTER_TRIES {
            job.stage = STAGE_DONE;
            reply.event.set("purchase_register_gave_up", load.item.display_name);
        } else {
            job.next = reply.now + REGISTER_RETRY;
        }
        return;
    };
    let names = env.live.state_names(machine.state_machine).unwrap_or_default();
    let extra_machines = match env.live.item_machines(machine.item) {
        Ok(machines) => {
            let views: Vec<_> = machines.iter().map(|m| m.plan_view()).collect();
            let allocator = reply.conn.authority.purchases.allocator.get_or_insert_with(PurchaseAllocator::default);
            plan_extra_machines(allocator, &views, machine.state_machine)
        }
        Err(error) => {
            reply.event.set("extra_machines_error", error);
            Vec::new()
        }
    };
    reply.event.set("purchase_extra_machines", extra_names(&extra_machines));
    let allocator = reply.conn.authority.purchases.allocator.get_or_insert_with(PurchaseAllocator::default);
    let equippable = match gun_equippable(load, Some(allocator), &names, extra_machines) {
        Ok(equippable) => equippable,
        Err(error) => {
            reply.event.set("purchase_error", error.to_string());
            job.stage = STAGE_DONE;
            return;
        }
    };
    send_extra_machines(reply, &equippable);
    let purchased = &mut reply.conn.authority.mirror.purchased;
    purchased.retain(|entry| entry.slot != equippable.slot);
    purchased.push(equippable);
    reply.event.set("purchase_ready", json!([load.item.display_name, names.len()]));
    job.stage = STAGE_DONE;
}
