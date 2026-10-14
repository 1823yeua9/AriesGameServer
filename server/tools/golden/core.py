"""core sections: bit primitives, transport, content transform."""

def hexbits(writer_or_pair):
    if isinstance(writer_or_pair, tuple):
        body, bits = writer_or_pair
        return {"hex": body.hex(), "bit_count": bits}
    return {"hex": writer_or_pair.to_bytes().hex(), "bit_count": writer_or_pair.bit_count}


def section_bits():
    from range_server_bunch import BitWriter, append_quantized_vector10
    from range_projectiles import append_quantized_vector, append_rotator_bytes

    cases = []

    def run(name, ops):
        w = BitWriter()
        for op in ops:
            kind = op[0]
            if kind == "bits":
                w.append_bits(op[1], op[2])
            elif kind == "packed":
                w.append_packed_uint(op[1])
            elif kind == "sint":
                w.append_serialize_int(op[1], op[2])
            elif kind == "fstring":
                w.append_fstring_ascii(op[1])
            elif kind == "qv10":
                append_quantized_vector10(w, tuple(op[1]))
            elif kind == "qv":
                append_quantized_vector(w, tuple(op[1]), op[2])
            elif kind == "rotbytes":
                append_rotator_bytes(w, *op[1])
            elif kind == "rotshorts":
                # build_dynamic_actor_reference / projectile header style
                for deg in op[1]:
                    s = round(deg * 65536.0 / 360.0) & 0xFFFF
                    w.append_bits(int(s != 0), 1)
                    if s:
                        w.append_bits(s, 16)
            else:
                raise ValueError(kind)
        cases.append({"name": name, "ops": ops, **hexbits(w)})

    run("bits_mixed", [["bits", 0, 1], ["bits", 5, 3], ["bits", 0xABCD, 16], ["bits", 1, 1], ["bits", 0x12345678, 32], ["bits", 0x7F, 7]])
    run("bits_64", [["bits", 1, 1], ["bits", 0xFEDCBA9876543210, 64], ["bits", 3, 2]])
    for v in (0, 1, 63, 64, 127, 128, 300, 16383, 16384, 300001, 610100, 0xFFFFFFFF):
        run(f"packed_{v}", [["bits", 1, 1], ["packed", v]])
    for v, m in ((44, 179), (25, 54), (57, 59), (56, 59), (0, 2), (1, 2), (13, 17), (3, 5), (123, 179), (4, 13), (0, 1), (170, 179), (65535, 65536), (5, 15)):
        run(f"sint_{v}_{m}", [["bits", 1, 1], ["sint", v, m]])
    for s in ("BombTeam", "/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK", "x"):
        run(f"fstring_{len(s)}", [["bits", 0, 3], ["fstring", s]])
    vecs = [[4825.0, -14600.0, 1351.8], [0.0, 0.0, 0.0], [0.04, -0.05, 0.05], [-0.05, 0.15, -1e-9],
            [6190.0, -11640.0, 1270.0], [3581.634, -12019.322, 1294.153], [-60000.5, 59999.95, 12.25], [1.0, 2.0, 3.0]]
    for i, v in enumerate(vecs):
        run(f"qv10_{i}", [["bits", 1, 1], ["qv10", v]])
        run(f"qv1_{i}", [["qv", v, 1]])
        run(f"qv100_{i}", [["bits", 0, 2], ["qv", v, 100]])
    for i, r in enumerate(([0.0, 90.0, 0.0], [5.076444, -130.79, 0.0], [-2.86, 359.99, 180.0], [0.00274658203125, 0.0082397460937, 1e-9], [-0.00274658203125, 180.0, -180.0])):
        run(f"rotbytes_{i}", [["rotbytes", r]])
        run(f"rotshorts_{i}", [["bits", 1, 1], ["rotshorts", r]])
    return cases


def make_client_datagram(decoded: bytes, bit_count: int, counter: int, stateless: int, send_key: bytes) -> bytes:
    """Inverse of decode_client_ares_packet, for tests: transform, gcm, then
    [64 envelope bits][6 stateless][32 counter][32 tag][ciphertext][trailer][terminal]."""
    import struct
    from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
    from range_udp_transform import transform_short_packet

    plain = transform_short_packet(decoded, bit_count, counter)
    nonce = struct.pack("<II", counter, 0x31ED27AB)
    enc = Cipher(algorithms.AES(send_key), modes.GCM(nonce)).encryptor()
    ciphertext = enc.update(plain) + enc.finalize()
    tag = enc.tag[:4]
    value = 0
    pos = 0

    def put(v, w):
        nonlocal value, pos
        value |= (v & ((1 << w) - 1)) << pos
        pos += w

    put(0x1122334455667788, 64)
    put(stateless, 6)
    put(counter, 32)
    put(int.from_bytes(tag, "little"), 32)
    for b in ciphertext:
        put(b, 8)
    # the plaintext bit count is recovered from the terminal bit: ciphertext
    # bits beyond bit_count are part of the 72 reserved bits budget.
    pos = 64 + 6 + 72 + bit_count
    put(1, 1)
    return value.to_bytes((pos + 7) // 8, "little")


def section_net():
    import random
    from range_udp_transform import inverse_short_packet, transform_short_packet
    from range_udp_handshake import decode_client_ares_packet, derive_client_receive_key, wrap_gcm
    from range_server_bunch import BunchHeader, build_server_bunch_frame

    rng = random.Random(1306)
    out = {"transform": [], "gcm": [], "frames": [], "client": [], "keys": []}
    for bits in (1, 7, 8, 9, 31, 32, 33, 63, 64, 65, 96, 127, 128, 200, 517, 1089, 1194, 4096, 8192):
        for counter in (0, 1, 721070435, 0xFFFFFFFF):
            data = bytes(rng.randrange(256) for _ in range((bits + 7) // 8))
            out["transform"].append({"bits": bits, "counter": counter, "plain": data.hex(),
                                     "forward": transform_short_packet(data, bits, counter).hex(),
                                     "inverse": inverse_short_packet(data, bits, counter).hex()})
    key = bytes(range(32))
    for bits in (9, 64, 100, 517, 1200):
        data = bytes(rng.randrange(256) for _ in range((bits + 7) // 8))
        if bits % 8:
            data = data[:-1] + bytes([data[-1] & ((1 << (bits % 8)) - 1)])
        out["gcm"].append({"key": key.hex(), "bits": bits, "plain": data.hex(),
                           "wrapped": wrap_gcm(data, "", bits, key).hex()})
    for i, (reliable, is_open, is_close, exports, chan, seq, bits) in enumerate((
            (True, True, False, True, 400, 0, 1197), (True, False, False, False, 8, 77, 25),
            (False, False, False, False, 8, 0, 39), (True, False, True, False, 21, 4095, 0),
            (True, False, False, True, 10000, 3, 1689), (False, False, False, False, 5, 0, 334))):
        body = bytes(rng.randrange(256) for _ in range((bits + 7) // 8))
        if bits % 8:
            body = body[:-1] + bytes([body[-1] & ((1 << (bits % 8)) - 1)])
        header = BunchHeader(channel_index=chan, channel_name_index=102, channel_sequence=seq,
                             is_open=is_open, is_close=is_close, is_reliable=reliable, has_exports=exports)
        info = dict(server_sequence=(1000 + 37 * i) & 0x3FFF, acked_client_sequence=(8299 + i) & 0x3FFF,
                    ack_history_mask=rng.getrandbits(32), stateless_header=8 + i)
        frame, packet_bits = build_server_bunch_frame(body, bits, header, receive_key=key, **info)
        out["frames"].append({"body": body.hex(), "bits": bits, "header": {
            "channel_index": chan, "channel_name_index": 102, "channel_sequence": seq, "is_open": is_open,
            "is_close": is_close, "is_reliable": reliable, "has_exports": exports}, "info": info,
            "key": key.hex(), "frame": frame.hex(), "packet_bits": packet_bits})
    send_key = bytes(reversed(range(32)))
    for bits, counter in ((1089, 1796011705), (1337, 7), (232, 0), (25, 99)):
        decoded = bytes(rng.randrange(256) for _ in range((bits + 7) // 8))
        if bits % 8:
            decoded = decoded[:-1] + bytes([decoded[-1] & ((1 << (bits % 8)) - 1)])
        datagram = make_client_datagram(decoded, bits, counter, 12, send_key)
        back = decode_client_ares_packet(datagram, send_key)
        assert back is not None and back[1] == bits and back[2] == counter and back[3] == 12
        mask = (1 << bits) - 1
        assert int.from_bytes(back[0], "little") & mask == int.from_bytes(decoded, "little")
        out["client"].append({"key": send_key.hex(), "datagram": datagram.hex(), "bits": bits,
                              "counter": counter, "stateless": 12, "decoded": decoded.hex()})
    for k in ("", "AAAA", "not base64!", "c2VjcmV0LXBsYXllci1rZXk="):
        out["keys"].append({"player_key": k, "receive_key": derive_client_receive_key(k).hex()})
    return out


def section_content():
    import random
    from range_server_bunch import (build_dynamic_actor_reference, encode_short_actor_rep_payload)

    rng = random.Random(53)
    out = {"transform": [], "offset": [], "actor_reference": []}
    sizes = [9, 10, 15, 16, 17, 25, 29, 31, 32, 33, 35, 39, 40, 41, 59, 63, 64, 65, 79, 89, 95, 96, 97, 112,
             127, 128, 129, 200, 257, 293, 316, 441, 561, 809, 993, 1000]
    for bits in sizes:
        for guid in (2, 4, 6, 10, 208, 300100, 610100, 0xFFFFFFFE):
            data = bytes(rng.randrange(256) for _ in range((bits + 7) // 8))
            if bits % 8:
                data = data[:-1] + bytes([data[-1] & ((1 << (bits % 8)) - 1)])
            out["transform"].append({"bits": bits, "guid": guid, "plain": data.hex(),
                                     "encoded": encode_short_actor_rep_payload(data, bits, 0, guid).hex()})
    for offset in range(1, 7):
        for guid in (2, 6, 10):
            v = rng.randrange(512)
            data = v.to_bytes(2, "little")
            out["offset"].append({"offset": offset, "guid": guid, "plain": data.hex(),
                                  "encoded": encode_short_actor_rep_payload(data, 9, offset, guid).hex()})
    for name, args in (
            ("bot", (("Default__TrainingBot_PC_C", "/Game/Characters/TrainingBot/TrainingBot_PC"), 300100, 300001, 300003, 5, (4825.0, -14600.0, 1351.8), 90.0)),
            ("bot_yaw0", (("Default__TrainingBot_PC_C", "/Game/Characters/TrainingBot/TrainingBot_PC"), 300100, 300001, 300003, 5, (4825.0, -14600.0, 1351.8), 0.0)),
            ("gamestate", (("Default__ShootingRangeGameState_C", "/Game/GameModes/ShootingRange/ShootingRangeGameState"), 2, 11, 13, 5, None, None)),
            ("warden", (("Default__BattleRifle_C", "/Game/Equippables/Guns/Rifles/BattleRifle/BattleRifle"), 520, 523, 525, 5, None, None)),
            ("dummy", (("Default__TargetPracticeTrainingBot_C", "/Game/Heroes/Decoy/TargetPracticeTrainingBot"), 300100, 300005, 300007, 5, (6190.0, -11640.0, 1270.0), 180.0))):
        body, bits = build_dynamic_actor_reference(*args)
        out["actor_reference"].append({"name": name, "archetype": list(args[0]), "actor": args[1], "archetype_guid": args[2],
                                       "package_guid": args[3], "level": args[4], "location": args[5], "yaw": args[6],
                                       "hex": body.hex(), "bit_count": bits})
    return out


SECTIONS = {"bits": section_bits, "net": section_net, "content": section_content}
