//! the client side of the ares transport, for the loopback harness and tests:
//! seal a client packet the way the game does, and open server datagrams.

use aries_bits::{BitReader, BitWriter, Bits};

use crate::frame::ARES_TRAILER;
use crate::gcm::{AresGcm, CLIENT_TO_SERVER, SERVER_TO_CLIENT};
use crate::transform::{inverse_short_packet, transform_short_packet};

/// inverse of decode_client_packet: transform with the counter, gcm, then
/// [64 envelope bits][6 stateless][32 counter][32 tag][ciphertext][0xcc trailer][terminal].
pub fn seal_client_packet(decoded: &Bits, counter: u32, stateless_header: u32, gcm: &AresGcm) -> Vec<u8> {
    let bits = decoded.bit_count;
    let mut data = decoded.bytes.clone();
    transform_short_packet(&mut data, bits, counter);
    // the trailer shares the last partial byte with the payload, like the server side
    let trailing = bits & 7;
    if trailing != 0 {
        let last = data.len() - 1;
        data[last] &= (1u8 << trailing) - 1;
        data[last] |= ARES_TRAILER << trailing;
    }
    let tag = gcm.seal(&AresGcm::nonce(counter, CLIENT_TO_SERVER), &mut data);
    let mut w = BitWriter::new();
    w.append_bits(0, 64);
    w.append_bits(stateless_header as u64, 6);
    w.append_u32(counter);
    w.append_bits(u32::from_le_bytes(tag) as u64, 32);
    w.append_payload(&data, data.len() * 8);
    // ciphertext covers ceil(bits/8) bytes; the terminal goes after payload + 8 trailer bits
    let terminal = 64 + 6 + 72 + bits;
    let have = w.bit_count();
    if have < terminal {
        let pad_bits = terminal - have;
        let remaining_trailer = if trailing != 0 { (ARES_TRAILER >> (8 - trailing)) as u64 } else { ARES_TRAILER as u64 };
        w.append_bits(remaining_trailer & ((1u64 << pad_bits) - 1), pad_bits as u32);
    }
    w.append_bits(1, 1);
    w.finish().expect("client frame").bytes
}

/// opens a server datagram (seal_packet output) back to its decoded packet bits.
pub fn open_server_datagram(datagram: &[u8], gcm: &AresGcm) -> Option<(Bits, u32)> {
    let last = *datagram.last()?;
    if last == 0 {
        return None;
    }
    let total_bits = (datagram.len() - 1) * 8 + (8 - last.leading_zeros() as usize) - 1;
    let bit_count = total_bits.checked_sub(32 + 6 + 64 + 8)?;
    let mut r = BitReader::new(datagram, datagram.len() * 8);
    r.set_position(32);
    let stateless_header = r.read_bits(6) as u32;
    let counter = r.read_u32();
    let tag = r.read_u32().to_le_bytes();
    let mut data: Vec<u8> = (0..bit_count.div_ceil(8)).map(|_| r.read_bits(8) as u8).collect();
    if r.is_error() || !gcm.open(&AresGcm::nonce(counter, SERVER_TO_CLIENT), &mut data, &tag) {
        return None;
    }
    if !inverse_short_packet(&mut data, bit_count, counter) {
        return None;
    }
    if bit_count % 8 != 0 {
        let last = data.len() - 1;
        data[last] &= (1u8 << (bit_count % 8)) - 1;
    }
    Some((Bits { bytes: data, bit_count }, stateless_header))
}
