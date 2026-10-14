//! content blocks, netguid exports and dynamic actor headers: the shapes every
//! range builder is made of (range_server_bunch + range_equippables helpers).

use aries_bits::{BitWriter, Bits, append_quantized_vector10, compress_short};

use crate::content::{ContentError, encode_block_payload};

/// netguid export header: [0 = netguid exports, not netfield][32-bit count].
pub fn append_export_header(w: &mut BitWriter, count: u32) {
    w.append_bits(0, 1);
    w.append_bits(count as u64, 32);
}

/// a static object and its package, both by inline path:
/// [obj][5][pkg][5][0][fstring pkg][0:32][fstring obj][0:32].
pub fn append_static_export_entry(w: &mut BitWriter, object_name: &str, package_path: &str, object_guid: u32, package_guid: u32) {
    w.append_packed_uint(object_guid as u64);
    w.append_bits(5, 8); // inline path | checksum
    w.append_packed_uint(package_guid as u64);
    w.append_bits(5, 8);
    w.append_packed_uint(0);
    w.append_fstring_ascii(package_path);
    w.append_bits(0, 32);
    w.append_fstring_ascii(object_name);
    w.append_bits(0, 32);
}

/// a stably-named subobject of an already-known outer: [obj][5][outer][0][fstring name][0:32].
pub fn append_stable_child_entry(w: &mut BitWriter, object_guid: u32, outer_guid: u32, object_name: &str) {
    w.append_packed_uint(object_guid as u64);
    w.append_bits(5, 8); // stable path | checksum
    w.append_packed_uint(outer_guid as u64);
    w.append_bits(0, 8); // outer already known
    w.append_fstring_ascii(object_name);
    w.append_bits(0, 32);
}

/// python range_equippables._append_stable_child_export: header for one entry + the entry.
pub fn append_stable_child_export(w: &mut BitWriter, object_guid: u32, outer_guid: u32, object_name: &str) {
    append_export_header(w, 1);
    append_stable_child_entry(w, object_guid, outer_guid, object_name);
}

/// what a content block targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// the channel's actor
    Actor,
    /// a stably-named subobject by its net guid
    Subobject(u32),
}

/// [bHasRepLayout][bIsActor]{[guid][bStablyNamed=1]}[packed bits][payload] with an already-encoded payload.
pub fn append_content_block(w: &mut BitWriter, has_rep_layout: bool, target: Target, encoded: &Bits) {
    w.append_bool(has_rep_layout);
    match target {
        Target::Actor => w.append_bits(1, 1),
        Target::Subobject(guid) => {
            w.append_bits(0, 1);
            w.append_packed_uint(guid as u64);
            w.append_bits(1, 1); // stably named
        }
    }
    w.append_packed_uint(encoded.bit_count as u64);
    w.append_bits_value(encoded);
}

/// rep-layout block from a plain payload, encoded with the channel actor's guid.
pub fn append_rep_block(w: &mut BitWriter, target: Target, plain: &Bits, actor_guid: u32) -> Result<(), ContentError> {
    let encoded = encode_block_payload(plain, actor_guid)?;
    append_content_block(w, true, target, &encoded);
    Ok(())
}

/// rpc block (no rep layout) from a plain payload, encoded with the channel actor's guid.
pub fn append_rpc_block(w: &mut BitWriter, target: Target, plain: &Bits, actor_guid: u32) -> Result<(), ContentError> {
    let encoded = encode_block_payload(plain, actor_guid)?;
    append_content_block(w, false, target, &encoded);
    Ok(())
}

/// python append_actor_rep_layout_payload(b"", 0, encoded, bits).
pub fn actor_rep_layout_block(encoded: &Bits) -> Result<Bits, aries_bits::BitError> {
    let mut w = BitWriter::new();
    append_content_block(&mut w, true, Target::Actor, encoded);
    w.finish()
}

/// python append_actor_rpc_content_block(b"", 0, encoded, bits).
pub fn actor_rpc_content_block(encoded: &Bits) -> Result<Bits, aries_bits::BitError> {
    let mut w = BitWriter::new();
    append_content_block(&mut w, false, Target::Actor, encoded);
    w.finish()
}

/// spawn transform for a dynamic actor open.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spawn {
    pub location: [f64; 3],
    pub yaw: f64,
}

/// python append_dynamic_actor_header: actor, archetype, level, then either
/// 4 default bits or location (+ yaw when it compresses to non-zero).
pub fn append_dynamic_actor_header(w: &mut BitWriter, actor_guid: u32, archetype_guid: u32, level_guid: u32, spawn: Option<Spawn>) {
    w.append_packed_uint(actor_guid as u64);
    w.append_packed_uint(archetype_guid as u64);
    w.append_packed_uint(level_guid as u64);
    let Some(spawn) = spawn else {
        w.append_bits(0, 4); // default position, rotation, scale, velocity
        return;
    };
    w.append_bits(1, 1); // position exists
    w.append_bits(1, 1); // quantized-vector format
    append_quantized_vector10(w, spawn.location);
    let yaw_short = compress_short(spawn.yaw);
    if yaw_short != 0 {
        w.append_bits(1, 1); // rotation present
        w.append_bits(0, 1); // pitch 0
        w.append_bits(1, 1);
        w.append_bits(yaw_short as u64, 16);
        w.append_bits(0, 1); // roll 0
        w.append_bits(0, 2); // default scale, velocity
    } else {
        w.append_bits(0, 3);
    }
}

/// python build_dynamic_actor_reference: one archetype export, then the header.
pub fn dynamic_actor_reference(archetype: (&str, &str), actor_guid: u32, archetype_guid: u32, package_guid: u32, level_guid: u32, spawn: Option<Spawn>) -> Result<Bits, aries_bits::BitError> {
    let mut w = BitWriter::new();
    if actor_guid < 2 || actor_guid & 1 != 0 || archetype_guid.min(package_guid).min(level_guid) < 3 || [archetype_guid, package_guid, level_guid].iter().any(|g| g & 1 == 0) {
        w.set_error(aries_bits::BitError::Malformed("invalid dynamic actor reference"));
    }
    append_export_header(&mut w, 1);
    for guid in [archetype_guid, package_guid] {
        w.append_packed_uint(guid as u64);
        w.append_bits(5, 8);
    }
    w.append_packed_uint(0);
    for component in [archetype.1, archetype.0] {
        w.append_fstring_ascii(component);
        w.append_bits(0, 32);
    }
    append_dynamic_actor_header(&mut w, actor_guid, archetype_guid, level_guid, spawn);
    w.finish()
}
