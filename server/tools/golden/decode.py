"""client -> server decode (spec 06): bunch walk, content transform, actor
blocks, controller rpcs (purchases, range settings) and ch8 input streams.

every expected value comes from calling the python decoders offline. the
inputs are the spec 06 section 13 vectors, the packets in the python tests,
synthetic packets built with the python BitWriter and content encoder,
bit-flip / truncation mutants, and real decoded client packets from the
responder logs (local_backend/runtime, field decoded_packet_hex).

the committed decode_packets fixture holds a curated set of captures. for a
one-off parity sweep over every captured packet set ARIES_DECODE_ALL_CAPTURES=1,
export decode_packets, run the rust tests with ARIES_DECODE_PACKETS=<that file>,
and do not commit it (it is several MB).
"""

import dataclasses
import glob
import json
import os
import random
from pathlib import Path


def err(error):
    return {"error": str(error), "error_type": type(error).__name__}


def attempt(fn):
    """fn() or its ValueError (UnicodeDecodeError included) as a dict."""
    try:
        return fn()
    except ValueError as error:
        return err(error)


def bunch_dict(bunch):
    return {"start_bit": bunch.start_bit, "is_control": bunch.is_control, "is_reliable": bunch.is_reliable,
            "is_partial": bunch.is_partial, "channel_index": bunch.channel_index,
            "channel_sequence": bunch.channel_sequence, "body_start_bit": bunch.body_start_bit,
            "end_bit": bunch.end_bit}


def block_dict(block):
    return {"bit_offset": block.bit_offset, "bit_count": block.bit_count, "has_rep_layout": block.has_rep_layout,
            "is_actor": block.is_actor, "payload": block.payload.hex()}


def stream_dict(stream):
    # None event fields are left out (the rust test leaves them out too)
    events = [{key: value for key, value in dataclasses.asdict(e).items() if value is not None} for e in stream.events]
    return {"marker_bit": stream.marker_bit, "end_bit": stream.end_bit, "first_event_id": stream.first_event_id,
            "last_event_id": stream.last_event_id, "events": events}


def optional_dict(value):
    return None if value is None else dataclasses.asdict(value)


def masked(data: bytes, bits: int) -> bytes:
    """bytes with the unused high bits of the last byte cleared."""
    if bits % 8 and data:
        return data[:-1] + bytes([data[-1] & ((1 << (bits % 8)) - 1)])
    return data


def random_bytes(rng, bits, clear_tail=True):
    data = bytes(rng.randrange(256) for _ in range((bits + 7) // 8))
    return masked(data, bits) if clear_tail else data


# ---------------------------------------------------------------- writers


def writer():
    from range_server_bunch import BitWriter
    return BitWriter()


def put_fstring(w, name):
    """ascii str (with nul), ("utf16", text), ("raw", length, bytes) or "" (length 0)."""
    if isinstance(name, str):
        if not name:
            w.append_bits(0, 32)
            return
        encoded = name.encode("ascii") + b"\0"
        w.append_bits(len(encoded), 32)
        for byte in encoded:
            w.append_bits(byte, 8)
        return
    if name[0] == "utf16":
        units = name[1].encode("utf-16-le") + b"\0\0"
        w.append_bits((1 << 32) - len(units) // 2, 32)
        for byte in units:
            w.append_bits(byte, 8)
        return
    _, length, raw = name
    w.append_bits(length, 32)
    for byte in raw:
        w.append_bits(byte, 8)


def put_ref(w, ref):
    """None -> guid 0; int -> mapped guid; ("bytes", [..]) -> raw packed bytes;
    ("default", flags) -> guid 1 without path; ("path", flags, outer, name, checksum)."""
    if ref is None:
        w.append_packed_uint(0)
    elif isinstance(ref, int):
        w.append_packed_uint(ref)
    elif ref[0] == "bytes":
        for byte in ref[1]:
            w.append_bits(byte, 8)
    elif ref[0] == "default":
        w.append_packed_uint(1)
        w.append_bits(ref[1], 8)
    else:
        _, flags, outer, name, checksum = ref
        w.append_packed_uint(1)
        w.append_bits(flags, 8)
        put_ref(w, outer)
        put_fstring(w, name)
        if flags & 4:
            w.append_bits(checksum, 32)


def chain(levels):
    """an outer chain of `levels` path exports ending in a null outer."""
    ref = None
    for level in range(levels):
        ref = ("path", 1, ref, f"L{level}", 0)
    return ref


def pair(w):
    return w.to_bytes(), w.bit_count


def rpc_payload(field, params, field_max=179, size=None):
    w = writer()
    w.append_serialize_int(field, field_max)
    body, bits = pair(params)
    w.append_packed_uint(bits if size is None else size)
    w.append_payload(body, bits)
    return pair(w)


def actor_block(plain, map_value, rep=0, actor=1):
    from range_server_bunch import encode_short_actor_rep_payload
    body, bits = plain
    wire = encode_short_actor_rep_payload(body, bits, 0, map_value)
    w = writer()
    w.append_bits(rep, 1)
    w.append_bits(actor, 1)
    w.append_packed_uint(bits)
    w.append_payload(wire, bits)
    return pair(w)


def write_bunch(w, body, control=False, open_=False, close=False, paused=False, reliable=True, channel=5,
                channel_bytes=None, exports=False, must_map=False, partial=False, flag4=False, sequence=0,
                partial_bits=0, name=None, name_bytes=None, size=None):
    """client bunch header per spec 06 4.1, then the body."""
    w.append_bits(int(control), 1)
    if control:
        w.append_bits(int(open_), 1)
        w.append_bits(int(close), 1)
        if close:
            return
    w.append_bits(int(paused), 1)
    w.append_bits(int(reliable), 1)
    if channel_bytes is not None:
        for byte in channel_bytes:
            w.append_bits(byte, 8)
    else:
        w.append_packed_uint(channel)
    for flag in (exports, must_map, partial, flag4):
        w.append_bits(int(flag), 1)
    if reliable:
        w.append_bits(sequence, 12)
    if partial:
        w.append_bits(partial_bits, 2)
    if control or reliable:
        has_name = name is not None or name_bytes is not None
        w.append_bits(int(has_name), 1)
        if name_bytes is not None:
            for byte in name_bytes:
                w.append_bits(byte, 8)
        elif name is not None:
            w.append_packed_uint(name)
    data, bits = body
    w.append_bits(bits if size is None else size, 13)
    w.append_payload(data, bits)


def client_packet(bunches, seq=100, acked=50, history=0xFFFFFFFF, info=0, flag75=0, terminator=True, tail=None):
    """decoded client packet: header word, history, packet info, bit 75, bunches, terminator."""
    w = writer()
    w.append_bits((seq << 18) | (acked << 4), 32)
    w.append_bits(history, 32)
    if info is None:
        w.append_bits(0, 1)
    else:
        w.append_bits(1, 1)
        w.append_bits(info, 10)
    w.append_bits(flag75, 1)
    for body, header in bunches:
        write_bunch(w, body, **header)
    if tail is not None:
        w.append_bits(*tail)
    if terminator:
        w.append_bits(1, 1)
    return pair(w)


def purchase_params(item=("absent",), slot=None, kind=None, extra=None):
    w = writer()
    if item == ("absent",):
        w.append_bits(0, 1)
    else:
        w.append_bits(1, 1)
        put_ref(w, item)
    for value, width in ((slot, 8), (kind, 3)):
        if value is None:
            w.append_bits(0, 1)
        else:
            w.append_bits(1, 1)
            w.append_bits(value, width)
    if extra:
        w.append_bits(*extra)
    return w


def object_params(ref):
    """a present object parameter (172/173)."""
    w = writer()
    w.append_bits(1, 1)
    put_ref(w, ref)
    return w


def settings_params(fields):
    """[(value, width)] in order."""
    w = writer()
    for value, width in fields:
        w.append_bits(value & ((1 << width) - 1), width)
    return w


def i32(value):
    return (value & 0xFFFFFFFF, 32)


def stream_bits(w, first_id, events, marker=0xA3, continuation=1, count=None, terminal=0):
    """an input stream: marker, 1, first id, packed count, events, terminal.
    events: (kind, payload_value) or ("raw", value, width)."""
    w.append_bits(marker, 8)
    w.append_bits(continuation, 1)
    w.append_bits(first_id, 32)
    w.append_packed_uint(len(events) if count is None else count)
    widths = {0: 37, 1: 11, 2: 1, 3: 2, 5: 48, 6: 32, 7: 32, 9: 40, 10: 9, 11: 8, 12: 32}
    for event in events:
        if event[0] == "raw":
            w.append_bits(event[1], event[2])
            continue
        kind, value = event
        w.append_bits(1, 1)
        w.append_bits(kind, 4)
        if kind in (4, 8):
            w.append_packed_uint(value)
        elif kind in widths:
            w.append_bits(value, widths[kind])
    if terminal is not None:
        w.append_bits(terminal, 1)


# ---------------------------------------------------------------- primitives


def section_primitives():
    from range_client_input import read_bits, read_packed_uint
    from range_client_rpc import _Reader, _object_reference, _packed_header_field

    rng = random.Random(606)
    out = {"read_bits": [], "packed": [], "header_packed": [], "serialize_int": [], "fstring": [],
           "object_reference": []}

    for _ in range(60):
        data = random_bytes(rng, rng.randrange(1, 12) * 8, clear_tail=False)
        offset = rng.randrange(0, len(data) * 8 + 4)
        width = rng.randrange(0, 65)
        out["read_bits"].append({"hex": data.hex(), "offset": offset, "width": width,
                                 **attempt(lambda: {"value": read_bits(data, offset, width)})})

    packed_inputs = []
    for value in (0, 1, 63, 64, 127, 128, 255, 525, 863, 16383, 4294967295, 2 ** 21, 2 ** 28 - 1, 2 ** 28):
        for offset in (0, 3):
            w = writer()
            w.append_bits(0, offset)
            w.append_packed_uint(value)
            w.append_bits(rng.getrandbits(5), 5)
            data, bits = pair(w)
            packed_inputs.append((data, offset, bits))
    for text, limit in (("ffffffff1e", 40), ("fffffffffe", 40), ("ffffffff1f", 40), ("ffffffffff00", 48),
                        ("01", 8), ("01", 16), ("0102", 15), ("0102", 16), ("ff", 7), ("80", 8)):
        packed_inputs.append((bytes.fromhex(text), 0, limit))
    for _ in range(80):
        data = bytes(rng.choice((0xFF, 0x01, 0x81, 0x7F, rng.randrange(256))) for _ in range(rng.randrange(1, 8)))
        offset = rng.randrange(0, 9)
        limit = rng.randrange(offset, len(data) * 8 + 9)
        packed_inputs.append((data, offset, limit))
    for data, offset, limit in packed_inputs:
        base = {"hex": data.hex(), "offset": offset, "limit": limit}

        def packed():
            value, end = read_packed_uint(data, offset, limit)
            return {"value": value, "end": end}

        def header():
            reader = _Reader(data, offset, limit)
            value = _packed_header_field(reader)
            return {"value": value, "end": reader.position}

        out["packed"].append({**base, **attempt(packed)})
        out["header_packed"].append({**base, **attempt(header)})

    sint_inputs = []
    for value, maximum in ((156, 179), (157, 179), (161, 179), (170, 179), (172, 179), (176, 179), (177, 179),
                           (70, 179), (83, 179), (92, 179), (154, 179), (0, 179), (1, 179), (178, 179),
                           (51, 53), (25, 53), (41, 53), (52, 53), (0, 2), (1, 2), (0, 1), (3, 5), (4, 5),
                           (65535, 65536), (2 ** 32 - 1, 2 ** 32)):
        w = writer()
        w.append_bits(1, 2)
        w.append_serialize_int(value, maximum)
        w.append_bits(rng.getrandbits(7), 7)
        data, bits = pair(w)
        sint_inputs.append((data, 2, bits, maximum))
        sint_inputs.append((data, 2, 4, maximum))
    for maximum in (0, 1, 2, 3, 5, 53, 54, 179, 180, 256, 257, 65536, 2 ** 32, 2 ** 32 + 1, 2 ** 40):
        for _ in range(3):
            data = random_bytes(rng, 64, clear_tail=False)
            offset = rng.randrange(0, 8)
            sint_inputs.append((data, offset, rng.randrange(offset, 65), maximum))
    for data, offset, limit, maximum in sint_inputs:
        def sint():
            reader = _Reader(data, offset, limit)
            value = reader.serialize_int(maximum)
            return {"value": value, "end": reader.position}

        out["serialize_int"].append({"hex": data.hex(), "offset": offset, "limit": limit, "maximum": maximum,
                                     **attempt(sint)})

    fstrings = [(bytes.fromhex("0400000041626300"), 0, 64), (bytes.fromhex("fdffffff480069000000"), 0, 80),
                (bytes.fromhex("00000000"), 0, 32)]
    for name in ("Abc", "a\0b", ("raw", 6, b"a\0b\0\0\0"), ("raw", 3, b"\0\0\0"), "x" * 1023,
                 ("raw", 1025, b""), ("raw", 2, b"\xc3\0"), ("raw", 1, b"\x7f"), ("utf16", "\u03a9\u00e9"),
                 ("utf16", "\U0001f600z"), ("raw", 0xFFFFFFFE, b"\x00\xd8\x41\x00"),
                 ("raw", 0xFFFFFFFE, b"\x00\xdc\x41\x00"), ("raw", 0xFFFFFFFD, b"\x41\x00\x00\x00\x00\x00"),
                 ("raw", 0xFFFFFFFF, b"\x41\x00"), ("raw", 0xFFFFFFFF, b"\x00\xd8"), ("raw", 0x80000000, b"ab"),
                 ("utf16", "a\0b"), "/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK", ("raw", 4, b"ab")):
        w = writer()
        w.append_bits(5, 3)
        put_fstring(w, name)
        data, bits = pair(w)
        fstrings.append((data, 3, bits))
        if bits > 48:
            fstrings.append((data, 3, bits - 9))
            fstrings.append((data, 3, len(data) * 8 + 24))
    for data, offset, limit in fstrings:
        def fstring():
            reader = _Reader(data, offset, limit)
            value = reader.fstring()
            return {"value": value, "end": reader.position}

        out["fstring"].append({"hex": data.hex(), "offset": offset, "limit": limit, **attempt(fstring)})

    warden_package = ("path", 5, None, "/Game/Equippables/Guns/Rifles/BattleRifle/BattleRifle", 0)
    refs = [None, 77, 2 ** 32 - 1, ("bytes", [0xFF, 0xFF, 0xFF, 0xFF, 0xFE]), ("bytes", [0xFF] * 5 + [0x00]),
            ("default", 0x00), ("default", 0x04), ("default", 0xFE),
            ("path", 1, None, "Name", 0), ("path", 5, warden_package, "BattleRifle_C", 0xB77E876F),
            ("path", 1, 525, "AssaultRifle_AK_C", 0), ("path", 0xFF, 525, "X_C", 0x12345678),
            ("path", 1, ("default", 0), "X", 0), ("path", 1, ("path", 1, None, "", 0), "X", 0),
            ("path", 1, None, "", 0), ("path", 1, ("path", 1, None, "A", 0), "", 0),
            ("path", 1, None, ("utf16", "W\u00e9"), 0), ("path", 1, None, ("raw", 2, b"\xffa"), 0),
            ("path", 1, None, ("raw", 2000, b""), 0), ("path", 1, None, "Trail\0\0", 0),
            ("path", 1, ("bytes", [0xFF, 0xFF, 0xFF, 0xFF, 0xFE]), "Big_C", 0),
            chain(3), chain(8), chain(9), chain(12)]
    for ref in refs:
        w = writer()
        w.append_bits(3, 2)
        put_ref(w, ref)
        w.append_bits(rng.getrandbits(6), 6)
        data, bits = pair(w)
        for limit in sorted({bits, max(2, bits - 7), len(data) * 8 + 40}):
            def reference():
                reader = _Reader(data, 2, limit)
                value = _object_reference(reader)
                return {"value": value, "end": reader.position}

            out["object_reference"].append({"hex": data.hex(), "offset": 2, "limit": limit, **attempt(reference)})
    return out


# ---------------------------------------------------------------- content transform and blocks


def section_content():
    from range_client_input import decode_content_payload
    from range_server_bunch import encode_short_actor_rep_payload

    rng = random.Random(1306)
    out = {"spec": [], "random": [], "roundtrip": [], "errors": []}
    for bits, actor_map, wire in ((9, 4, "0800"), (31, 4, "bf63ff4f"), (32, 4, "5a2a2cd0"), (33, 10, "1a212c9300"),
                                  (64, 10, "eb565e03d857b905"), (65, 4, "eb565e2ed89a5f0501"),
                                  (97, 10, "eb565e82d82775052ce71ecf00"),
                                  (135, 4, "e9565e62d8fa75059de91279eaf20d1040")):
        out["spec"].append({"bits": bits, "map": actor_map, "wire": wire,
                            "plain": decode_content_payload(bytes.fromhex(wire), bits, actor_map).hex()})
    maps = (0, 2, 4, 10, 208, 525, 300100, 0x7FFFFFFF, 0xFFFFFFFE, 0xFFFFFFFF)
    sizes = list(range(1, 161)) + [191, 192, 193, 255, 256, 257, 300, 343, 431, 511, 512, 513, 739, 787,
                                   1000, 1023, 1024, 1025, 2047, 4096, 8191, 8192]
    for bits in sizes:
        for actor_map in rng.sample(maps, 2):
            wire = random_bytes(rng, bits, clear_tail=rng.random() < 0.7)
            out["random"].append({"bits": bits, "map": actor_map, "wire": wire.hex(),
                                  "plain": decode_content_payload(wire, bits, actor_map).hex()})
    # range_client_input tests: test_content_transform_roundtrip
    for bits in (9, 15, 31, 32, 33, 63, 64, 65, 95, 96, 97, 343, 431, 1023):
        plain = masked(bytes((index * 37 + 19) & 0xFF for index in range((bits + 7) // 8)), bits)
        wire = encode_short_actor_rep_payload(plain, bits, 0, 10)
        out["roundtrip"].append({"bits": bits, "map": 10, "plain": plain.hex(), "wire": wire.hex(),
                                 "decoded": decode_content_payload(wire, bits, 10).hex()})
    for bits, wire in ((0, b""), (8193, bytes(1025)), (9, b"\x01"), (9, b"\x01\x02\x03"), (16, b"\x01")):
        out["errors"].append({"bits": bits, "map": 4, "wire": wire.hex(),
                              **attempt(lambda: {"plain": decode_content_payload(wire, bits, 4).hex()})})
    return out


def section_blocks():
    from range_client_input import read_actor_content_block
    from range_server_bunch import encode_short_actor_rep_payload

    rng = random.Random(4)
    cases = []

    def add(name, data, start, end, actor_map=10, bit_count=None):
        cases.append({"name": name, "hex": data.hex(), "bit_count": len(data) * 8 if bit_count is None else bit_count,
                      "start": start, "end": end, "map": actor_map,
                      **attempt(lambda: block_dict(read_actor_content_block(data, start, end, actor_map_value=actor_map)))})

    def body(rep, actor, bits, actor_map, prefix=3, extra=5, count=None, packed_bytes=None):
        plain = random_bytes(rng, bits)
        wire = encode_short_actor_rep_payload(plain, bits, 0, actor_map) if bits >= 9 else random_bytes(rng, bits)
        w = writer()
        w.append_bits(rng.getrandbits(prefix), prefix)
        w.append_bits(rep, 1)
        w.append_bits(actor, 1)
        if packed_bytes is not None:
            for byte in packed_bytes:
                w.append_bits(byte, 8)
        else:
            w.append_packed_uint(bits if count is None else count)
        w.append_payload(wire, bits)
        w.append_bits(rng.getrandbits(extra), extra)
        data, total = pair(w)
        return data, prefix, total - extra

    for bits in (1, 7, 9, 31, 32, 64, 100, 431, 1963, 8192):
        for actor_map in (4, 10):
            data, start, end = body(0, 1, bits, actor_map)
            add(f"plain_{bits}_{actor_map}", data, start, end, actor_map)
            add(f"slack_{bits}_{actor_map}", data, start, end + 5, actor_map)
    data, start, end = body(1, 1, 64, 10)
    add("rep_layout", data, start, end)
    data, start, end = body(0, 0, 64, 10)
    add("subobject", data, start, end)
    add("start_at_end", data, end, end)
    add("end_past_data", data, start, len(data) * 8 + 1)
    add("header_truncated", data, start, start + 1)
    add("two_bits", data, start, start + 2)
    data, start, end = body(0, 1, 64, 10)
    add("packed_truncated", data, start, start + 2 + 7)
    add("content_overruns", data, start, end - 1)
    data, start, end = body(0, 1, 16, 10, packed_bytes=[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00])
    add("packed_six_bytes", data, start, end)
    data, start, end = body(0, 1, 16, 10, count=0)
    add("zero_bits", data, start, end)
    data, start, end = body(0, 1, 16, 10, count=8193)
    add("over_8192", data, start, end)
    # range_client_input tests: test_first_captured_channel8_packet
    packet = bytes.fromhex("200a2183ffffffff470f08080e7e1988917d93132198b5ea5aaea301ca0e"
                           "c955a30fd194da748ad20fe660abeeaf6430c159400b58796b0668f7ed82b6"
                           "d5e8838e9a0ab60733")
    add("test_first_captured_channel8_packet", packet, 104, 553)
    add("test_first_captured_channel8_packet_map4", packet, 104, 553, actor_map=4)
    return cases


# ---------------------------------------------------------------- input streams


def section_inputs():
    from range_client_input import (_parse_input_stream_at, decode_received_actor_inputs,
                                    scan_received_input_streams)
    from range_server_bunch import encode_short_actor_rep_payload

    rng = random.Random(8)
    out = {"scan": [], "parse_at": [], "actor_inputs": []}

    def scan(name, data, bits, max_ids=(None,)):
        for max_id in max_ids:
            out["scan"].append({"name": name, "hex": data.hex(), "bit_count": bits, "max_id": max_id,
                                **attempt(lambda: {"streams": [stream_dict(s) for s in
                                                               scan_received_input_streams(data, bits, max_id)]})})

    def stream_payload(first_id, events, prefix=13, suffix=16, **kw):
        w = writer()
        w.append_bits(0, prefix)
        stream_bits(w, first_id, events, **kw)
        w.append_bits(0, suffix)
        return pair(w)

    # test_synthetic_received_activation
    activation = stream_payload(42, [(1, (1 << 8) | (2 << 5) | (2 << 1) | 1), (7, 0xA5A55A5A)])
    scan("test_synthetic_received_activation", *activation, max_ids=(None, 43, 42, 41, 0, 0xFFFFFFFF))
    # test_incomplete_stream_not_acknowledged
    w = writer()
    stream_bits(w, 100, [(1, 5)], terminal=None)
    scan("test_incomplete_stream", *pair(w))
    # test_empty_or_unknown_stream_not_acknowledged
    for index, suffix in enumerate(((0, 1), (1, 1, 0, 32, 2, 8), (1, 1, 7, 32, 2, 8, 1, 1, 15, 4, 0, 1))):
        w = writer()
        w.append_bits(0xA3, 8)
        for at in range(0, len(suffix), 2):
            w.append_bits(suffix[at], suffix[at + 1])
        scan(f"test_empty_or_unknown_{index}", *pair(w))
    widths = {0: 37, 1: 11, 2: 1, 3: 2, 5: 48, 6: 32, 7: 32, 9: 40, 10: 9, 11: 8, 12: 32}
    for kind in range(16):
        value = rng.getrandbits(widths.get(kind, 0)) if kind in widths else (rng.getrandbits(12) if kind in (4, 8) else 0)
        scan(f"kind_{kind}", *stream_payload(rng.getrandbits(32) & 0x7FFFFFFF, [(kind, value)]))
    for index in range(40):
        events = []
        for _ in range(rng.randrange(1, 24)):
            kind = rng.choice((0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12))
            if kind in (4, 8):
                events.append((kind, rng.choice((0, 1, 204, 2024, 2 ** 20, 2 ** 32 - 1))))
            else:
                events.append((kind, rng.getrandbits(widths[kind])))
        first_id = min(0xFFFFFFFF, rng.choice((0, 7139, 10450, 2 ** 31, 0xFFFFFFFF - len(events) + 1,
                                               0xFFFFFFFF - len(events) + 2)))
        payload = stream_payload(first_id, events, prefix=rng.randrange(0, 40), suffix=rng.randrange(0, 30))
        last = (first_id + len(events) - 1) & 0xFFFFFFFF
        scan(f"random_{index}", *payload, max_ids=(None, last, max(0, last - 1)))
    scan("count_0", *stream_payload(5, [], count=0))
    scan("count_256", *stream_payload(5, [(2, 1)] * 256))
    scan("count_257", *stream_payload(5, [(2, 1)] * 257))
    scan("count_says_more", *stream_payload(5, [(2, 1)] * 3, count=4))
    scan("overflow_id", *stream_payload(0xFFFFFFFF, [(2, 1), (2, 0)]))
    scan("last_id_max", *stream_payload(0xFFFFFFFF, [(2, 1)]), max_ids=(None, 0xFFFFFFFE, 0xFFFFFFFF))
    scan("bad_continuation", *stream_payload(5, [(2, 1)], continuation=0))
    scan("event_missing_bit", *stream_payload(5, [("raw", 0b00100, 5), ("raw", 1, 1)]))
    scan("terminal_one", *stream_payload(5, [(2, 1)], terminal=1))
    scan("no_room_for_terminal", *stream_payload(5, [(7, 0x12345678)], suffix=0, terminal=None))
    guid_stream = writer()
    guid_stream.append_bits(0, 4)
    guid_stream.append_bits(0xA3, 8)
    guid_stream.append_bits(1, 1)
    guid_stream.append_bits(9, 32)
    guid_stream.append_packed_uint(2)
    guid_stream.append_bits(1, 1)
    guid_stream.append_bits(4, 4)
    for byte in (0xFF, 0xFF, 0xFF, 0xFF, 0xFE):
        guid_stream.append_bits(byte, 8)
    guid_stream.append_bits(1, 1)
    guid_stream.append_bits(8, 4)
    for byte in (0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00):
        guid_stream.append_bits(byte, 8)
    guid_stream.append_bits(0, 20)
    scan("guid_35_bit_then_overlong", *pair(guid_stream))
    back_to_back = writer()
    back_to_back.append_bits(0, 3)
    stream_bits(back_to_back, 7139, [(7, 1), (5, 2)])
    stream_bits(back_to_back, 7141, [(8, 2024)])
    back_to_back.append_bits(0, 30)
    scan("two_streams", *pair(back_to_back))
    # a kind 7 payload that starts with a marker byte and a set bit: a candidate inside the stream
    scan("marker_in_payload", *stream_payload(9, [(7, 0x1A3 | (0x55 << 9)), (6, 0xA3A3A3A3)]))
    for index in range(30):
        bits = rng.randrange(40, 700)
        value = rng.getrandbits(bits)
        for _ in range(rng.randrange(1, 6)):
            at = rng.randrange(0, max(1, bits - 8))
            value = (value & ~(0xFF << at)) | (0xA3 << at)
            if at + 8 < bits:
                value |= 1 << (at + 8)
        data = value.to_bytes((bits + 7) // 8, "little")
        scan(f"noise_{index}", data, bits)
    long_data, long_bits = stream_payload(77, [(2, 1)], prefix=0, suffix=0)
    scan("bit_count_past_data", long_data, len(long_data) * 8 + 9)

    def parse_at(data, bits, marker, max_id):
        stream = _parse_input_stream_at(data, bits, marker, max_id)
        return {"stream": None if stream is None else stream_dict(stream)}

    # a stream read that runs off the bytes (bit_count larger than the data)
    short = writer()
    stream_bits(short, 1, [(5, 1)] * 3)
    short_data, short_bits = pair(short)
    out["scan"].append({"name": "past_data", "hex": short_data[:7].hex(), "bit_count": short_bits, "max_id": None,
                        "skip_scan": True})
    for index, entry in enumerate(out["scan"]):
        if entry["max_id"] is not None or ("streams" not in entry and "skip_scan" not in entry):
            continue
        data, bits = bytes.fromhex(entry["hex"]), entry["bit_count"]
        found = {s["marker_bit"] for s in entry.get("streams", [])}
        for marker in sorted(found | {0, 1, max(0, bits - 49), max(0, bits - 48)}):
            for max_id in ((None, 0) if marker in found else (None,)):
                out["parse_at"].append({"scan": index, "marker": marker, "max_id": max_id,
                                        **attempt(lambda: parse_at(data, bits, marker, max_id))})

    def actor(name, data, start, end, max_ids=(None,)):
        for max_id in max_ids:
            out["actor_inputs"].append({"name": name, "hex": data.hex(), "start": start, "end": end, "max_id": max_id,
                                        **attempt(lambda: {"streams": [stream_dict(s) for s in decode_received_actor_inputs(
                                            data, start, end, max_id)]})})

    # test_synthetic_received_activation, as an actor body
    plain, bits = activation
    wire = encode_short_actor_rep_payload(plain, bits, 0, 10)
    w = writer()
    w.append_bits(0, 1)
    w.append_bits(1, 1)
    w.append_packed_uint(bits)
    w.append_payload(wire, bits)
    body, body_bits = pair(w)
    actor("test_synthetic_received_activation", body, 0, body_bits, max_ids=(None, 43, 41))
    w = writer()
    w.append_bits(1, 1)
    w.append_bits(1, 1)
    w.append_packed_uint(bits)
    w.append_payload(wire, bits)
    rep_body, rep_bits = pair(w)
    actor("rep_layout", rep_body, 0, rep_bits)
    # test_first_captured_channel8_packet and test_captured_slot_two_event
    actor("test_first_captured_channel8_packet", bytes.fromhex(
        "200a2183ffffffff470f08080e7e1988917d93132198b5ea5aaea301ca0ec955a30fd194da748ad20fe660abeeaf6430c15940"
        "0b58796b0668f7ed82b6d5e8838e9a0ab60733"), 104, 553, max_ids=(None, 1, 0))
    actor("test_captured_slot_two_event", bytes.fromhex(
        "c034319cffffffff9f0908c80f3e1b88857d13aa6f45a06ca876d21ec60eea4dd5b7a234a902a3512f65d00a416202b33288d830"
        "a45bebdb8d12e628f6a9b5d12b0ac07e681c8c75e1c5830933"), 104, 609, max_ids=(None, 372, 371))
    actor("bad_bounds", body, body_bits, body_bits)
    return out


# ---------------------------------------------------------------- whole packets


def packet_case(name, data, bit_count):
    from range_client_input import decode_received_actor_inputs, read_actor_content_block
    from range_client_rpc import (_Reader, decode_controller_rpc, decode_purchase_requests,
                                  decode_range_settings_requests, decode_range_settings_rpc, iter_bunches)
    from range_udp_handshake import inspect_control_bunch

    case = {"name": name, "hex": data.hex(), "bit_count": bit_count}
    try:
        bunches = iter_bunches(data, bit_count)
    except ValueError as error:
        case["walk_error"] = err(error)
        bunches = []
    else:
        case["bunches"] = []
    for bunch in bunches:
        entry = bunch_dict(bunch)
        actor_map = 4 if bunch.channel_index == 5 else 10
        entry["block_map"] = actor_map
        try:
            block = read_actor_content_block(data, bunch.body_start_bit, bunch.end_bit, actor_map_value=actor_map)
        except ValueError as error:
            entry["block"] = err(error)
        else:
            entry["block"] = block_dict(block)
            if not block.has_rep_layout:
                field_max = 179 if bunch.channel_index == 5 else 53

                def header():
                    reader = _Reader(block.payload, 0, block.bit_count)
                    field = reader.serialize_int(field_max)
                    size = reader.packed()
                    return {"field": field, "payload_bits": size, "params_start": reader.position}

                entry["rpc_header"] = {"field_max": field_max, **attempt(header)}
        entry["purchase"] = attempt(lambda: optional_dict(decode_controller_rpc(data, bunch)))
        entry["settings"] = attempt(lambda: optional_dict(decode_range_settings_rpc(data, bunch)))
        entry["inputs"] = attempt(lambda: [stream_dict(s) for s in decode_received_actor_inputs(
            data, bunch.body_start_bit, bunch.end_bit)])
        case["bunches"].append(entry)
    case["purchases"] = [dataclasses.asdict(r) for r in decode_purchase_requests(data, bit_count)]
    case["settings"] = [dataclasses.asdict(r) for r in decode_range_settings_requests(data, bit_count)]
    # the input ack reads the first bunch with the packet inspector's bounds
    try:
        info = inspect_control_bunch(data, bit_count)
    except ValueError as error:
        info = {"inspect_error": str(error)}
    # the first-bunch header fields python's Bunch does not keep, for the BunchView extras.
    # false / None values are left out to keep the file small (the rust test reads them as such)
    keep = ("inspect_error", "bunch_start_bit", "bunch_is_close", "bunch_is_open", "bunch_replication_paused",
            "has_exports", "has_must_map", "header_flag_4", "hardcoded_name", "channel_name_index",
            "bunch_body_start_bit", "bunch_end_bit")
    case["inspect"] = {key: info[key] for key in keep if info.get(key) not in (None, False)}
    if "bunch_body_start_bit" in info:
        bounds = (info["bunch_body_start_bit"], info["bunch_end_bit"])
        # same bounds as the walk's first bunch: its "inputs" already holds this result
        if not bunches or (bunches[0].body_start_bit, bunches[0].end_bit) != bounds:
            case["first_inputs"] = attempt(lambda: [stream_dict(s) for s in decode_received_actor_inputs(
                data, *bounds)])
    return case


def load_captures():
    """every (decoded_packet_hex, payload_bits) in the responder logs, plus a
    (file stem, elapsed) index for the named captures."""
    project = Path(os.environ.get("ARIES_PROJECT_ROOT", r"C:/Users/Cayden/Desktop/valorant proj"))
    runtime = project / "local_backend" / "runtime"
    unique, named = {}, {}

    def add(event, path):
        data, bits = event.get("decoded_packet_hex"), event.get("payload_bits")
        if data is None or bits is None:
            return
        unique.setdefault((data, bits), None)
        if event.get("elapsed_seconds") is not None:
            named.setdefault((Path(path).name.split("_phase")[0].split(".")[0], round(event["elapsed_seconds"], 3)),
                             (data, bits))

    for path in sorted(glob.glob(str(runtime / "*"))):
        if not os.path.isfile(path) or not path.endswith((".out", ".json", ".log")):
            continue
        if path.endswith(".json"):
            try:
                with open(path, encoding="utf-8", errors="replace") as handle:
                    document = json.load(handle)
            except ValueError:
                document = None
            if isinstance(document, dict) and isinstance(document.get("events"), list):
                for event in document["events"]:
                    if isinstance(event, dict):
                        add(event, path)
                continue
        with open(path, encoding="utf-8", errors="replace") as handle:
            for line in handle:
                if "decoded_packet_hex" not in line:
                    continue
                try:
                    event, _ = json.JSONDecoder().raw_decode(line[line.find("{"):])
                except ValueError:
                    continue
                if isinstance(event, dict):
                    add(event, path)
    return sorted(unique), named


# spec 06 section 13.4 / 13.7: named real captures (log stem, elapsed seconds)
NAMED_CAPTURES = (
    ("acr", "responder_28576", 379.282), ("settings", "responder_24252", 148.422),
    ("ch5_field92", "responder_24252", 146.485), ("ch5_field70", "range_launch_23872", 100.859),
    ("ch5_field154", "responder_13096", 621.218), ("ch5_unreliable_field83", "range_launch_23872", 97.297),
    ("ch5_subobject", "responder_31280", 920.781), ("ch8_plus_ch4", "range_capture_25768", 56.125),
    ("ch8_overrun", "responder_31504", 140.094), ("ch8_subobject", "responder_13096", 111.734),
    ("multi_stream", "responder_28576", 126.563), ("slot_two_object", "range_launch_21904", 182.891),
)


def curated(captures):
    """every capture with a notable decode (a purchase, a settings rpc, any decode error, a partial
    bunch, an odd channel, 3+ bunches, 2+ streams), plus seeded samples of the common shapes."""
    from range_client_input import decode_received_actor_inputs
    from range_client_rpc import (decode_controller_rpc, decode_range_settings_rpc, iter_bunches)

    notable, groups = [], {"ch5_unreliable": [], "ch5_then_ch8": [], "with_stream": [], "plain": []}
    for data_hex, bits in captures:
        data = bytes.fromhex(data_hex)
        try:
            bunches = iter_bunches(data, bits)
            results = [(decode_controller_rpc(data, b), decode_range_settings_rpc(data, b)) for b in bunches]
            streams = [decode_received_actor_inputs(data, b.body_start_bit, b.end_bit) for b in bunches]
        except ValueError:
            notable.append((data_hex, bits))
            continue
        shape = [(b.channel_index, b.is_reliable, b.is_partial) for b in bunches]
        if (any(p or s for p, s in results) or any(len(found) > 1 for found in streams) or len(bunches) > 2
                or any(entry not in ((5, False, False), (5, True, False), (8, False, False)) for entry in shape)):
            notable.append((data_hex, bits))
        elif shape == [(5, False, False)]:
            groups["ch5_unreliable"].append((data_hex, bits))
        elif len(shape) > 1 or shape[0] != (8, False, False):
            groups["ch5_then_ch8"].append((data_hex, bits))
        elif streams[0]:
            groups["with_stream"].append((data_hex, bits))
        else:
            groups["plain"].append((data_hex, bits))
    rng = random.Random(1306)
    picked = list(notable)
    for name, count in (("ch5_unreliable", 20), ("ch5_then_ch8", 40), ("with_stream", 80), ("plain", 80)):
        picked += rng.sample(groups[name], min(count, len(groups[name])))
    return picked


def synthetic_packets():
    """packets built with the python writers: (name, data, bits)."""
    rng = random.Random(482)
    out = []

    def purchase(field, params, sequence=7, map_value=4, **header):
        return actor_block(rpc_payload(field, params), map_value), {"sequence": sequence, **header}

    def add(name, bunches, **packet):
        out.append((name, *client_packet(bunches, **packet)))

    # spec 06 13.5 / 13.6 synthetic packets (checked against the spec hex below)
    add("spec_repeat_purchase", [purchase(156, purchase_params(("path", 1, 525, "AssaultRifle_AK_C", 0), 0, 0), 1290)])
    add("spec_sell", [purchase(161, purchase_params(("absent",), 11, None), 7)])
    add("spec_settings_three", [
        (actor_block(rpc_payload(174, writer()), 4), {"sequence": 4094}),
        (actor_block(rpc_payload(177, settings_params([(1, 1), i32(-2), (1, 1), (0, 1)])), 4), {"sequence": 4095}),
        (actor_block(rpc_payload(172, object_params(("path", 1, None, "/Game/X/Mod", 0))), 4), {"sequence": 0})])
    expected = {"spec_repeat_purchase": "20039001ffffffff014005502808c17601d89f6590129a18bf5b69464768a82caf2bab3edd4fcd9cb385c0b7370ee83e",
                "spec_sell": "20039001ffffffff014005380025401b5e685e05",
                "spec_settings_three": "20039001ffffffff014005f07f1a40906d582ac0ffeb019a21691fcf63c6920a000086816302633f618fc509d47ee2369f8080b2fbd378420a98812d03"}
    for name, data, bits in out:
        if data.hex() != expected[name]:
            raise ValueError(f"synthetic {name} does not rebuild the spec 06 vector")

    warden_package = ("path", 5, None, "/Game/Equippables/Guns/Rifles/BattleRifle/BattleRifle", 0)
    items = {
        "warden_path": ("path", 5, warden_package, "BattleRifle_C", 0xB77E876F),
        "guid_outer": ("path", 1, 525, "AssaultRifle_AK_C", 0),
        "mapped": 77, "null_guid": None, "default_no_path": ("default", 0), "absent": ("absent",),
        "deep_chain": chain(3), "too_deep": chain(9), "empty_outer": ("path", 1, ("path", 1, None, "", 0), "X_C", 0),
        "empty_name": ("path", 1, None, "", 0), "utf16_name": ("path", 1, None, ("utf16", "B\u00e9_C"), 0),
        "non_ascii": ("path", 1, None, ("raw", 3, b"\xc3\xa9\0"), 0), "long_name": ("path", 1, None, ("raw", 1025, b""), 0),
        "big_guid": ("bytes", [0xFF, 0xFF, 0xFF, 0xFF, 0xFE]), "overlong_guid": ("bytes", [0xFF] * 6),
    }
    for name, item in items.items():
        add(f"purchase_{name}", [purchase(156, purchase_params(item, rng.choice((None, 0, 11)), rng.choice((None, 0, 5))),
                                           rng.randrange(4096), name=102)])
    add("purchase_slot_type", [purchase(156, purchase_params(items["guid_outer"], 255, 7), 12, name=102)])
    add("remove", [purchase(157, purchase_params(items["warden_path"], 0, None), 13, name=102)])
    add("sell_full", [purchase(161, purchase_params(items["mapped"], 1, 2), 14, name=102)])
    add("field_160", [purchase(160, purchase_params(items["mapped"], 1, 2), 15, name=102)])
    add("field_170", [purchase(170, writer(), 16, name=102)])
    add("field_0", [purchase(0, purchase_params(items["mapped"]), 17, name=102)])
    add("field_178", [purchase(178, purchase_params(items["mapped"]), 18, name=102)])
    add("purchase_trailing_params", [purchase(156, purchase_params(items["mapped"], 3, 1, extra=(0x2A, 7)), 19, name=102)])
    add("purchase_params_short", [(actor_block(rpc_payload(156, purchase_params(items["guid_outer"], 3, 1), size=20), 4),
                                   {"sequence": 20, "name": 102})])
    add("purchase_params_long", [(actor_block(rpc_payload(156, purchase_params(items["mapped"], None, None), size=200), 4),
                                  {"sequence": 21, "name": 102})])
    add("purchase_rep_layout", [(actor_block(rpc_payload(156, purchase_params(items["mapped"])), 4, rep=1),
                                 {"sequence": 22, "name": 102})])
    add("purchase_subobject", [(actor_block(rpc_payload(156, purchase_params(items["mapped"])), 4, actor=0),
                                {"sequence": 23, "name": 102})])
    add("purchase_unreliable", [purchase(156, purchase_params(items["mapped"]), reliable=False)])
    add("purchase_partial", [purchase(156, purchase_params(items["mapped"]), 24, name=102, partial=True, partial_bits=2)])
    add("purchase_ch6", [purchase(156, purchase_params(items["mapped"]), 25, name=102, channel=6)])
    add("purchase_wrong_map", [purchase(156, purchase_params(items["warden_path"]), 26, map_value=10, name=102)])
    add("purchase_control_bunch", [purchase(156, purchase_params(items["guid_outer"]), 27, control=True, name=102)])
    add("purchase_name_255", [purchase(156, purchase_params(items["guid_outer"]), 28, name=255, exports=True, must_map=True,
                                        flag4=True, paused=True)])
    add("purchase_no_packet_info", [purchase(156, purchase_params(items["guid_outer"]), 29, name=102)], info=None, flag75=1)
    add("purchase_packet_info_set", [purchase(156, purchase_params(items["guid_outer"]), 30, name=102)], info=609, flag75=1)
    add("purchase_twice_same_sequence", [purchase(156, purchase_params(items["guid_outer"]), 31, name=102),
                                         purchase(156, purchase_params(items["warden_path"]), 31, name=102)])
    add("purchase_three", [purchase(156, purchase_params(items["guid_outer"]), 4095, name=102),
                           purchase(161, purchase_params(("absent",), 1, None), 0, name=102),
                           purchase(156, purchase_params(items["mapped"]), 1, name=102)])
    add("purchase_after_bad", [purchase(156, purchase_params(items["too_deep"]), 32, name=102),
                               purchase(156, purchase_params(items["guid_outer"]), 33, name=102)])

    def settings(field, params, sequence, size=None, **header):
        return actor_block(rpc_payload(field, params, size=size), 4), {"sequence": sequence, "name": 102, **header}

    full_176 = [(1, 1), (1, 1), (0, 1), (1, 1), i32(3), i32(-1), i32(2 ** 31 - 1), (1, 1), i32(-(2 ** 31))]
    add("settings_176_full", [settings(176, settings_params(full_176), 40)])
    add("settings_176_absent", [settings(176, settings_params([(0, 1)]), 41)])
    add("settings_176_extra_bit", [settings(176, settings_params(full_176 + [(0, 1)]), 42)])
    add("settings_176_truncated", [settings(176, settings_params(full_176[:5]), 43)])
    add("settings_176_absent_extra", [settings(176, settings_params([(0, 1), (0, 3)]), 44)])
    add("settings_177_present", [settings(177, settings_params([(1, 1), i32(7), (0, 1), (1, 1)]), 45)])
    add("settings_177_absent", [settings(177, settings_params([(0, 1), (1, 1), (1, 1)]), 46)])
    add("settings_177_extra", [settings(177, settings_params([(0, 1), (1, 1), (1, 1), (1, 1)]), 47)])
    for field in (172, 173):
        for ref_name, ref in (("path", ("path", 1, None, "/Game/X/Mod", 0)), ("mapped", 9), ("null", None),
                              ("guid_outer", items["guid_outer"])):
            add(f"settings_{field}_{ref_name}", [settings(field, object_params(ref), 48)])
        add(f"settings_{field}_absent", [settings(field, settings_params([(0, 1)]), 49)])
    add("settings_174", [settings(174, writer(), 50)])
    add("settings_175", [settings(175, writer(), 51)])
    add("settings_175_with_params", [settings(175, settings_params([(1, 1)]), 52)])
    add("settings_174_sized_but_empty", [settings(174, writer(), 53, size=0)])
    add("settings_unreliable", [settings(176, settings_params(full_176), 54, reliable=False)])
    add("settings_and_purchase", [settings(176, settings_params(full_176), 55),
                                  purchase(156, purchase_params(items["guid_outer"]), 56, name=102),
                                  settings(177, settings_params([(0, 1), (0, 1), (1, 1)]), 57)])

    # walk edges
    movement = actor_block(rpc_payload(51, settings_params([(rng.getrandbits(30), 30)] * 4), field_max=53), 10)
    add("close_stops_walk", [purchase(156, purchase_params(items["mapped"]), 60, name=102),
                             ((b"", 0), {"control": True, "close": True}),
                             purchase(156, purchase_params(items["mapped"]), 61, name=102)])
    add("close_first", [((b"", 0), {"control": True, "close": True})], tail=(0xFFFF, 20))
    add("open_control", [purchase(156, purchase_params(items["mapped"]), 62, control=True, open_=True, name=255)])
    add("partial_bits", [(movement, {"reliable": False, "channel": 8, "partial": True, "partial_bits": 3}),
                         purchase(156, purchase_params(items["mapped"]), 63, name=102)])
    add("exceeds_packet", [purchase(156, purchase_params(items["mapped"]), 64, name=102),
                           (movement, {"reliable": False, "channel": 8, "size": movement[1] + 40})])
    add("ends_at_packet_end", [(movement, {"reliable": False, "channel": 8})], terminator=False)
    add("channel_six_bytes", [(movement, {"reliable": False, "channel_bytes": [0xFF] * 5 + [0x00]})])
    add("channel_35_bit", [(movement, {"reliable": False, "channel_bytes": [0xFF] * 4 + [0x7E]})])
    add("name_six_bytes", [(movement, {"sequence": 5, "channel": 8, "name_bytes": [0x81] * 6})])
    add("empty_body", [((b"", 0), {"sequence": 70, "name": 102}),
                       purchase(156, purchase_params(items["mapped"]), 71, name=102)])
    add("tail_exactly_16", [purchase(156, purchase_params(items["mapped"]), 72, name=102)], tail=(0, 15))
    add("tail_17", [purchase(156, purchase_params(items["mapped"]), 73, name=102)], tail=(0, 16))
    add("ch8_movement_then_purchase", [(movement, {"reliable": False, "channel": 8}),
                                       purchase(156, purchase_params(items["guid_outer"]), 74, name=102)])
    for index in range(6):
        events = [(rng.choice((1, 4, 5, 7)), 0) for _ in range(rng.randrange(1, 5))]
        events = [(kind, rng.getrandbits({1: 11, 5: 48, 7: 32}[kind]) if kind != 4 else rng.randrange(300)) for kind, _ in events]
        payload = writer()
        payload.append_serialize_int(51, 53)
        params = writer()
        params.append_bits(rng.getrandbits(40), 40)
        stream_bits(params, rng.randrange(20000), events)
        params.append_bits(rng.getrandbits(20), 20)
        params_data, params_bits = pair(params)
        payload.append_packed_uint(params_bits)
        payload.append_payload(params_data, params_bits)
        block = actor_block(pair(payload), 10)
        add(f"ch8_stream_{index}", [(block, {"reliable": False, "channel": 8})] +
            ([purchase(156, purchase_params(items["guid_outer"]), 80 + index, name=102)] if index % 2 else []))
    header, header_bits = client_packet([], terminator=False)
    out.append(("too_short_63", header[:8], 63))
    out.append(("too_short_64", header[:8], 64))
    out.append(("info_cut_65", header[:9], 65))
    out.append(("header_76", header, header_bits))
    out.append(("header_77", *client_packet([])))
    out.append(("garbage_zeros_400", bytes(64), 400))
    out.append(("garbage_ff_400", b"\xff" * 50, 400))
    return out


def mutants(bases, count_per_base, seed):
    """bit flips past the packet header and truncations of real packets."""
    rng = random.Random(seed)
    out = []
    for name, data, bits in bases:
        for index in range(count_per_base):
            mode = index % 3
            if mode == 2 and bits > 80:
                cut = rng.randrange(64, bits)
                out.append((f"{name}_cut{cut}", data[:(cut + 7) // 8], cut))
                continue
            flipped = bytearray(data)
            for _ in range(1 if mode == 0 else 3):
                at = rng.randrange(64, bits)
                flipped[at // 8] ^= 1 << (at % 8)
            out.append((f"{name}_flip{index}", bytes(flipped), bits))
    return out


def section_packets():
    project = Path(os.environ.get("ARIES_PROJECT_ROOT", r"C:/Users/Cayden/Desktop/valorant proj"))
    warden = json.loads((project / "local_backend" / "fixtures" / "server_purchase_warden_packet.json").read_text())
    captures, named = load_captures()
    packets = [("warden_fixture", bytes.fromhex(warden["decoded_packet_hex"]), warden["payload_bits"])]
    # spec 06 13.3 control bunches
    for name, data_hex, bits in (
            ("hello", "0000000000000000ff3701a02cfe0b8001000000000000000000000080", 232),
            ("login", "0000000000000000ff3701a02cfe0b8081020000000000000000000080", 232),
            ("join", "0000000000000000ff3701a02cfe0b8081040000000000000000000080", 232),
            ("ack_only", "d036676500000000059a", 77)):
        packets.append((name, bytes.fromhex(data_hex), bits))
    for name, stem, elapsed in NAMED_CAPTURES:
        data_hex, bits = named[(stem, elapsed)]
        packets.append((name, bytes.fromhex(data_hex), bits))
    synthetic = synthetic_packets()
    packets += synthetic
    small_named = [p for p in packets[:len(packets) - len(synthetic)] if p[2] <= 2000]
    packets += mutants(small_named + [p for p in synthetic if p[2] > 77], 3, 9)
    if os.environ.get("ARIES_DECODE_ALL_CAPTURES") == "1":
        chosen = captures
    else:
        chosen = curated(captures)
        rng = random.Random(77)
        packets += mutants([(f"capture{i}", bytes.fromhex(h), b) for i, (h, b) in
                            enumerate(rng.sample([c for c in chosen if c[1] <= 1600], 24))], 3, 10)
    packets += [(f"capture_{index}", bytes.fromhex(h), b) for index, (h, b) in enumerate(chosen)]
    return [packet_case(name, data, bits) for name, data, bits in packets]


def section_datagrams():
    """spec 06 13.2: whole client datagrams (key 00..1f) through decode_client_ares_packet, so the
    rust test can run net's decoder, restore the trailer bits and decode the same packet."""
    from range_client_rpc import decode_purchase_requests
    from range_udp_handshake import decode_client_ares_packet

    key = bytes(range(32))
    out = []
    for name, datagram_hex in (
            ("ack_only", "000000000000000018ff8e0f9b7566d7f672617f448a27d4c8fd650e"),
            ("warden", "00000000000000000c9e158d44cd3889f9ff73a199862cc0541a4f91d73732ab6cc3e0313f85a881763fd110"
                       "979137bd03e34deb43234b9cb5dd009326c0ae866869ade316e28b6b3f75d6f659477c7ec90b3fd9e86a56a7ade6"
                       "fb8221b0007c1320728ae198bf776034d6b4b461c090b342710e31d80543df48ba6a244b6dbe03")):
        datagram = bytes.fromhex(datagram_hex)
        for variant in ("clean", "flip20"):
            data = datagram if variant == "clean" else datagram[:20] + bytes([datagram[20] ^ 1]) + datagram[21:]
            decoded = decode_client_ares_packet(data, key)
            case = {"name": f"{name}_{variant}", "key": key.hex(), "datagram": data.hex(), "decoded": None}
            if decoded is not None:
                packet, bits, counter, stateless = decoded
                case.update({"decoded": packet.hex(), "bit_count": bits, "counter": counter,
                             "stateless_header": stateless,
                             "purchases": [dataclasses.asdict(r) for r in decode_purchase_requests(packet, bits)]})
            out.append(case)
    return out


SECTIONS = {"decode_primitives": section_primitives, "decode_content": section_content,
            "decode_blocks": section_blocks, "decode_inputs": section_inputs, "decode_packets": section_packets,
            "decode_datagrams": section_datagrams}
