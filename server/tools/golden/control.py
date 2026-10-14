"""session acks, nmt control replies and first-bunch inspection."""

import random


def _bits(fields):
    value = pos = 0
    for v, w in fields:
        value |= (v & ((1 << w) - 1)) << pos
        pos += w
    return value.to_bytes((pos + 7) // 8, "little"), pos


def _packed(v):
    out = []
    while True:
        part = v & 0x7F
        v >>= 7
        out.append(((part << 1) | bool(v), 8))
        if not v:
            return out


def hello_packet(rng, seq, acked, name):
    head = [((seq << 18) | (acked << 4), 32), (rng.getrandbits(32), 32), (1, 1), (0, 10), (0, 1)]
    bunch = [(1, 1), (1, 1), (0, 1), (0, 1), (1, 1)] + _packed(0) + [(0, 1), (0, 1), (0, 1), (0, 1), (rng.randrange(4096), 12), (1, 1)] + _packed(name)
    used = sum(w for _, w in head + bunch) + 13
    body_bits = 231 - used
    body = [(0, 8), (rng.getrandbits(body_bits - 8), body_bits - 8)]
    return _bits(head + bunch + [(body_bits, 13)] + body + [(1, 1)])


def client_packet(rng, seq, acked, message_type, total_bits=200, reliable=True):
    head = [((seq << 18) | (acked << 4), 32), (rng.getrandbits(32), 32), (1, 1), (0, 10), (0, 1)]
    bunch = [(1, 1), (0, 1), (0, 1), (0, 1), (int(reliable), 1)] + _packed(0) + [(0, 1), (0, 1), (0, 1), (0, 1)]
    if reliable:
        bunch += [(rng.randrange(4096), 12)]
    bunch += [(1, 1)] + _packed(2)
    used = sum(w for _, w in head + bunch) + 13
    body_bits = total_bits - 1 - used
    body = [(message_type, 8), (rng.getrandbits(body_bits - 8), body_bits - 8)]
    return _bits(head + bunch + [(body_bits, 13)] + body + [(1, 1)])


def section_control():
    import range_udp_handshake as ruh

    rng = random.Random(4040)
    key = bytes(range(32))
    out = {"session_ack": [], "replies": [], "inspect": [], "history": []}
    for i in range(6):
        server, client, mask, sh = rng.randrange(0x4000), rng.randrange(0x4000), rng.getrandbits(32), rng.randrange(32)
        frame, _, _ = ruh.build_server_session_ack_from_sequence(client, sh, key, server, mask)
        out["session_ack"].append({"server": server, "client": client, "mask": mask, "stateless": sh, "frame": frame.hex()})
    for i in range(6):
        hello, hello_bits = hello_packet(rng, rng.randrange(0x4000), rng.randrange(0x4000), [2, 255, 300][i % 3])
        short, short_bits = _bits([((rng.randrange(0x4000) << 18) | (rng.randrange(0x4000) << 4), 32), (rng.getrandbits(32), 32), (1, 1), (0, 11), (1, 1)])
        sh = rng.randrange(32)
        challenge = ruh.build_server_challenge_probe(hello, hello_bits, short, short_bits, sh, key)
        login, login_bits = client_packet(rng, rng.randrange(0x4000), rng.randrange(0x4000), 5)
        server_sequence = rng.randrange(0x4000)
        welcome = ruh.build_server_welcome_probe(hello, hello_bits, login, login_bits, sh, key, server_sequence)
        out["replies"].append({"hello": hello.hex(), "short": short.hex(), "short_bits": short_bits, "login": login.hex(), "login_bits": login_bits,
                               "stateless": sh, "server_sequence": server_sequence,
                               "challenge": [challenge[0].hex(), challenge[1], challenge[2]],
                               "welcome": [welcome[0].hex(), welcome[1], welcome[2]]})
    for i in range(10):
        reliable = i % 2 == 0
        pkt, bits = client_packet(rng, rng.randrange(0x4000), rng.randrange(0x4000), rng.randrange(256), 150 + 37 * i, reliable)
        out["inspect"].append({"packet": pkt.hex(), "bits": bits, "info": ruh.inspect_control_bunch(pkt, bits)})
    from collections import deque
    recent = deque(maxlen=64)
    seq = 16370
    for i in range(80):
        seq = (seq + rng.choice((1, 1, 1, 2, 3))) & 0x3FFF
        before = list(recent)
        mask = ruh.update_received_packet_history(recent, seq)
        out["history"].append({"recent": before, "current": seq, "mask": mask})
    return out


SECTIONS = {"control": section_control}
