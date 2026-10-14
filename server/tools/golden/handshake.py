"""stateless handshake: parse, challenge and ack with fixed randomness."""

import random
import struct


def _initial(rng, packet_type, timestamp=0.0, cookie=bytes(20)):
    bits = []

    def put(v, w):
        bits.extend((v >> i) & 1 for i in range(w))

    put(rng.randrange(4), 2)
    put(rng.randrange(8), 3)
    put(1, 1)
    put(0, 1)
    put(4, 8)
    put(4, 8)
    put(packet_type, 8)
    put(rng.randrange(256), 8)
    put(rng.getrandbits(32), 32)
    put(rng.getrandbits(16), 16)
    put(rng.randrange(2), 1)
    put(int.from_bytes(struct.pack("<d", timestamp), "little"), 64)
    put(int.from_bytes(cookie, "little"), 160)
    for _ in range(16):  # the client pads 8-16 random bytes
        put(rng.randrange(256), 8)
    put(1, 1)
    out = bytearray((len(bits) + 7) // 8)
    for i, b in enumerate(bits):
        out[i // 8] |= b << (i % 8)
    return bytes(8) + bytes(out)


def section_handshake():
    import os
    import range_udp_handshake as ruh

    rng = random.Random(77)
    cases = []
    for i in range(12):
        initial = _initial(rng, 0)
        packet = ruh.parse_handshake(initial)
        timestamp = 1000.0 + rng.random() * 50000
        cookie = bytes(rng.randrange(256) for _ in range(20))
        padding = bytes(rng.randrange(256) for _ in range(8))
        real = os.urandom
        ruh.os.urandom = lambda n, p=padding: p[:n]
        try:
            challenge = ruh.build_challenge(packet, timestamp, cookie)
            wrapped = ruh.wrap_raw_challenge(challenge, ruh.challenge_bit_count, 4, False)
            response_datagram = _initial(rng, 2, timestamp, cookie)
            response = ruh.parse_handshake(response_datagram)
            ack = ruh.wrap_raw_challenge(ruh.build_ack(response), ruh.challenge_bit_count, 4, False)
        finally:
            ruh.os.urandom = real
        cases.append({"initial": initial.hex(), "timestamp": timestamp, "cookie": cookie.hex(), "padding": padding.hex(),
                      "challenge": wrapped.hex(), "response": response_datagram.hex(), "ack": ack.hex(),
                      "parsed": {"travel_count": packet.travel_count, "client_id": packet.client_id,
                                 "packet_count": packet.packet_count, "network_version": packet.network_version,
                                 "runtime_features": packet.runtime_features, "secret_id": packet.secret_id}})
    return cases


SECTIONS = {"handshake": section_handshake}
