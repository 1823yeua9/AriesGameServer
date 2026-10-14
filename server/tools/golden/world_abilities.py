"""spec 08 sections for the world crate (I3): abilities, gameplay-tag net
index, bots, cloudburst projectiles, teams.

every expected value comes from calling the python offline:
- builders and pure helpers are called directly with production args and
  edge cases;
- gameplay_tag_net_index.predict_net_indices runs unmodified against a fake
  memory image (its reads are patched, the algorithm is not);
- the per-packet systems (bot tick, bot_shots, cloudburst_state,
  tick_projectiles, advance_ability_job) and the inline team bodies run the
  responder's own source, cut out of range_udp_handshake.py with ast and
  exec'd against fake readers and a recording send_reliable.

ruh = range_udp_handshake.py, rb = range_bots.py, rp = range_projectiles.py,
ra = range_abilities.py, gtni = gameplay_tag_net_index.py.
"""

import ast
import math
import random
import struct
import textwrap
from pathlib import Path


def hb(pair):
    body, bits = pair
    return {"hex": body.hex(), "bit_count": bits}


def attempt(fn, *args, **kwargs):
    """{"hex", "bit_count"} or {"error": <python exception name>}."""
    try:
        return hb(fn(*args, **kwargs))
    except Exception as error:  # the guard itself is the fixture
        return {"error": type(error).__name__}


def f64_bits(value):
    return struct.unpack("<Q", struct.pack("<d", value))[0]


def row(*values):
    """one compact fixture line: floats as repr (exact round trip), None as '-'."""
    def text(value):
        if value is None:
            return "-"
        if isinstance(value, bool):
            return "1" if value else "0"
        if isinstance(value, float):
            return repr(value)
        return str(value)
    return " ".join(text(v) for v in values)


def bits_row(values):
    """f64 values as 16-hex-digit bit patterns (keeps -0.0, inf and nan exact)."""
    return " ".join(f"{f64_bits(v):016x}" for v in values)


def vec(values):
    return [float(v) for v in values]


# ---------------------------------------------------------------- source cuts

_ruh_cache = {}


def ruh_source():
    if "text" not in _ruh_cache:
        import range_udp_handshake
        path = Path(range_udp_handshake.__file__).resolve()
        text = path.read_text(encoding="utf-8")
        _ruh_cache["text"] = text
        _ruh_cache["tree"] = ast.parse(text)
    return _ruh_cache["text"], _ruh_cache["tree"]


def nested_function(name, namespace):
    """compile run_probe's nested def `name` into namespace (its free variables
    resolve as globals there) and return it."""
    text, tree = ruh_source()
    for node in ast.walk(tree):
        if isinstance(node, ast.FunctionDef) and node.name == name:
            exec(compile(textwrap.dedent(ast.get_source_segment(text, node)), f"ruh:{node.lineno}", "exec"), namespace)
            return namespace[name], node.lineno
    raise LookupError(name)


def if_statement(test_fragment):
    """the whole (unique) `if` statement whose test contains test_fragment, gate
    included, as one dedented code object, plus its line."""
    text, tree = ruh_source()
    found = [node for node in ast.walk(tree)
             if isinstance(node, ast.If) and test_fragment in ast.get_source_segment(text, node.test)]
    if len(found) != 1:
        raise LookupError(f"{test_fragment!r}: {len(found)} matches")
    node = found[0]
    lines = text.splitlines()[node.lineno - 1:node.end_lineno]
    return compile(textwrap.dedent("\n".join(lines)), f"ruh:{node.lineno}", "exec"), node.lineno


def argparse_default(flag):
    """the default= of ruh main()'s parser.add_argument(flag, ...) and its line;
    the launchers (launch.ps1, launch_range.py) never pass these flags."""
    text, tree = ruh_source()
    for node in ast.walk(tree):
        if (isinstance(node, ast.Call) and getattr(node.func, "attr", None) == "add_argument" and node.args
                and isinstance(node.args[0], ast.Constant) and node.args[0].value == flag):
            for keyword in node.keywords:
                if keyword.arg == "default":
                    return ast.literal_eval(keyword.value), node.lineno
    raise LookupError(flag)


def run_probe_literal(name):
    """the literal value assigned to `name` inside run_probe (e.g. ruh:786)."""
    text, tree = ruh_source()
    for node in ast.walk(tree):
        if (isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name)
                and node.targets[0].id == name):
            return ast.literal_eval(node.value), node.lineno
    raise LookupError(name)


class Harness:
    """the per-packet namespace: send_reliable records each bunch with the
    sequence ruh's own take_channel_sequence hands out (B = 1000)."""

    def __init__(self, **names):
        self.source = ("127.0.0.1", 49152)
        self.sends = []
        first_offsets, _ = run_probe_literal("channel_first_dynamic_offset")
        self.ns = {"source": self.source, "channel_first_dynamic_offset": dict(first_offsets), "channel_next_offset": {},
                   "initial_actor_channel_sequences": {self.source: 1000}, "send_reliable": self.send_reliable}
        self.ns.update(names)
        nested_function("take_channel_sequence", self.ns)

    def send_reliable(self, channel, body, bits, has_exports, is_open=False, is_close=False):
        sequence = self.ns["take_channel_sequence"](self.source, channel)
        self.sends.append({"channel": channel, "sequence": sequence, "hex": body.hex(), "bit_count": bits,
                           "has_exports": bool(has_exports), "is_open": bool(is_open), "is_close": bool(is_close)})

    def take_sends(self):
        sends, self.sends = self.sends, []
        return sends


class FakeReader:
    """stands in for EquippableStateReader: only its caches are touched."""

    def __init__(self):
        self.handle = None
        self.image_base = 0
        self.key = b""
        self._names = {}
        self._class_names = {}


def first_offsets_json():
    """ruh:786 channel_first_dynamic_offset as the run starts (B = 1000)."""
    offsets, line = run_probe_literal("channel_first_dynamic_offset")
    return {"line": line, "base": 1000, "offsets": {str(channel): offset for channel, offset in offsets.items()}}


def jsonable_event(event):
    out = {}
    for key, value in event.items():
        out[key] = value if not isinstance(value, str) or not key.endswith("_error") else "error"
    return out


def statements_between(start_fragment, stop_fragment):
    """consecutive sibling statements from the one whose source starts with
    start_fragment up to (excluding) the one starting with stop_fragment."""
    text, tree = ruh_source()
    for node in ast.walk(tree):
        body = getattr(node, "body", None)
        if not isinstance(body, list):
            continue
        sources = [ast.get_source_segment(text, s) or "" for s in body]
        starts = [i for i, s in enumerate(sources) if s.startswith(start_fragment)]
        if not starts:
            continue
        first = starts[0]
        stop = next(i for i in range(first, len(body)) if sources[i].startswith(stop_fragment))
        lines = text.splitlines()[body[first].lineno - 1:body[stop - 1].end_lineno]
        return compile(textwrap.dedent("\n".join(lines)), f"ruh:{body[first].lineno}", "exec"), body[first].lineno
    raise LookupError(start_fragment)


# ---------------------------------------------------------------- abilities

def load_json(load):
    return {"index": load.index, "class_name": load.class_name, "channel": load.channel,
            "actor_guid": load.actor_guid, "slot": load.slot,
            "state_machine_guid": load.state_machine_guid, "inactive_guid": load.inactive_guid}


def tuning_entry_json(entry):
    value = entry.value
    if entry.kind == "float":
        return {"tag": entry.tag_net_index, "kind": "float", "value_bits": f64_bits(float(value))}
    return {"tag": entry.tag_net_index, "kind": entry.kind, "value": int(value)}


def section_world_abilities():
    import range_abilities as ra
    from range_abilities import AbilityLoad, TuningEntry

    out = {"constants": {}, "loads": [], "bad_index": [], "setup": [], "allocator": [],
           "tuning": [], "open": [], "tunings": {}}
    import range_server_bunch as rsb
    out["constants"] = {"ability_indices": list(ra.ability_indices), "first_ability_guid": ra.first_ability_guid,
                        "tuning_guids": {str(i): ra.ability_tuning_guid(i) for i in ra.ability_indices},
                        # ruh:2067 gate: starting_equippable_index >= len(wushu_starting_equippables)
                        "starting_equippable_count": len(rsb.wushu_starting_equippables),
                        "starting_equippables": [list(row) for row in rsb.wushu_starting_equippables]}
    # rsb:699-706 for every index, and the first one past the table
    out["constants"]["starting_equippable_guids"] = [list(rsb.starting_equippable_guids(i))
                                                     for i in range(len(rsb.wushu_starting_equippables))]
    try:
        rsb.starting_equippable_guids(len(rsb.wushu_starting_equippables))
        out["constants"]["past_the_table"] = None
    except ValueError as error:
        out["constants"]["past_the_table"] = type(error).__name__
    for index, rows in ra.starting_ability_tunings.items():
        out["tunings"][str(index)] = [[tag, kind, value] for tag, kind, value in rows]

    allocator = ra.ability_allocator()
    loads = [ra.plan_ability_load(index, allocator) for index in ra.ability_indices]
    out["after_plans"] = {"next_guid": allocator.next_guid, "next_channel": allocator.next_channel}
    # synthetic loads: the same shapes at registration-sized guids
    loads += [AbilityLoad(2, "Ability_Wushu_4_Smoke_C", 14, 208, 3, 100016, 100018),
              AbilityLoad(5, "Ability_Wushu_X_Dagger_Production_C", 17, 220, 9, 123456, 2097152),
              AbilityLoad(3, "Ability_Wushu_E_Dash_C", 15, 212, 5, 16382, 16384)]
    for load in loads:
        out["loads"].append(load_json(load))
        out["setup"].append([{**hb((body, bits)), "exports": exports}
                             for body, bits, exports in ra.ability_setup_bodies(load)])
    for index in (-1, 0, 1, 6, 7):
        fresh = ra.ability_allocator()
        try:
            ra.plan_ability_load(index, fresh)
            outcome = None
        except Exception as error:
            outcome = type(error).__name__
        out["bad_index"].append({"index": index, "error": outcome, "next_guid": fresh.next_guid})
    for start in (100000, 100001, 0, 1, 99999, 4294967200):
        from range_gun_loader import PurchaseAllocator
        a = PurchaseAllocator(next_channel=0, next_guid=start)
        out["allocator"].append({"start": start, "guids": [a.dynamic_guid() for _ in range(5)], "next_guid": a.next_guid})

    # tuning bodies (export=False is the production form, ra:197)
    def tuning(actor, component, entries, bits):
        case = {"actor": actor, "component": component, "tag_bits": bits,
                "entries": [tuning_entry_json(e) for e in entries]}
        case.update(attempt(ra.build_ability_tuning_body, actor, component, entries, bits))
        out["tuning"].append(case)

    tuning(212, 190006, [TuningEntry(245, "float", 7.5)], 10)                               # V.1 tuning_one_float
    tuning(212, 190006, [TuningEntry(245, "float", 7.5), TuningEntry(244, "int", 1)], 10)  # V.1 tuning_float_int
    rng = random.Random(806)
    specials = [0.0, -0.0, 1.0, 2.5, 7.5, -1.5, 1e-45, 1.5e-45, 3.4028234663852886e38, 3.4028235677973366e38,
                0.1, 1.0 / 3.0, 16777217.0, -123456.789, float("inf"), float("-inf"), float("nan")]
    for count in (1, 2, 3, 5, 8, 13, 21, 22, 32):
        for bits in (1, 3, 8, 10, 12, 16):
            entries = []
            for _ in range(count):
                tag = rng.randrange(1 << bits)
                if rng.random() < 0.5:
                    entries.append(TuningEntry(tag, "int", rng.choice([0, 1, 2, -1, -(2 ** 31), 2 ** 31 - 1, 2 ** 32 - 1,
                                                                       rng.randrange(-(2 ** 40), 2 ** 40)])))
                else:
                    entries.append(TuningEntry(tag, "float", rng.choice(specials + [rng.uniform(-1e6, 1e6)])))
            tuning(rng.choice([208, 212, 216, 220, 300100]), rng.choice([190004, 190006, 190008, 2 ** 32 - 2]), entries, bits)
    # guards (ra:105-106, 112-113) and struct.pack overflow
    tuning(212, 190006, [], 10)
    tuning(212, 190006, [TuningEntry(1, "int", 1)] * 33, 10)
    tuning(212, 190006, [TuningEntry(1, "int", 1)], 0)
    tuning(212, 190006, [TuningEntry(1, "int", 1)], 17)
    tuning(212, 190006, [TuningEntry(1024, "float", 1.0)], 10)
    tuning(212, 190006, [TuningEntry(1023, "float", 1.0)], 10)
    tuning(212, 190006, [TuningEntry(-1, "int", 1)], 10)
    tuning(212, 190006, [TuningEntry(3, "float", 1e39)], 10)
    tuning(212, 190006, [TuningEntry(3, "float", -3.4028236e38)], 10)
    tuning(212, 190006, [TuningEntry(3, "int", 5), TuningEntry(4000, "float", 1.0)], 10)

    # opens with tuning (ra:163-201), fixed tag tables
    sample = {"AbilityTuning.Wushu.4.SmokeDuration": 240, "AbilityTuning.Wushu.4.SmokeMaxCharges": 241,
              "AbilityTuning.Wushu.E.DashVelocityMultiplier": 243, "AbilityTuning.Wushu.E.MaxCharges": 244,
              "AbilityTuning.Wushu.E.PrimeDuration": 245, "AbilityTuning.Wushu.Q.MaxCharges": 250}

    def open_case(index, tags, bits):
        case = {"index": index, "tags": dict(tags), "tag_bits": bits}
        case.update(attempt(ra.build_ability_open_with_tuning, index, tags, bits))
        out["open"].append(case)

    for index in (2, 3, 4):
        open_case(index, sample, 10)
    names = sorted(sample)
    for seed in range(6):
        r = random.Random(1306 + seed)
        for bits in (10, 9, 11, 16, 1):
            tags = {name: r.randrange(1 << bits) for name in names}
            tags["AbilityTuning.Wushu.Unused"] = r.randrange(1 << bits)
            for index in (2, 3, 4):
                open_case(index, tags, bits)
    open_case(5, sample, 10)
    open_case(0, sample, 10)
    open_case(3, {k: v for k, v in sample.items() if "PrimeDuration" not in k}, 10)
    open_case(2, {**sample, "AbilityTuning.Wushu.4.SmokeDuration": 1024}, 10)
    open_case(4, sample, 0)
    open_case(4, sample, 17)
    open_case(4, {**sample, "AbilityTuning.Wushu.Q.MaxCharges": 65535}, 16)
    return out


# ---------------------------------------------------------------- tag net index

class FakeMemory:
    """flat little-endian memory for gtni's reads."""

    def __init__(self):
        self.regions = []

    def add(self, base, data):
        self.regions.append((base, bytearray(data)))

    def read(self, address, size):
        for base, data in self.regions:
            if base <= address and address + size <= base + len(data):
                return bytes(data[address - base:address - base + size])
        raise ValueError(f"unmapped read {address:#x}+{size}")


def run_tag_case(nodes, common, setting, free_every=0):
    """nodes: [(name, number, net_index)] in node-map order; common: [name]."""
    import gameplay_tag_net_index as gtni

    memory = FakeMemory()
    manager, settings = 0x10000, 0x20000
    names = {}
    comparison_of = {}

    def comparison(name):
        if name not in comparison_of:
            comparison_of[name] = 0x100 + 8 * len(comparison_of)
            names[comparison_of[name]] = name
        return comparison_of[name]

    slots = []
    for i, (name, number, net_index) in enumerate(nodes):
        if free_every and i % free_every == 0:
            slots.append(None)
        slots.append((comparison(name), number, 0x1000000 + 0x100 * i, net_index))
    table = bytearray(0x28 * max(1, len(slots)))
    for slot, entry in enumerate(slots):
        if entry is None:
            continue
        comp, number, node, net_index = entry
        struct.pack_into("<IIQQ", table, 0x28 * slot, comp, number, 0, node)
        node_bytes = bytearray(0x100)
        struct.pack_into("<H", node_bytes, 0x50, net_index)
        memory.add(node, node_bytes)
    memory.add(0x100000, table)
    common_table = bytearray(12 * max(1, len(common)))
    for i, name in enumerate(common):
        struct.pack_into("<I", common_table, 12 * i, comparison(name))
    memory.add(0x200000, common_table)
    manager_bytes = bytearray(0x300)
    struct.pack_into("<QII", manager_bytes, 0x130, 0x100000, len(nodes), len(slots))
    struct.pack_into("<QI", manager_bytes, 0x180, 0x200000, len(common))
    memory.add(manager, manager_bytes)
    settings_bytes = bytearray(0x100)
    struct.pack_into("<i", settings_bytes, 0xBC, setting)
    memory.add(settings, settings_bytes)

    class Rws:
        @staticmethod
        def read_bytes(handle, address, size):
            return memory.read(address, size)

    saved = (gtni.find_tags_manager, gtni.read_u32, gtni.read_u64, gtni.decode_name, gtni.rws)
    gtni.find_tags_manager = lambda handle, image_base, key: (manager, settings)
    gtni.read_u32 = lambda handle, address: struct.unpack("<I", memory.read(address, 4))[0]
    gtni.read_u64 = lambda handle, address: struct.unpack("<Q", memory.read(address, 8))[0]
    gtni.decode_name = lambda handle, image_base, comp, key: names[comp]
    gtni.rws = Rws
    try:
        result = gtni.predict_net_indices(None, 0, b"")
    except Exception as error:
        return {"error": type(error).__name__}
    finally:
        gtni.find_tags_manager, gtni.read_u32, gtni.read_u64, gtni.decode_name, gtni.rws = saved
    return {"indices": result.indices, "true_bit_num": result.true_bit_num,
            "first_bit_segment": result.first_bit_segment, "constructed": result.constructed}


def section_world_tags():
    out = {"cases": [], "true_bits": []}

    def case(label, nodes, common, setting, free_every=0):
        result = run_tag_case(nodes, common, setting, free_every)
        # node rows: "name number net_index" in node-map order (names have no spaces)
        out["cases"].append({"label": label, "nodes": [row(*n) for n in nodes], "common": common,
                             "setting": setting, **result})

    # V.2 synthetic list (node map order b.Tag, A.tag, a.Tag2, C, a_b, a.b, A)
    v2 = ["b.Tag", "A.tag", "a.Tag2", "C", "a_b", "a.b", "A"]
    case("v2", [(n, 0, 0xFFFF) for n in v2], ["C", "a_b"], 16)
    case("v2_constructed", [(n, 0, i * 3 + 1) for i, n in enumerate(v2)], ["C", "a_b"], 16)
    case("v2_no_common", [(n, 0, 0xFFFF) for n in v2], [], 16)
    case("v2_missing_common", [(n, 0, 0xFFFF) for n in v2], ["Nope", "C", "Zed.x"], 2)
    case("v2_free_slots", [(n, 0, 0xFFFF) for n in v2], ["a_b"], 16, free_every=2)
    case("empty", [], [], 16)
    case("common_past_end", [("A", 0, 0xFFFF), ("B", 0, 0xFFFF)], ["X", "Y", "B"], 16)
    case("duplicate_names", [("Dup", 2, 0xFFFF), ("dup", 1, 0xFFFF), ("Dup", 0, 0xFFFF), ("a", 0, 0xFFFF)], ["Dup"], 16)
    case("duplicate_names_constructed", [("Dup", 2, 7), ("dup", 1, 8), ("Dup", 0, 9), ("a", 0, 10)], [], 16)
    case("unicode", [(n, 0, 0xFFFF) for n in ("Ã‰tÃ©.x", "ete.y", "ÃŸ.tag", "SS.tag", "Ä°.a", "i.b", "Z")], ["ÃŸ.tag"], 16)
    case("negative_setting", [(n, 0, 0xFFFF) for n in v2], [], -3)
    case("constructed_first_only", [("b", 0, 0xFFFF), ("a", 0, 5), ("c", 0, 0xFFFF)], [], 16)
    case("not_constructed_first", [("b", 0, 4), ("a", 0, 0xFFFF), ("c", 0, 6)], [], 16)

    rng = random.Random(898)
    alphabet = "aAbBzZ_.09"
    for trial in range(30):
        count = rng.choice([1, 2, 3, 7, 31, 64, 200])
        names = []
        for _ in range(count):
            parts = ["".join(rng.choice(alphabet) for _ in range(rng.randint(1, 4))) for _ in range(rng.randint(1, 4))]
            names.append(".".join(parts))
        if trial in (0, 1):
            # the live count (spec 08 3.1): 898 tags -> 10 bits
            names = [f"AbilityTuning.Wushu.{i}" for i in range(898)]
            count = len(names)
        nodes = [(name, rng.choice([0, 0, 0, 1, 2]), 0xFFFF) for name in names]
        constructed = trial % 3 == 1
        if constructed:
            nodes = [(name, number, rng.randrange(0xFFFF)) for name, number, _ in nodes]
        common = [rng.choice(names) for _ in range(rng.randint(0, min(12, count)))]
        common += ["Missing.Tag"] * rng.randint(0, 2)
        rng.shuffle(common)
        case(f"random_{trial}", nodes, common, rng.choice([16, 10, 9, 4, 0, 31]), free_every=rng.choice([0, 0, 3, 7]))

    for n in list(range(0, 2100)) + [2 ** k + d for k in range(11, 32) for d in (-1, 0, 1)]:
        out["true_bits"].append([n, math.ceil(math.log2(n + 1))])
    return out


def section_world_tags_live():
    """the client's own node map, captured read-only by _capture_live_tags.py
    (fixtures/world_tags_live_capture.json), replayed through
    predict_net_indices on a fake memory image: the client's NetIndex path, the
    order python computes when every NetIndex is still 0xffff, and the B18 opens
    with the six live AbilityTuning indices. {"captured": false} until a
    capture exists (spec 08 open question 1)."""
    import json
    import range_abilities as ra

    path = Path(__file__).resolve().parents[2] / "fixtures" / "world_tags_live_capture.json"
    if not path.is_file():
        return {"captured": False}
    capture = json.loads(path.read_text(encoding="utf-8"))
    nodes = [tuple(node) for node in capture["nodes"]]
    client = run_tag_case(nodes, capture["common"], capture["setting"])
    computed = run_tag_case([(name, number, 0xFFFF) for name, number, _ in nodes], capture["common"], capture["setting"])
    out = {"captured": True, "when": capture["captured"], "python_live": capture["python_live"],
           "client": client, "computed": computed,
           # the computed order is ignored once the client built its index; does it agree?
           "computed_matches_client": client.get("indices") == computed.get("indices")}
    if "indices" in client:
        out["opens"] = [{"index": i, **attempt(ra.build_ability_open_with_tuning, i, client["indices"], client["true_bit_num"])}
                        for i in (2, 3, 4)]
    return out


# ---------------------------------------------------------------- bots

def bot_json(bot):
    return {"index": bot.index, "channel": bot.channel, "actor_guid": bot.actor_guid, "health_guid": bot.health_guid,
            "handler_guid": bot.handler_guid, "team_guid": bot.team_guid, "team": bot.team,
            "location": vec(bot.location), "yaw": bot.yaw, "life": bot.life, "alive": bot.alive, "opened": bot.opened}


def bot_bodies(bot):
    import range_bots as rb
    return {"open": attempt(rb.build_bot_open, bot), "health_export": attempt(rb.build_bot_health_body, bot, True),
            "health_update": attempt(rb.build_bot_health_body, bot, False),
            "alive_export": attempt(rb.build_bot_alive_body, bot, True)}


def section_world_bots():
    import range_bots as rb

    out = {}
    out["constants"] = {"bot_center_z": rb.bot_center_z, "manager_origin": vec(rb.manager_origin),
                        "dummy_location": vec(rb.dummy_location), "first_bot_channel": rb.first_bot_channel,
                        "first_bot_guid": rb.first_bot_guid, "bot_guid_stride": rb.bot_guid_stride,
                        "player_team": rb.player_team, "red_team": rb.red_team}

    # production order: the dummy first, then bots, none dying (20 of them)
    field = rb.BotField()
    dummy = field.allocate_dummy()
    entities = [{**bot_json(dummy), "bodies": {"open": attempt(rb.build_dummy_open, dummy),
                                               "health": attempt(rb.build_dummy_health_body, dummy)}}]
    for _ in range(20):
        bot = field.allocate(len(field.bots))
        field.bots.append(bot)
        entities.append({**bot_json(bot), "bodies": bot_bodies(bot)})
    out["placement_all_alive"] = {"entities": entities, "next_channel": field.next_channel,
                                  "next_guid": field.next_guid, "rng_next": field.rng.random()}

    # kills, retirements and respawns (placement depends on who is alive)
    rng = random.Random(4242)
    field = rb.BotField()
    field.allocate_dummy()
    ops = []

    def spawn():
        bot = field.allocate(len(field.bots))
        field.bots.append(bot)
        bot.opened = True
        ops.append({"op": "spawn", "bot": bot_json(bot)})

    for _ in range(5):
        spawn()
    for _ in range(120):
        roll = rng.random()
        alive = [i for i, b in enumerate(field.bots) if b.alive]
        dead = [i for i, b in enumerate(field.bots) if not b.alive]
        if roll < 0.4 and alive:
            i = rng.choice(alive)
            field.bots[i].alive = False
            ops.append({"op": "kill", "at": i})
        elif roll < 0.7 and dead:
            i = rng.choice(dead)
            field.bots.pop(i)
            ops.append({"op": "retire", "at": i})
        elif sum(b.alive for b in field.bots) < field.count:
            spawn()
    out["placement_respawns"] = {"ops": ops, "next_channel": field.next_channel, "next_guid": field.next_guid,
                                 "rng_next": field.rng.random()}

    # crowded field: 40 alive bots force rejected attempts and the fallback spot
    field = rb.BotField()
    crowd = []
    for _ in range(40):
        bot = field.allocate(len(field.bots))
        field.bots.append(bot)
        crowd.append({"location": vec(bot.location), "yaw": bot.yaw})
    out["placement_crowded"] = {"bots": crowd, "rng_next": field.rng.random()}

    # bodies with other life values (rb:237-266)
    base = rb.Bot(0, 401, 300108, 300110, (4463.170774427629, -14407.674895086804, 1351.8), 300112,
                  65.86815966797721, team_guid=300114)
    health = []
    for life, alive in ((100.0, True), (74.0, True), (48.0, True), (0.5, True), (0.0, False), (-26.0, False),
                        (99.99999999, True), (1e-50, True), (33.3, True)):
        bot = rb.Bot(**{**base.__dict__, "life": life, "alive": alive})
        health.append({"life": life, "alive": alive, "export": attempt(rb.build_bot_health_body, bot, True),
                       "update": attempt(rb.build_bot_health_body, bot, False),
                       "alive_export": attempt(rb.build_bot_alive_body, bot, True)})
    out["health"] = health
    sections = []
    for life, maximum in ((50.0, 50.0), (0.0, 0.0), (1000.0, 1000.0), (-1.0, 100.0), (101.0, 100.0), (5.0, 1000.5),
                          (float("nan"), 100.0), (0.1, 0.2)):
        for actor, section, name, alive, export in ((10, 316, "ShieldDamageSection", True, False),
                                                    (300100, 300102, "HealthDamageSection", False, True)):
            sections.append({"actor": actor, "section": section, "name": name, "life_bits": f64_bits(life),
                             "maximum_bits": f64_bits(maximum), "alive": alive, "export": export,
                             **attempt(rb.build_damage_section_body, actor, section, name, life, maximum, alive, export)})
    out["damage_sections"] = sections
    bad_team = []
    location = (4825.0, -14600.0, 1351.8)
    for team_guid, team, yaw in ((300106, 0, 90.0), (300106, 0, 0.0), (0, 0, 90.0), (300107, 0, 90.0),
                                 (300106, 1, 90.0), (300106, 2, 90.0), (300106, 255, 90.0), (300106, 0, -0.0027),
                                 (300106, 0, 0.0027465820312499), (300106, 0, 359.99), (300106, 0, -180.0)):
        bot = rb.Bot(0, 400, 300100, 300102, location, 300104, yaw, team_guid=team_guid, team=team)
        bad_team.append({"team_guid": team_guid, "team": team, "yaw": yaw, **attempt(rb.build_bot_open, bot)})
    out["bot_open_cases"] = bad_team

    # dummy damage text (rb:210-226)
    dummy = rb.BotField().allocate_dummy()
    damages = sorted({d for row in rb.weapon_damage.values() for d in row} | set(rb.default_damage))
    text = []
    for damage in [float(d) for d in damages] + [0.0, 0.5, -5.5, 1e300, 255.99]:
        text.append({"damage": damage, "payload": hb(rb.build_update_damage_text_payload(damage)),
                     "body": hb(rb.build_update_damage_text_body(dummy, damage))})
    fixed = rb.Bot(0, 400, 300400, 300402, rb.dummy_location, 300404, 180.0)
    text.append({"damage": 26.0, "actor": 300400, "body": hb(rb.build_update_damage_text_body(fixed, 26.0)),
                 "payload": hb(rb.build_update_damage_text_payload(26.0))})
    out["damage_text"] = text

    # damage table (rb:71-83, 341-343)
    classes = list(rb.weapon_damage) + ["Unknown_C", "", None]
    out["shot_damage"] = [[c, region, rb.shot_damage(c, region)] for c in classes for region in ("head", "body", "legs")]

    # view rays
    rays = []
    for pitch in [-90.0, -89.9, -45.0, -2.86, 0.0, 1.7288577981699722, 10.0, 45.0, 89.9, 90.0, 123.4, -400.0]:
        for yaw in [-720.0, -180.0, -130.79, -101.31889987841747, 0.0, 45.0, 90.0, 179.999, 270.0, 1000.5]:
            rays.append({"pitch": pitch, "yaw": yaw, "dir": vec(rb.view_ray(pitch, yaw))})
    out["view_ray"] = rays

    # hit regions: aimed grids around bot-like centres, the V.3 cases and random near-head rays.
    # rows: origin(3) dir(3) center(3) region distance
    hits = []

    def hit(origin, direction, center):
        result = rb.hit_region(origin, direction, center)
        hits.append(row(*vec(origin), *vec(direction), *vec(center),
                        None if result is None else result[0], None if result is None else result[1]))

    v3_center = (1000.0, 0.0, 100.0)
    for origin, direction in (((0, 0, 170), (1.0, 0.0, 0.0)), ((0, 0, 120), (1.0, 0.0, 0.0)), ((0, 0, 40), (1.0, 0.0, 0.0)),
                              ((0, 0, 176), (1.0, 0.0, 0.0)), ((0, 0, 3), (1.0, 0.0, 0.0)), ((0, 0, 1), (1.0, 0.0, 0.0)),
                              ((0, 100, 120), (1.0, 0.0, 0.0)), ((0, 0, 120), (-1.0, 0.0, 0.0)),
                              ((1000, 0, 400), (0.0, 0.0, -1.0)), ((1000, 0, 100), (1.0, 0.0, 0.0)),
                              ((1000, 0, 175), (0.0, 0.0, 1.0)), ((1010, 0, 300), (0.0, 0.0, -1.0))):
        hit(origin, direction, v3_center)
    centers = [(4463.170774427629, -14407.674895086804, 1351.8), (4795.468318650681, -14628.70432286504, 1351.8),
               (6190.0, -11640.0, 1274.0)]
    origins = [(4825.0, -12600.0, 1371.8), (4786.03, -13285.84, 1427.98), (6000.0, -12000.0, 1300.0),
               (3000.0, -15500.0, 1500.0)]
    for center in centers:
        for origin in origins:
            for jz in range(-110, 120, 15):
                for jx in (-31.0, 0.0, 26.5):
                    for jy in (-24.0, 0.0, 29.0):
                        dx, dy, dz = center[0] + jx - origin[0], center[1] + jy - origin[1], center[2] + jz - origin[2]
                        yaw = math.degrees(math.atan2(dy, dx))
                        pitch = math.degrees(math.atan2(dz, math.hypot(dx, dy)))
                        hit(origin, rb.view_ray(pitch, yaw), center)
    r = random.Random(5)
    for _ in range(800):
        center = (r.uniform(3500, 6500), r.uniform(-15500, -11500), 1351.8)
        origin = (center[0] + r.uniform(-2500, 2500), center[1] + r.uniform(-2500, 2500), r.uniform(1260, 1500))
        target = (center[0] + r.uniform(-30, 30), center[1] + r.uniform(-30, 30), center[2] + r.uniform(40, 100))
        dx, dy, dz = (target[i] - origin[i] for i in range(3))
        yaw = math.degrees(math.atan2(dy, dx))
        pitch = math.degrees(math.atan2(dz, math.hypot(dx, dy)))
        hit(origin, rb.view_ray(pitch, yaw), center)
    out["hit_region"] = hits

    # the fixture must catch plain left-to-right addition in _ray_sphere (spec 08
    # 7.4): count rows whose result changes when sum() is replaced by +
    def plain_sphere(origin, direction, center, radius):
        oc = [origin[i] - center[i] for i in range(3)]
        b = oc[0] * direction[0] + oc[1] * direction[1] + oc[2] * direction[2]
        c = oc[0] * oc[0] + oc[1] * oc[1] + oc[2] * oc[2] - radius * radius
        disc = b * b - c
        if disc < 0:
            return None
        t = -b - math.sqrt(disc)
        return t if t > 0 else None

    def parse(line):
        parts = line.split(" ")
        values = [float(p) for p in parts[:9]]
        return tuple(values[0:3]), tuple(values[3:6]), tuple(values[6:9]), parts[9:]

    real_sphere = rb._ray_sphere
    rb._ray_sphere = plain_sphere
    differs = 0
    try:
        for line in hits:
            origin, direction, center, expected = parse(line)
            result = rb.hit_region(origin, direction, center)
            if row(*((None, None) if result is None else result)) != " ".join(expected):
                differs += 1
    finally:
        rb._ray_sphere = real_sphere
    assert differs > 50, f"only {differs} hit rows tell neumaier from plain addition"
    out["plain_sum_differs"] = differs

    # dummy_hit and first_hit
    field = rb.BotField()
    dummy = field.allocate_dummy()
    bots = [field.allocate(i) for i in range(5)]
    first = []
    probe_origins = [(4825.0, -12600.0, 1371.8), (5200.0, -12900.0, 1400.0), (6190.0, -11000.0, 1290.0)]
    probe_rays = [(0.0, -101.31889987841747), (1.7288577981699722, -101.31889987841747), (-0.5, -95.0), (0.3, -80.0),
                  (0.0, -90.0), (2.0, -110.0), (-1.0, -88.5), (0.0, -270.0), (1.0, -91.0)]
    states = [(True, True), (True, False), (False, True)]
    for step, state_seed in enumerate(range(12)):
        sr = random.Random(state_seed)
        for bot in bots:
            bot.alive, bot.opened = sr.choice(states) if step else (True, True)
        dummy.opened = step % 2 == 0
        for origin in probe_origins:
            for pitch, yaw in probe_rays:
                direction = rb.view_ray(pitch, yaw)
                best = rb.first_hit(bots, origin, direction)
                on_dummy = rb.dummy_hit(dummy, origin, direction)
                # row: alive/opened per bot, dummy opened, origin, pitch, yaw, first (index region
                # distance), dummy (region distance)
                first.append(row(*[f"{int(b.alive)}{int(b.opened)}" for b in bots], dummy.opened, *vec(origin), pitch, yaw,
                                 *((None, None, None) if best is None else (bots.index(best[0]), best[1], best[2])),
                                 *((None, None) if on_dummy is None else on_dummy)))
    out["first_hit"] = {"dummy": bot_json(dummy), "bots": [bot_json(b) for b in bots], "cases": first}
    out["dummy_hit_none"] = rb.dummy_hit(None, (0.0, 0.0, 0.0), (1.0, 0.0, 0.0))

    # cpython 3.12 math.hypot (vector_norm), used by place() (rb:155)
    hr = random.Random(200)
    pairs = [(120.0, 160.0), (200.0, 0.0), (0.0, -200.0), (-160.0, 120.0), (0.0, 0.0), (1e-310, 3e-310),
             (1e308, 1e308), (199.99999999999997, 0.0), (141.4213562373095, 141.4213562373095),
             (141.42135623730951, 141.42135623730951), (5e-324, 0.0), (2.0 ** 1023, 2.0 ** 1022)]
    for _ in range(1500):
        angle = hr.uniform(0, 2 * math.pi)
        radius = hr.choice([200.0, hr.uniform(150.0, 250.0), hr.uniform(0, 3000)])
        pairs.append((radius * math.cos(angle), radius * math.sin(angle)))
    out["hypot"] = [bits_row((x, y, math.hypot(x, y))) for x, y in pairs]
    # cpython 3.12 builtin sum() over floats (neumaier), used by _ray_sphere (rb:296-297)
    sums = []
    sr = random.Random(30)
    for _ in range(800):
        values = [sr.uniform(-1e4, 1e4) * sr.choice([1.0, 1e-8, 1e8]) for _ in range(sr.randint(1, 4))]
        sums.append(values)
    sums += [[0.1, 0.2, 0.3], [-0.0], [-0.0, -0.0], [1e308, 1e308, -1e308], [1e308, 1e308], [1.0, 1e100, 1.0, -1e100]]
    # rows: the values then their sum() last
    out["py_sum"] = [bits_row(values + [sum(values)]) for values in sums]
    return out


# ---------------------------------------------------------------- projectiles

def section_world_projectiles():
    import range_projectiles as rp
    from range_server_bunch import BitWriter

    out = {"throws": [], "zones": [], "fallback": [], "hit_offset": [], "thrown": [], "stops": [], "alloc": {}}
    poses = [((4786.03, -13285.84, 1427.98), -2.86, -130.79)]
    for pitch in (-89.0, -45.0, 0.0, 10.0, 60.0, 85.0, 89.9, 90.0, 120.0, 200.0, -200.0, 359.0, 540.0, -540.0,
                  720.5, -180.0, 180.0, 82.5, 81.9):
        for yaw in (-180.0, -130.79, 0.0, 45.0, 179.99, 270.0, -720.0):
            poses.append(((4825.0, -12600.0, 1371.8), pitch, yaw))
    r = random.Random(61)
    for _ in range(60):
        poses.append(((r.uniform(3000, 7000), r.uniform(-16000, -11000), r.uniform(1260, 1700)),
                      r.uniform(-89, 89), r.uniform(-180, 180)))
    field = rp.ProjectileField()
    for view_location, view_pitch, view_yaw in poses:
        location, pitch, yaw, velocity = rp.throw_transform(view_location, view_pitch, view_yaw,
                                                            forward=rp.stage_a_spawn_forward)
        channel, actor, movement = field.channel(), field.guid(), field.guid()
        movement_bits = BitWriter()
        rp.append_rep_movement(movement_bits, location, pitch, yaw, velocity)
        out["throws"].append({
            "view": vec(view_location), "view_pitch": view_pitch, "view_yaw": view_yaw,
            "location": vec(location), "pitch": pitch, "yaw": yaw, "velocity": vec(velocity),
            "channel": channel, "actor": actor, "movement": movement,
            "rep_movement": hb((movement_bits.to_bytes(), movement_bits.bit_count)),
            "open": attempt(rp.smoke_projectile_open_body, actor, location, pitch, yaw, velocity,
                            movement_guid=movement, ignore_pawn=False),
            "fallback": vec(rp.ballistic_fallback(location, velocity))})
    out["alloc"] = {"next_channel": field.next_channel, "next_guid": field.next_guid}
    edge = rp.ProjectileField(next_channel=32766)
    taken = [edge.channel()]
    try:
        edge.channel()
        taken.append("ok")
    except RuntimeError:
        taken.append("RuntimeError")
    out["alloc_edge"] = taken

    # zone opens (rp:253-264); yaw is injected (ruh:1087 uses the unseeded global random)
    zr = random.Random(62)
    zone_cases = [(610104, (5000.0, -14000.0, 1319.0), 123.0), (610104, (100.0, 200.0, 1318.8), 123.0),
                  (610110, (3180.6890381934163, -15146.301771658329, 1318.8), 0.0),
                  (610110, (3180.6890381934163, -15146.301771658329, 1318.8), 359.99),
                  (612000, (-5.0, 0.04, -0.05), 0.0027465820312499), (612002, (0.0, 0.0, 0.0), 0.0027465820312501),
                  (612004, (214748364.7, -214748364.8, 1.0), 180.0), (612006, (214748364.8, 0.0, 0.0), 180.0),
                  (3, (1.0, 2.0, 3.0), 90.0), (612008, (1.0, 2.0, 3.0), -90.0)]
    for _ in range(20):
        zone_cases.append((610100 + 2 * zr.randrange(10000), (zr.uniform(3000, 7000), zr.uniform(-16000, -11000),
                                                               zr.uniform(1250, 1700)), zr.uniform(0.0, 360.0)))
    for zone, location, yaw in zone_cases:
        out["zones"].append({"zone": zone, "location": vec(location), "yaw": yaw,
                             **attempt(rp.smoke_zone_open_body, zone, location, yaw)})

    # ballistic fallback (rp:285-295) beyond the throws
    fr = random.Random(63)
    falls = [((0.0, 0.0, 2000.0), (0.0, 0.0, 0.0)), ((0.0, 0.0, 1253.8), (0.0, 0.0, 0.0)),
             ((0.0, 0.0, 1253.80001), (0.0, 0.0, 0.0)), ((10.0, 20.0, 1300.0), (0.0, 0.0, 2900.0)),
             ((10.0, 20.0, 5000.0), (100.0, -100.0, 0.0)), ((4746.9864419503865, -13331.088360883461, 1425.352643253695),
                                                           (-1887.1053057312877, -2187.004109367309, 256.6059054100816))]
    for _ in range(40):
        falls.append(((fr.uniform(3000, 7000), fr.uniform(-16000, -11000), fr.uniform(1200, 3000)),
                      (fr.uniform(-2900, 2900), fr.uniform(-2900, 2900), fr.uniform(-2900, 2900))))
    for location, velocity in falls:
        out["fallback"].append({"location": vec(location), "velocity": vec(velocity),
                                "result": vec(rp.ballistic_fallback(location, velocity))})

    for hit_location, normal in (((100.0, 200.0, 1253.8), (0.0, 0.0, 1.0)), ((1.5, -2.5, 3.25), (0.6, 0.0, -0.8)),
                                 ((4746.98, -13331.08, 1425.35), (-0.70710678, 0.70710678, 0.0))):
        out["hit_offset"].append({"location": vec(hit_location), "normal": vec(normal),
                                  "result": vec(rp.smoke_location_from_hit(hit_location, normal))})

    states = [None, "InactiveState", "EquippedState", "ProjectileThrowState", "CommitTime", "AirBendingState", "IdleState"]
    out["thrown"] = [[a, b, rp.smoke_thrown(a, b)] for a in states for b in states]

    # flight_stop_point sequences (rp:346-360)
    def snap(class_name, location, collisions=0, hit_location=(0.0, 0.0, 0.0), hit_normal=(0.0, 0.0, 0.0)):
        return rp.ProjectileSnapshot(class_name, location, collisions, hit_location, hit_normal)

    smoke = rp.smoke_projectile_class
    sequences = [
        [(10.5, snap(smoke, (1.0, 2.0, 3.0))), (10.6, snap(smoke, (1.0, 2.0, 3.0), 1, (100.0, 200.0, 1253.8), (0.0, 0.0, 1.0)))],
        [(10.5, snap(smoke, (1.0, 2.0, 3.0))), (12.0, snap(smoke, (4.0, 5.0, 6.0)))],
        [(10.5, snap(smoke, (1.0, 2.0, 3.0))), (11.0, None)],
        [(10.1, None), (10.29, None), (10.3, None)],
        [(10.1, None), (10.4, snap("Other_C", (1.0, 2.0, 3.0)))],
        [(10.1, snap(smoke, None)), (10.2, snap(smoke, (7.0, 8.0, 9.0))), (10.3, snap("Other_C", (1.0, 1.0, 1.0)))],
        [(11.99, snap(smoke, (1.0, 2.0, 3.0))), (12.0, snap(smoke, (1.5, 2.5, 3.5), 0))],
        [(10.0, snap(smoke, (0.0, 0.0, 0.0), 3, (5.0, 6.0, 7.0), (0.0, 1.0, 0.0)))],
    ]
    view = ((4786.03, -13285.84, 1427.98), -2.86, -130.79)
    for sequence in sequences:
        location, pitch, yaw, velocity = rp.throw_transform(*view, forward=60.0)
        flight = rp.Flight(10000, 610100, 610102, 10.0, location, velocity, pitch, yaw)
        steps = []
        for now, snapshot in sequence:
            stop = rp.flight_stop_point(flight, snapshot, now)
            steps.append({"now": now, "snapshot": None if snapshot is None else {
                "class_name": snapshot.class_name, "location": None if snapshot.location is None else vec(snapshot.location),
                "collisions": snapshot.collisions, "hit_location": vec(snapshot.hit_location),
                "hit_normal": vec(snapshot.hit_normal)},
                "stop": None if stop is None else [vec(stop[0]), stop[1]], "seen": flight.seen,
                "last_location": None if flight.last_location is None else vec(flight.last_location)})
        out["stops"].append({"opened": 10.0, "location": vec(location), "velocity": vec(velocity), "steps": steps})
    return out


# ---------------------------------------------------------------- teams

def capture_frame_body(module, fn, *args, **kwargs):
    """call a range_bomb_team_open helper with its framing patched out."""
    captured = {}
    original = module.build_server_bunch_frame

    # the frame itself is net's job; only the body is compared here
    def fake(body, bits, header, **info):
        captured["body"] = (body, bits)
        captured["header"] = header
        return b"", 0

    module.build_server_bunch_frame = fake
    try:
        fn(*args, **kwargs)
    finally:
        module.build_server_bunch_frame = original
    return captured["body"], captured["header"]


def section_world_teams():
    import range_bomb_team_open as rbt
    import range_server_bunch as rsb

    def actor_rep(plain, seed):
        payload, bits = plain
        return rsb.append_actor_rep_layout_payload(b"", 0, rsb.encode_short_actor_rep_payload(payload, bits, 0, seed), bits)

    out = {}
    out["team_state_open"] = [{"guid": g, **attempt(rsb.build_dynamic_base_team_state_reference, g)} for g in (100, 102, 104, 101, 0)]
    out["team_states_array"] = hb(actor_rep(rsb.build_object_array_property_payload(34, (100, 102)), 2))   # ruh:3449-3454
    arrays = []
    for handle, guids in ((34, (100, 102)), (34, ()), (1, (1,)), (0xFFFF, (0xFFFF, 2, 3)), (0, (100,)), (0x10000, (100,)),
                          (34, (0,)), (34, (0x10000,)), (34, tuple(range(1, 258))), (34, tuple(range(1, 257)))):
        arrays.append({"handle": handle, "guids": list(guids), **attempt(rsb.build_object_array_property_payload, handle, guids)})
    out["object_arrays"] = arrays
    links = []
    for actor, component, name, role in ((100, 104, "BlueTeam", 1), (102, 106, "RedTeam", 2), (100, 104, "BlueTeam", None),
                                         (102, 106, "RedTeam", 1), (100, 106, "RedTeam", 2), (101, 104, "BlueTeam", 1),
                                         (100, 105, "BlueTeam", 1), (100, 104, "GreenTeam", 1), (100, 104, "BlueTeam", 3),
                                         (100, 104, "BlueTeam", 0)):
        links.append({"actor": actor, "component": component, "name": name, "role": role,
                      **attempt(rsb.build_team_state_component_update, actor, component, name, role)})
    out["team_links"] = links
    out["player_state_team"] = hb(actor_rep(rsb.build_object_property_payload(27, 100), 6))    # ruh:3512-3516

    # B7/B8: the inline production statements (ruh:3548-3568, 3601-3615)
    code, line = statements_between("BOMB_TEAM_GUID = 108", "frame_bt, _ = build_server_bunch_frame(")
    namespace = {"BitWriter": rsb.BitWriter, "encode_short_actor_rep_payload": rsb.encode_short_actor_rep_payload}
    exec(code, namespace)
    inline_open = (namespace["hdr_bt"].to_bytes(), namespace["hdr_bt"].bit_count)
    code2, line2 = statements_between("rep_bt2 = BitWriter()", "frame_bt2, _ = build_server_bunch_frame(")
    value_default, value_line = argparse_default("--bomb-team-number-value")
    bits_default, bits_line = argparse_default("--bomb-team-number-bits")
    out["bomb_team_number_production"] = {"value": value_default, "bits": bits_default, "lines": [value_line, bits_line]}
    numbers = []
    # production first, then the rbt helper's 0..255 range at 8 bits
    for value, bits in ((value_default, bits_default), (0, 8), (0xFE, 8), (255, 8)):
        namespace = {"BitWriter": rsb.BitWriter, "encode_short_actor_rep_payload": rsb.encode_short_actor_rep_payload,
                     "bomb_team_number_value": value, "bomb_team_number_bits": bits}
        exec(code2, namespace)
        numbers.append({"value": value, "bits": bits, **hb((namespace["hdr_bt2"].to_bytes(), namespace["hdr_bt2"].bit_count))})
    out["bomb_team_inline_lines"] = [line, line2]
    out["bomb_team_number_inline"] = numbers

    # the rbt helper pair with the production arguments, framing stripped
    helper_open, header = capture_frame_body(rbt, rbt.build_bomb_team_open_bunch, 108, channel_sequence=1006,
                                             server_sequence=8770, acked_client_sequence=4817,
                                             ack_history_mask=0xFFFFFFFF, stateless_header=16, receive_key=bytes(range(32)))
    assert helper_open == inline_open, "rbt open body differs from the inline production body"
    out["bomb_team_open"] = hb(helper_open)
    helper_numbers = []
    for subobj, team, handle in ((108, 1, 3), (108, 0, 3), (108, 0xFE, 3), (108, 255, 3), (300106, 0, 3), (108, 1, 200)):
        body, _ = capture_frame_body(rbt, rbt.build_bomb_team_number_update, subobj, team, handle, 1007, 8770,
                                     acked_client_sequence=4817, ack_history_mask=0xFFFFFFFF, stateless_header=16,
                                     receive_key=bytes(range(32)))
        helper_numbers.append({"subobj": subobj, "team": team, "handle": handle, **hb(body)})
    for helper, inline in zip(helper_numbers[:4], numbers):
        assert helper["hex"] == inline["hex"], "rbt number body differs from the inline production body"
    for team in (-1, 256):
        try:
            rbt.build_bomb_team_number_update(108, team, 3, 0, 0)
            helper_numbers.append({"team": team, "error": None})
        except ValueError:
            helper_numbers.append({"team": team, "error": "ValueError"})
    out["bomb_team_number"] = helper_numbers
    # outer stays PlayerState 6 (the rbt default and the inline value); its seed is always 6 (rbt:61)
    other_opens = []
    for subobj in (108, 300106, 2, 16384):
        body, _ = capture_frame_body(rbt, rbt.build_bomb_team_open_bunch, subobj)
        other_opens.append({"subobj": subobj, **hb(body)})
    out["bomb_team_opens"] = other_opens
    return out


# ---------------------------------------------------------------- per-packet systems

def section_world_bot_systems():
    """bot_shots (ruh:969-1012) then the bot tick block (ruh:2034-2062) per
    datagram, in production order (mirror poll before the bot tick)."""
    import range_bots as rb
    import read_equippable_state as res

    tick_code, tick_line = if_statement("range_bots and source in range_streaming_sent_sources")
    bot_count, bot_count_line = argparse_default("--range-bot-count")
    harness = Harness(range_bots=True, bot_fields={}, bot_ticks={}, bot_spawned_at={}, range_bot_count=bot_count)
    source = harness.source
    harness.ns["range_streaming_sent_sources"] = {source}
    bot_shots, shots_line = nested_function("bot_shots", harness.ns)
    reader = FakeReader()
    camera = {}
    saved = res.read_camera
    res.read_camera = lambda handle, image_base, controller=None: camera.get("value")
    rng = random.Random(1012)
    classes = [name for name, _ in sorted(rb.weapon_damage.items())] + ["Unknown_C"]
    datagrams = []
    now = 5000.0
    try:
        for _ in range(900):
            now += rng.choice([0.004, 0.008, 0.016, 0.031, 0.05, 0.07, 0.12, 0.26])
            event = {}
            shot = None
            field = harness.ns["bot_fields"].get(source)
            if field is not None and rng.random() < 0.45:
                origin = (4825.0 + rng.uniform(-250, 250), -12600.0 + rng.uniform(-400, 300), 1371.8 + rng.uniform(-40, 40))
                pick = rng.random()
                live = [b for b in field.bots if b.alive and b.opened]
                if pick < 0.7 and live:
                    bot = rng.choice(live)
                    target = (bot.location[0] + rng.uniform(-12, 12), bot.location[1] + rng.uniform(-12, 12),
                              bot.location[2] + rng.choice([75.0, 70.0, 20.0, 0.0, -60.0, -90.0, 140.0]))
                elif pick < 0.85 and field.dummy is not None:
                    d = field.dummy.location
                    target = (d[0] + rng.uniform(-10, 10), d[1] + rng.uniform(-10, 10), d[2] + 4.0 + rng.choice([75.0, 20.0, -60.0]))
                else:
                    target = (origin[0] + rng.uniform(-3000, 3000), origin[1] - rng.uniform(100, 3000), origin[2] + rng.uniform(-300, 300))
                dx, dy, dz = (target[i] - origin[i] for i in range(3))
                yaw = math.degrees(math.atan2(dy, dx))
                pitch = math.degrees(math.atan2(dz, math.hypot(dx, dy)))
                camera["value"] = (origin, (pitch, yaw, 0.0))
                class_name = rng.choice(classes)
                bot_shots(source, class_name, 1, now, event, harness.send_reliable, reader)
                shot = {"origin": vec(origin), "pitch": pitch, "yaw": yaw, "class_name": class_name}
            shot_sends = harness.take_sends()
            harness.ns.update(now=now, event=event)
            exec(tick_code, harness.ns)
            datagrams.append({"now": now, "shot": shot, "shot_sends": shot_sends, "tick_sends": harness.take_sends(),
                              "event": jsonable_event(event)})
    finally:
        res.read_camera = saved
    field = harness.ns["bot_fields"][source]
    return {"lines": {"bot_tick": tick_line, "bot_shots": shots_line, "range_bot_count": bot_count_line},
            "range_bot_count": bot_count, "first_offsets": first_offsets_json(), "datagrams": datagrams,
            "final": {"bots": [bot_json(b) for b in field.bots], "next_channel": field.next_channel,
                      "next_guid": field.next_guid, "rng_next": field.rng.random()}}


def section_world_projectile_systems():
    """cloudburst_state (ruh:1016-1044) from the mirror poll, then the flight
    tick block (ruh:2026-2030 -> tick_projectiles ruh:1046-1100) per datagram."""
    import random as global_random
    import range_projectiles as rp
    import read_equippable_state as res

    tick_code, tick_line = if_statement("ability_projectiles and source in projectile_fields")
    stage, _ = argparse_default("--ability-projectile-stage")
    assert stage == "A", "the port covers stage A only"
    harness = Harness(ability_projectiles=True, ability_projectile_stage=stage, projectile_fields={}, projectile_ticks={})
    source = harness.source
    cloudburst_state, state_line = nested_function("cloudburst_state", harness.ns)
    nested_function("tick_projectiles", harness.ns)
    reader = FakeReader()
    rng = random.Random(1044)
    camera = {}
    reads = []
    yaws = []
    behaviours = {}
    kinds = ["hit", "never", "lost", "timeout", "error", "noroot", "hit_late", "other_class"]

    def fake_camera(handle, image_base, controller=None):
        value = camera.get("value")
        if value == "raise":
            raise OSError("camera read failed")
        return value

    def fake_read_projectile(state_reader, channel):
        state = behaviours.setdefault(channel, {"kind": kinds[len(behaviours) % len(kinds)], "reads": 0})
        state["reads"] += 1
        kind, count = state["kind"], state["reads"]
        flying = rp.ProjectileSnapshot(rp.smoke_projectile_class, (4700.0 - 37.5 * count, -13300.0 - 41.25 * count, 1420.0 - 3.0 * count),
                                       0, (0.0, 0.0, 0.0), (0.0, 0.0, 0.0))
        if kind == "error" and count <= 3:
            reads.append({"channel": channel, "error": True})
            raise OSError("projectile read failed")
        if kind == "hit" and count >= 3 or kind == "hit_late" and count >= 9:
            snapshot = rp.ProjectileSnapshot(rp.smoke_projectile_class, flying.location, 1 + count % 2,
                                             (4600.0 + count, -13400.0 - count, 1253.8), (0.0, 0.6, 0.8))
        elif kind in ("never", "error"):
            snapshot = None
        elif kind == "lost" and count >= 3:
            snapshot = None
        elif kind == "noroot":
            snapshot = rp.ProjectileSnapshot(rp.smoke_projectile_class, None, 0, (0.0, 0.0, 0.0), (0.0, 0.0, 0.0))
        elif kind == "other_class":
            snapshot = rp.ProjectileSnapshot("Projectile_Other_C", flying.location, 1, (1.0, 2.0, 3.0), (0.0, 0.0, 1.0))
        else:
            snapshot = flying
        reads.append({"channel": channel, "snapshot": None if snapshot is None else {
            "class_name": snapshot.class_name, "location": None if snapshot.location is None else vec(snapshot.location),
            "collisions": snapshot.collisions, "hit_location": vec(snapshot.hit_location), "hit_normal": vec(snapshot.hit_normal)}})
        return snapshot

    def fake_uniform(a, b):
        assert (a, b) == (0.0, 360.0)
        value = rng.uniform(a, b)
        yaws.append(value)
        return value

    saved = (res.read_camera, rp.read_projectile, global_random.uniform)
    res.read_camera, rp.read_projectile, global_random.uniform = fake_camera, fake_read_projectile, fake_uniform
    states = ["InactiveState", "EquippedState", "EquippedState", "ProjectileThrowState", "CommitTime", "AirBendingState",
              "AirBendingState", "InactiveState", None, "CommitTime", "IdleState", "EquippedState", "AirBendingState",
              "ProjectileThrowState", "InactiveState"]
    datagrams = []
    now = 7000.0
    try:
        for k in range(1500):
            now += rng.choice([0.004, 0.008, 0.016, 0.031, 0.05, 0.09, 0.2])
            event = {}
            predicted = states[(k // 3) % len(states)]
            roll = rng.random()
            view = ((rng.uniform(3500, 6000), rng.uniform(-15000, -12000), rng.uniform(1300, 1500)),
                    (rng.uniform(-60, 80), rng.uniform(-180, 180), 0.0))
            camera["value"] = None if roll < 0.06 else "raise" if roll < 0.1 else view
            reads_before, yaws_before = len(reads), len(yaws)
            cloudburst_state(source, predicted, now, event, harness.send_reliable, reader)
            state_sends = harness.take_sends()
            harness.ns.update(now=now, event=event, state_reader=reader)
            exec(tick_code, harness.ns)
            # camera row: x y z pitch yaw (none when the read gave none or raised)
            datagrams.append({"now": now, "predicted": predicted,
                              "camera": None if not isinstance(camera["value"], tuple) else row(*vec(view[0]), view[1][0], view[1][1]),
                              "state_sends": state_sends, "tick_sends": harness.take_sends(),
                              "reads": reads[reads_before:], "yaws": yaws[yaws_before:], "event": jsonable_event(event)})
    finally:
        res.read_camera, rp.read_projectile, global_random.uniform = saved
    field = harness.ns["projectile_fields"][source]
    return {"lines": {"cloudburst_state": state_line, "flight_tick": tick_line}, "first_offsets": first_offsets_json(),
            "datagrams": datagrams,
            "final": {"next_channel": field.next_channel, "next_guid": field.next_guid, "flights": len(field.flights)}}


def section_world_ability_systems():
    """the ability job block (ruh:2066-2078) with advance_ability_job,
    plan_extra_machines and send_extra_machines (ruh:1102-1170) per datagram."""
    import range_server_bunch as rsb
    import read_equippable_state as res
    from range_equippables import starting_ability_resources

    jobs_code, jobs_line = if_statement("ability_activation and source in melee_inactive_state_sent_sources")
    harness = Harness(ability_activation=True, starting_equippable_index={}, ability_jobs={}, ability_allocators={},
                      ability_ready_equippables={}, equippable_mirror_states={},
                      starting_ability_equippables=starting_ability_resources(),
                      wushu_starting_equippables=rsb.wushu_starting_equippables)
    source = harness.source
    harness.ns["melee_inactive_state_sent_sources"] = {source}
    harness.ns["starting_equippable_index"][source] = len(rsb.wushu_starting_equippables)
    for name in ("advance_ability_job", "plan_extra_machines", "send_extra_machines"):
        nested_function(name, harness.ns)
    reader = FakeReader()

    def names(prefix, count, inactive_at):
        out = [f"{prefix}State{i}" for i in range(count - 1)]
        out.insert(inactive_at, "InactiveState")
        return out

    # the live shapes (responder_24252): smoke 11 states, dash 15 + StateMachine_Priming,
    # Q 12 + BoostStateMachine, X 35 (never registers here: it gives up)
    world = {
        3: {"class": "Ability_Wushu_4_Smoke_C", "item": 0x7000, "machine": 0x7100, "states": names("Smoke", 11, 2),
            "wrong_first": 0, "slot_error_first": 0, "items": "error"},
        5: {"class": "Ability_Wushu_E_Dash_C", "item": 0x8000, "machine": 0x8100, "states": names("Dash", 15, 0),
            "wrong_first": 2, "slot_error_first": 0,
            "items": [("EquippableStateMachine", 0x8100, "DashState1", None),
                      ("StateMachine_Priming", 0x8200, "Prime_Idle", ["Prime_Idle", "Prime_Priming", "Prime_Primed", "Prime_Idle"]),
                      ("NoStartMachine", 0x8300, None, ["A", "B"]),
                      ("EmptyStartMachine", 0x8400, "", ["C"])]},
        4: {"class": "Ability_Wushu_Q_CycloneBoost_C", "item": 0x9000, "machine": 0x9100, "states": names("Boost", 12, 11),
            "wrong_first": 0, "slot_error_first": 1,
            "items": [("BoostStateMachine", 0x9200, "Boost_Ready", ["Boost_Ready", "Boost_Active", "Boost_Done"])]},
        9: {"class": "Ability_Wushu_X_Dagger_Production_C", "item": 0xA000, "machine": 0xA100, "states": names("Dagger", 35, 5),
            "wrong_first": 10 ** 9, "slot_error_first": 0, "items": []},
    }
    calls = {}
    oracle = []

    def fake_slots(state_reader, slots):
        assert len(slots) == 1
        slot = slots[0]
        spec = world[slot]
        calls[slot] = calls.get(slot, 0) + 1
        if calls[slot] <= spec["slot_error_first"]:
            oracle.append({"call": "slot", "slot": slot, "error": True})
            raise OSError("slot read failed")
        if calls[slot] <= spec["wrong_first"]:
            # the slot still holds another class (or nothing readable)
            result = {} if calls[slot] % 2 else {slot: res.SlotMachineState(slot, spec["item"], "Ability_Melee_Base_C", spec["machine"],
                                                                            None, None, False)}
        else:
            state_reader._names[spec["machine"]] = {name: spec["machine"] + 0x10 * (i + 1) for i, name in enumerate(spec["states"])}
            result = {slot: res.SlotMachineState(slot, spec["item"], spec["class"], spec["machine"], None, None, False)}
        machine = result.get(slot)
        oracle.append({"call": "slot", "slot": slot, "machine": None if machine is None else {
            "item": machine.item, "class_name": machine.class_name, "state_machine": machine.state_machine,
            "state_names": list(state_reader._names.get(machine.state_machine, {}))}})
        return result

    def fake_items(state_reader, item):
        spec = next(s for s in world.values() if s["item"] == item)
        if spec["items"] == "error":
            oracle.append({"call": "items", "item": item, "error": True})
            raise OSError("item machines read failed")
        machines = []
        for name, address, starting, state_names in spec["items"]:
            state_names = state_names if state_names is not None else spec["states"]
            machines.append(res.MachineState(name=name, address=address, predicted=None, authoritative=None,
                                             starting=starting, state_names=tuple(dict.fromkeys(state_names))))
        oracle.append({"call": "items", "item": item, "machines": [
            {"name": m.name, "address": m.address, "starting": m.starting, "state_names": list(m.state_names)} for m in machines]})
        return machines

    saved = (res.read_slot_machines, res.read_item_machines)
    res.read_slot_machines, res.read_item_machines = fake_slots, fake_items
    rng = random.Random(1170)
    datagrams = []
    now = 9000.0
    try:
        # fine spacing so every 0.25/0.4/0.3 s deadline is crossed by a datagram within ms
        for _ in range(1400):
            now += rng.choice([0.002, 0.004, 0.006, 0.008, 0.011, 0.016])
            event = {}
            before = len(oracle)
            harness.ns.update(now=now, event=event, state_reader=reader)
            exec(jobs_code, harness.ns)
            datagrams.append({"now": now, "sends": harness.take_sends(), "oracle": oracle[before:], "event": jsonable_event(event)})
    finally:
        res.read_slot_machines, res.read_item_machines = saved
    ready = harness.ns["ability_ready_equippables"].get(source, {})
    allocator = harness.ns["ability_allocators"][source]
    return {"lines": {"jobs": jobs_line}, "first_offsets": first_offsets_json(), "datagrams": datagrams,
            "ready": [{"key": e.key, "class_name": e.class_name, "channel": e.channel, "actor_guid": e.actor_guid,
                       "slot": e.slot, "state_machine_guid": e.state_machine_guid,
                       "state_guids": [[n, g] for n, g in e.state_guids.items()], "mirror_states": e.mirror_states,
                       "extra": [{"name": x.name, "guid": x.guid, "starting_state": x.starting_state,
                                  "state_guids": [[n, g] for n, g in x.state_guids.items()]} for x in e.extra_machines]}
                      for e in ready.values()],
            "mirror_states": sorted(key for _, key in harness.ns["equippable_mirror_states"]),
            "jobs": [{"stage": j["stage"], "registered_tries": j["registered_tries"]} for j in harness.ns["ability_jobs"][source]],
            "next_guid": allocator.next_guid}


# ---------------------------------------------------------------- debug queue, B18 hook, firing state

def if_in_loop(test_fragment):
    """if_statement, wrapped in a one-pass loop so the `continue` it ends with compiles."""
    text, tree = ruh_source()
    found = [node for node in ast.walk(tree)
             if isinstance(node, ast.If) and test_fragment in ast.get_source_segment(text, node.test)]
    if len(found) != 1:
        raise LookupError(f"{test_fragment!r}: {len(found)} matches")
    node = found[0]
    body = textwrap.dedent("\n".join(text.splitlines()[node.lineno - 1:node.end_lineno]))
    source = "for _queued_once in (0,):\n" + textwrap.indent(body, "    ")
    return compile(source, f"ruh:{node.lineno}", "exec"), node.lineno


def section_world_bot_queue():
    """the debug queue's spawn_bot (ruh:1958-1971) in production order per
    datagram: the mirror poll's shot, the command queue, the bot tick."""
    import range_bots as rb
    import read_equippable_state as res

    tick_code, tick_line = if_statement("range_bots and source in range_streaming_sent_sources")
    spawn_code, spawn_line = if_in_loop('"spawn_bot" in command')
    bot_count, _ = argparse_default("--range-bot-count")
    harness = Harness(range_bots=True, bot_fields={}, bot_ticks={}, bot_spawned_at={}, range_bot_count=bot_count)
    source = harness.source
    harness.ns["range_streaming_sent_sources"] = {source}
    bot_shots, _ = nested_function("bot_shots", harness.ns)
    reader = FakeReader()
    camera = {}
    saved = res.read_camera
    res.read_camera = lambda handle, image_base, controller=None: camera.get("value")
    rng = random.Random(1971)
    classes = [name for name, _ in sorted(rb.weapon_damage.items())]
    datagrams = []
    now = 6000.0
    try:
        for k in range(600):
            now += rng.choice([0.004, 0.016, 0.05, 0.12, 0.26])
            event = {}
            shot = None
            field = harness.ns["bot_fields"].get(source)
            if field is not None and rng.random() < 0.4:
                origin = (4825.0 + rng.uniform(-250, 250), -12600.0 + rng.uniform(-400, 300), 1371.8 + rng.uniform(-40, 40))
                live = [b for b in field.bots if b.alive and b.opened]
                if live and rng.random() < 0.85:
                    bot = rng.choice(live)
                    target = (bot.location[0] + rng.uniform(-10, 10), bot.location[1] + rng.uniform(-10, 10),
                              bot.location[2] + rng.choice([75.0, 20.0, -60.0]))
                else:
                    target = (origin[0] + rng.uniform(-3000, 3000), origin[1] - rng.uniform(100, 3000), origin[2])
                dx, dy, dz = (target[i] - origin[i] for i in range(3))
                yaw = math.degrees(math.atan2(dy, dx))
                pitch = math.degrees(math.atan2(dz, math.hypot(dx, dy)))
                camera["value"] = (origin, (pitch, yaw, 0.0))
                class_name = rng.choice(classes)
                bot_shots(source, class_name, 1, now, event, harness.send_reliable, reader)
                shot = {"origin": vec(origin), "pitch": pitch, "yaw": yaw, "class_name": class_name}
            shot_sends = harness.take_sends()
            # the first datagram queues a bot before any tick: the queue makes the
            # field, and the dummy then takes the channel after it
            queued = []
            if k == 0 or rng.random() < 0.07:
                for _ in range(rng.choice([1, 1, 2])):
                    location = [rng.uniform(3000, 7000), rng.uniform(-16000, -11000), rng.choice([1351.8, rng.uniform(1200, 1500)])]
                    if rng.random() < 0.25:
                        location = [int(v) for v in location]   # json ints, float() in ruh
                    harness.ns.update(now=now, event=event, command={"spawn_bot": location})
                    exec(spawn_code, harness.ns)
                    queued.append(vec(location))
            queue_sends = harness.take_sends()
            harness.ns.update(now=now, event=event)
            exec(tick_code, harness.ns)
            datagrams.append({"now": now, "shot": shot, "shot_sends": shot_sends, "queued": queued,
                              "queue_sends": queue_sends, "tick_sends": harness.take_sends(), "event": jsonable_event(event)})
    finally:
        res.read_camera = saved
    field = harness.ns["bot_fields"][source]
    return {"lines": {"spawn_bot": spawn_line, "bot_tick": tick_line}, "first_offsets": first_offsets_json(),
            "datagrams": datagrams,
            "final": {"bots": [bot_json(b) for b in field.bots], "next_channel": field.next_channel,
                      "next_guid": field.next_guid, "rng_next": field.rng.random()}}


def section_world_ability_open_hook():
    """the B18 tuning hook (ruh:4045-4068) run as written with the process tag
    cache already filled: what it leaves in open_body for every starting index,
    and its event."""
    import range_abilities as ra
    from gameplay_tag_net_index import TagNetIndex

    code, line = if_statement("ability_activation and equip_i in starting_ability_tunings")
    # stands for build_starting_equippable_open (the rep crate's): the hook's fallback
    default = (b"\xde\xfa\x17", 23)
    sample08 = {"AbilityTuning.Wushu.4.SmokeDuration": 240, "AbilityTuning.Wushu.4.SmokeMaxCharges": 241,
                "AbilityTuning.Wushu.E.DashVelocityMultiplier": 243, "AbilityTuning.Wushu.E.MaxCharges": 244,
                "AbilityTuning.Wushu.E.PrimeDuration": 245, "AbilityTuning.Wushu.Q.MaxCharges": 250}
    # 04_bringup_b.md section 11 open_tuning_SAMPLE_TAGS
    sample04 = {"AbilityTuning.Wushu.4.SmokeDuration": 100, "AbilityTuning.Wushu.4.SmokeMaxCharges": 101,
                "AbilityTuning.Wushu.E.DashVelocityMultiplier": 243, "AbilityTuning.Wushu.E.PrimeDuration": 245,
                "AbilityTuning.Wushu.E.MaxCharges": 244, "AbilityTuning.Wushu.Q.MaxCharges": 300}
    no_prime = {k: v for k, v in sample08.items() if "PrimeDuration" not in k}
    wide = {name: 40000 + 7 * i for i, name in enumerate(sorted(sample08))}
    # (label, activation, indices, true_bit_num, first_bit_segment, constructed)
    caches = [("spec08_sample", True, sample08, 10, 10, True), ("spec04_sample", True, sample04, 10, 10, True),
              ("unconstructed", True, sample08, 10, 10, False), ("segmented", True, sample08, 10, 8, True),
              ("negative_segment", True, sample08, 10, -3, True), ("eight_bits", True, sample04, 8, 8, True),
              ("missing_prime", True, no_prime, 10, 10, True), ("sixteen_bits", True, wide, 16, 16, True),
              ("off", False, sample08, 10, 10, True)]
    out = []
    for label, activation, indices, true_bits, first_segment, constructed in caches:
        rows = []
        for equip_i in range(7):
            event = {}
            ns = {"ability_activation": activation, "equip_i": equip_i,
                  "starting_ability_tunings": ra.starting_ability_tunings,
                  "tag_net_index_cache": {"indices": TagNetIndex(dict(indices), true_bits, first_segment, constructed)},
                  "open_body": default[0], "open_bits": default[1], "event": event}
            exec(code, ns)
            body = (ns["open_body"], ns["open_bits"])
            rows.append({"index": equip_i, **({"default": True} if body == default else hb(body)),
                         "event": jsonable_event(event)})
        out.append({"label": label, "activation": activation, "indices": dict(indices), "true_bit_num": true_bits,
                    "first_bit_segment": first_segment, "constructed": constructed, "rows": rows})
    return {"line": line, "caches": out}


def section_world_firing_seed():
    """read_firing_seed (ruh:946-964) run as written on fake reads that make
    the seed equal the address of the firing state it picked."""
    import capture_input_event_cursor as cec

    harness = Harness(firing_rng_components={})
    read_firing_seed, line = nested_function("read_firing_seed", harness.ns)
    rng_base = 1 << 40

    def fake_u64(handle, address):
        return address - 0x4A0 + rng_base           # the RNG component of state `address - 0x4A0`

    def fake_u32(handle, address):
        return address - 0xEC - rng_base             # its seed: the state address again

    class Machine:
        state_machine = 0xABC000

    class Equippable:
        key = "BasePistol"

    cases = [[("InactiveState", 0x1000), ("FiringState", 0x2000)],
             [("FiringBurst", 0x3000), ("FiringState", 0x2000)],
             [("FiringBurst", 0x3000), ("Idle", 0x10)],
             [("FiringState", 0), ("FiringAlt", 0x4000)],
             [("FiringAlt", 0x4000), ("FiringState", 0)],
             [],
             [("Firing", 0x5000)],
             [("firingstate", 0x6000), ("FIRINGState", 0x6100)],
             [("FiringState", 0x7000), ("Idle", 1), ("FiringState", 0x8000)],
             [("FiringAlt", 0x9000), ("FiringState", 0x7000), ("FiringAlt", 0)],
             [("FiringAlt", 0x9000), ("FiringState", 0), ("FiringAlt", 0)],
             [("Idle", 0x10), ("ReloadState", 0x20)]]
    r = random.Random(964)
    pool = ["FiringState", "FiringAlt", "Firing", "FiringBurst", "InactiveState", "EquipState", "ReloadState", "firing"]
    for _ in range(300):
        cases.append([(r.choice(pool), r.choice([0, 0x100 * r.randrange(1, 4096)])) for _ in range(r.randint(0, 7))])
    out = []
    saved = (cec.read_u32, cec.read_u64)
    cec.read_u32, cec.read_u64 = fake_u32, fake_u64
    try:
        for pairs in cases:
            reader = FakeReader()
            reader._names[Machine.state_machine] = dict(pairs)
            harness.ns["firing_rng_components"] = {}
            seed = read_firing_seed(harness.source, Equippable, Machine, reader)
            out.append({"states": [[name, address] for name, address in pairs], "address": seed})
    finally:
        cec.read_u32, cec.read_u64 = saved
    return {"line": line, "cases": out}


SECTIONS = {
    "world_abilities": section_world_abilities,
    "world_bot_queue": section_world_bot_queue,
    "world_ability_open_hook": section_world_ability_open_hook,
    "world_firing_seed": section_world_firing_seed,
    "world_bot_systems": section_world_bot_systems,
    "world_projectile_systems": section_world_projectile_systems,
    "world_ability_systems": section_world_ability_systems,
    "world_tags": section_world_tags,
    "world_tags_live": section_world_tags_live,
    "world_bots": section_world_bots,
    "world_projectiles": section_world_projectiles,
    "world_teams": section_world_teams,
}
