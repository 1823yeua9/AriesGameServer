//! loopback harness: a scripted client drives the stateless handshake, hello,
//! nmt challenge/welcome, join and session acks through RangeServer, and
//! decodes every reply.

use std::net::SocketAddr;

use aries_net::AresGcm;
use aries_net::client::open_server_datagram;
use aries_net::control::{inspect_first_bunch, read_packet_header};
use aries_net::handshake::{self, CHALLENGE_BITS};
use aries_range::{Outbox, RangeConfig, RangeServer};

mod common;
use common::*;

#[test]
fn handshake_through_session_acks() {
    let source: SocketAddr = "127.0.0.1:50506".parse().unwrap();
    let mut server = RangeServer::new(RangeConfig::default(), Box::new(FixedKeys), None);
    let mut client = Client { gcm: AresGcm::new(&CLIENT_KEY), server_gcm: AresGcm::new(&SERVER_KEY), sequence: 8000, acked: 11195, counter: 700 };
    let mut out = Outbox::default();
    let mut now = 100.0;

    // stateless handshake
    server.handle_datagram(now, source, &initial_datagram(), &mut out);
    let (_, challenge) = out.datagrams.pop().expect("challenge");
    assert_eq!(challenge.len(), 4 + CHALLENGE_BITS.div_ceil(8));
    server.handle_datagram(now, source, &response_datagram(&challenge), &mut out);
    let (_, ack) = out.datagrams.pop().expect("ack");
    let ack_packet = handshake::parse_handshake(&[vec![0u8; 4], ack.clone()].concat()).ok();
    assert!(ack_packet.is_none() || ack_packet.unwrap().packet_type == 3);
    assert!(out.datagrams.is_empty());

    // hello (232 bits) is stored, nothing sent
    let hello = client.control(0, true, 232);
    let datagram = client.seal(&hello);
    server.handle_datagram(now, source, &datagram, &mut out);
    assert!(out.datagrams.is_empty());

    // 77-bit packet -> nmt challenge with S = client's acked + 1
    let short = client.short();
    let datagram = client.seal(&short);
    server.handle_datagram(now, source, &datagram, &mut out);
    let (_, reply) = out.datagrams.pop().expect("nmt challenge");
    let (packet, stateless) = open_server_datagram(&reply, &client.server_gcm).expect("opens");
    assert_eq!(stateless, STATELESS);
    let header = read_packet_header(&packet).unwrap();
    assert_eq!(header.sequence, 11196);
    let bunch = inspect_first_bunch(&packet).expect("control bunch");
    assert_eq!((bunch.channel_index, bunch.control_message_type, bunch.is_open), (0, Some(3), false));
    assert_eq!(bunch.channel_sequence, Some(11197 & 0xFFF));

    // login -> welcome
    now += 0.5;
    let login = client.control(5, false, 600);
    let datagram = client.seal(&login);
    server.handle_datagram(now, source, &datagram, &mut out);
    let (_, reply) = out.datagrams.pop().expect("welcome");
    let (packet, _) = open_server_datagram(&reply, &client.server_gcm).unwrap();
    assert_eq!(read_packet_header(&packet).unwrap().sequence, 11197);
    assert_eq!(inspect_first_bunch(&packet).unwrap().control_message_type, Some(1));

    // join -> join ack (77-bit ack-only, mask 3)
    let join = client.control(9, false, 142);
    let datagram = client.seal(&join);
    server.handle_datagram(now, source, &datagram, &mut out);
    let (_, reply) = out.datagrams.pop().expect("join ack");
    let (packet, _) = open_server_datagram(&reply, &client.server_gcm).unwrap();
    assert_eq!(packet.bit_count, 77);
    let header = read_packet_header(&packet).unwrap();
    assert_eq!((header.sequence, header.history), (11198, 3));

    // inside 0.1 s: no session ack; after: one
    now += 0.05;
    let short = client.short();
    let datagram = client.seal(&short);
    server.handle_datagram(now, source, &datagram, &mut out);
    assert!(out.datagrams.is_empty());
    now += 0.06;
    let short = client.short();
    let datagram = client.seal(&short);
    server.handle_datagram(now, source, &datagram, &mut out);
    assert_eq!(out.datagrams.len(), 2, "session ack + the first bring-up stage");
    let (_, reply) = out.datagrams.remove(0);
    let (packet, _) = open_server_datagram(&reply, &client.server_gcm).unwrap();
    assert_eq!(packet.bit_count, 77);
    assert_eq!(read_packet_header(&packet).unwrap().sequence, 11199);

    // live log: "session ACK seq 11199 + ch3 open (142 B, seq 11200, ch seq 3005)"
    let (_, open) = out.datagrams.remove(0);
    assert_eq!(open.len(), 142);
    let (packet, _) = open_server_datagram(&open, &client.server_gcm).unwrap();
    assert_eq!(read_packet_header(&packet).unwrap().sequence, 11200);
    let bunch = inspect_first_bunch(&packet).expect("bunch");
    assert_eq!((bunch.channel_index, bunch.is_open, bunch.reliable, bunch.channel_sequence, bunch.has_exports), (3, true, true, Some(3005), true));
    let conn = &server.connections[&source];
    assert_eq!(conn.base_sequence, 11197 & 0xFFF);
    assert_eq!(conn.next_server_sequence, 11201);
}

/// drives a whole bring-up with a client that sends a packet every 100-130 ms:
/// every stage frame carries the right channel and fixed reliable sequence, and
/// every channel's reliable sequences arrive contiguous and in order.
#[test]
fn full_bringup_sequences() {
    use std::collections::HashMap;
    let source: SocketAddr = "127.0.0.1:50507".parse().unwrap();
    let mut server = RangeServer::new(RangeConfig::default(), Box::new(FixedKeys), None);
    let mut client = Client { gcm: AresGcm::new(&CLIENT_KEY), server_gcm: AresGcm::new(&SERVER_KEY), sequence: 100, acked: 4000, counter: 1 };
    let mut out = Outbox::default();
    let mut now = 50.0;
    server.handle_datagram(now, source, &initial_datagram(), &mut out);
    let (_, challenge) = out.datagrams.pop().unwrap();
    server.handle_datagram(now, source, &response_datagram(&challenge), &mut out);
    out.datagrams.clear();
    for packet in [client.control(0, true, 232)] {
        let d = client.seal(&packet);
        server.handle_datagram(now, source, &d, &mut out);
    }
    let short = client.short();
    let d = client.seal(&short);
    server.handle_datagram(now, source, &d, &mut out);
    let login = client.control(5, false, 600);
    let d = client.seal(&login);
    server.handle_datagram(now, source, &d, &mut out);
    let join = client.control(9, false, 142);
    let d = client.seal(&join);
    server.handle_datagram(now, source, &d, &mut out);
    out.datagrams.clear();
    let base = server.connections[&source].base_sequence;

    // per channel: the reliable sequences seen, in send order
    let mut per_channel: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut rng = aries_world::PyRandom::new(7);
    for _ in 0..1200 {
        now += 0.1 + 0.03 * rng.random();
        let short = client.short();
        let d = client.seal(&short);
        server.handle_datagram(now, source, &d, &mut out);
        for (_, datagram) in out.datagrams.drain(..) {
            let (packet, _) = open_server_datagram(&datagram, &client.server_gcm).unwrap();
            if let Some(bunch) = inspect_first_bunch(&packet) {
                if bunch.reliable {
                    per_channel.entry(bunch.channel_index).or_default().push(bunch.channel_sequence.unwrap());
                }
            }
        }
    }
    // ch4 +8/+9 range hud classes; ch5 +6 hud rebuild, +7/+8 streaming, +9 settings request
    let expected_counts = [(3u32, 1usize), (4, 10), (5, 10), (6, 11), (7, 2), (8, 51), (9, 5), (10, 2), (11, 2), (12, 2), (13, 5), (14, 2), (15, 2), (16, 2), (17, 2), (18, 2)];
    for (channel, count) in expected_counts {
        let sequences = per_channel.get(&channel).unwrap_or_else(|| panic!("channel {channel} never sent"));
        let wanted: Vec<u32> = (0..count as u32).map(|i| (base + i) & 0xFFF).collect();
        assert_eq!(sequences, &wanted, "channel {channel}");
    }
    assert_eq!(per_channel.len(), expected_counts.len());
    let bringup = &server.connections[&source].bringup;
    assert!(bringup.sent(aries_range::bringup::Stage::MeleeInactiveState));
    assert_eq!(server.connections[&source].correction_index, 5);
}

/// a client whose saved-move number climbs ~15 per tick once the pawn exists.
struct ScriptedMoves {
    number: u32,
    stall: bool,
}

impl aries_range::LiveClient for ScriptedMoves {
    fn saved_moves(&mut self) -> Result<Option<aries_range::SavedMoves>, String> {
        if !self.stall {
            self.number += 15;
        }
        Ok(Some(aries_range::SavedMoves { newest: self.number, count: 20, ..Default::default() }))
    }

    fn latest_generated_input_event(&mut self) -> Result<Option<u32>, String> {
        Ok(None)
    }
}

#[test]
fn move_acks_follow_client_restart() {
    let source: SocketAddr = "127.0.0.1:50508".parse().unwrap();
    let mut server = RangeServer::new(RangeConfig::default(), Box::new(FixedKeys), None);
    server.live = Box::new(ScriptedMoves { number: 186, stall: false });
    let mut client = Client { gcm: AresGcm::new(&CLIENT_KEY), server_gcm: AresGcm::new(&SERVER_KEY), sequence: 300, acked: 900, counter: 5 };
    let mut out = Outbox::default();
    let mut now = 10.0;
    server.handle_datagram(now, source, &initial_datagram(), &mut out);
    let (_, challenge) = out.datagrams.pop().unwrap();
    server.handle_datagram(now, source, &response_datagram(&challenge), &mut out);
    for packet in [client.control(0, true, 232), client.short(), client.control(5, false, 600), client.control(9, false, 142)] {
        let d = client.seal(&packet);
        server.handle_datagram(now, source, &d, &mut out);
    }
    out.datagrams.clear();

    let mut acks: Vec<(usize, usize)> = Vec::new(); // (datagram bytes, body bits)
    let mut restart_seen = false;
    for _ in 0..400 {
        now += 0.125;
        let short = client.short();
        let d = client.seal(&short);
        server.handle_datagram(now, source, &d, &mut out);
        restart_seen |= server.connections[&source].bringup.sent(aries_range::bringup::Stage::ClientRestart);
        for (_, datagram) in out.datagrams.drain(..) {
            let (packet, _) = open_server_datagram(&datagram, &client.server_gcm).unwrap();
            if let Some(bunch) = inspect_first_bunch(&packet) {
                if !bunch.reliable && bunch.channel_index == 8 {
                    assert!(restart_seen, "no move ack before ClientRestart");
                    acks.push((datagram.len(), bunch.bunch_bit_count));
                }
            }
        }
        if acks.len() > 30 {
            break;
        }
    }
    assert!(acks.len() > 30);
    assert_eq!(acks[0].1, 950, "placement correction body");
    for &(bytes, bits) in &acks[1..] {
        assert_eq!((bytes, bits), (39, 90), "ClientAckInputAndMovement");
    }
    assert!(server.connections[&source].moves.correction_sent);

    // a move number that does not advance sends nothing
    server.live = Box::new(ScriptedMoves { number: 99_999, stall: true });
    server.connections.get_mut(&source).unwrap().moves.last_number = Some(99_999);
    now += 0.125;
    let short = client.short();
    let d = client.seal(&short);
    server.handle_datagram(now, source, &d, &mut out);
    assert!(out.datagrams.iter().all(|(_, d)| {
        let (packet, _) = open_server_datagram(d, &client.server_gcm).unwrap();
        inspect_first_bunch(&packet).is_none_or(|b| b.reliable || b.channel_index != 8)
    }));
}
