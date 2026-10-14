//! rep-layout property payloads (plain, before the content transform).
//! every payload is [0 = no checksum] then (packed handle, value)... then packed 0.
//! docs/port/02_bunch_primitives.md section 6.1.

use aries_bits::{BitWriter, Bits};

use crate::RepResult;

/// the empty rep payload [0][packed 0] = 9 bits (opens a subobject's layout).
pub fn empty_rep() -> Bits {
    let mut w = BitWriter::new();
    w.append_bits(0, 1);
    w.append_packed_uint(0);
    w.finish().expect("fixed")
}

/// python build_bool_property_payload: [0][handle][1-bit value][0].
pub fn bool_property(handle: u32, value: bool) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    w.append_bits(0, 1);
    w.append_packed_uint(handle as u64);
    w.append_bool(value);
    w.append_packed_uint(0);
    Ok(w.finish()?)
}

/// python build_object_property_payload: [0][handle][guid][0].
pub fn object_property(handle: u32, net_guid: u32) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    w.append_bits(0, 1);
    w.append_packed_uint(handle as u64);
    w.append_packed_uint(net_guid as u64);
    w.append_packed_uint(0);
    Ok(w.finish()?)
}

/// python build_pawn_autonomous_role_payload / build_base_pistol_autonomous_role_payload:
/// RemoteRole (handle 4) as a `width`-bit ENetRole. production: width 3, value 2.
pub fn autonomous_role(role_bit_width: u32, role_value: u32) -> RepResult<Bits> {
    let mut w = BitWriter::new();
    w.append_bits(0, 1);
    w.append_packed_uint(4);
    w.append_bits(role_value as u64, role_bit_width);
    w.append_packed_uint(0);
    Ok(w.finish()?)
}
