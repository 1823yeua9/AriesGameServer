//! complete bunch bodies shared by many senders (range_server_bunch 6.2).

use aries_bits::{BitWriter, Bits};

use crate::block::{Target, append_rep_block};
use crate::props::object_property;
use crate::RepResult;

/// AActor.Owner rep handle.
pub const ACTOR_OWNER_HANDLE: u32 = 12;
/// UItemSlot.Contents rep handle.
pub const ITEM_SLOT_CONTENTS_HANDLE: u32 = 1;
pub const PAWN_GUID: u32 = 10;
pub const INVENTORY_GUID: u32 = 60;
pub const ITEM_SLOT_CLASS_GUID: u32 = 61;
pub const FIRST_ITEM_SLOT_GUID: u32 = 62;

/// net guid of UItemSlot_<slot> (dynamic subobject of the inventory).
pub fn item_slot_net_guid(slot: u32) -> u32 {
    FIRST_ITEM_SLOT_GUID + 2 * slot
}

/// python build_channel_actor_owner_block: rep block on the channel actor, Owner = owner.
pub fn channel_actor_owner_block(actor_guid: u32, owner_guid: u32) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    append_rep_block(&mut w, Target::Actor, &object_property(ACTOR_OWNER_HANDLE, owner_guid)?, actor_guid)?;
    Ok(w.finish()?)
}

/// python build_item_slot_contents_block: dynamic-subobject block on the slot
/// (class 61, outer inventory 60) setting Contents, encoded with the pawn guid.
pub fn item_slot_contents_block(slot: u32, item_guid: u32, pawn_guid: u32) -> RepResult<Bits> {
    let plain = object_property(ITEM_SLOT_CONTENTS_HANDLE, item_guid)?;
    let encoded = crate::encode_block_payload(&plain, pawn_guid)?;
    let mut w = BitWriter::new();
    w.append_bits(1, 1); // bHasRepLayout
    w.append_bits(0, 1); // subobject
    w.append_packed_uint(item_slot_net_guid(slot) as u64);
    w.append_bits(0, 1); // not stably named
    w.append_bits(0, 1); // extra writer bit
    w.append_packed_uint(ITEM_SLOT_CLASS_GUID as u64);
    w.append_bits(0, 1); // actor is not the outer
    w.append_packed_uint(INVENTORY_GUID as u64);
    w.append_packed_uint(encoded.bit_count as u64);
    w.append_bits_value(&encoded);
    Ok(w.finish()?)
}

/// python build_inventory_correction_body: AuthServerCorrectRepVariables on the
/// inventory (60): CurrentEquippable (25, optional) and CorrectionIndex (26, u32).
pub fn inventory_correction_body(correction_index: u32, current_equippable_guid: Option<u32>) -> RepResult<Bits> {
    if correction_index == 0 {
        return Err(crate::RepError::Invalid("correction index must be > 0".into()));
    }
    let mut rep = BitWriter::new();
    rep.append_bits(0, 1);
    if let Some(guid) = current_equippable_guid {
        rep.append_packed_uint(25);
        rep.append_packed_uint(guid as u64);
    }
    rep.append_packed_uint(26);
    rep.append_u32(correction_index);
    rep.append_packed_uint(0);
    let mut w = BitWriter::new();
    append_rep_block(&mut w, Target::Subobject(INVENTORY_GUID), &rep.finish()?, PAWN_GUID)?;
    Ok(w.finish()?)
}
