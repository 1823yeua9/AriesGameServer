"""oracle read sections: every production live-client read (spec 09), run by the
real python readers against synthetic process images.

ctypes.windll.kernel32 is swapped for a fake whose ReadProcessMemory serves
4 KiB pages of a scripted address space (unmapped page -> the call fails, like
a real RPM), so no live process is touched. Each scenario is one ordered list
of steps: memory ops ("map", "write") and reader calls with the python result.
The rust test replays the same steps into FakeMemory and compares.

Inline responder code (read_firing_seed, the hud refresh and ultimate blocks)
is pulled out of range_udp_handshake.py with ast and executed as-is.
"""

import ast
import ctypes
import math
import struct
import sys
import textwrap
import types
from pathlib import Path

MASK64 = (1 << 64) - 1
PAGE = 0x1000
IMAGE_BASE = 0x7FF6A0000000
OWNER = IMAGE_BASE + 0xD3E7140      # RWS:10
POOL = IMAGE_BASE + 0xD548540       # FNP:20
HUD_THUNK = IMAGE_BASE + 0x4B6B340  # H:3784
NAME_KEY = bytes.fromhex("5ac3179e")
ARRAY = 0x1A0000000
CHUNK_TABLE = 0x1A0001000
CHUNK0 = 0x1B0000000
CHUNK1 = 0x1C0000000
NAME_BLOCK = 0x1D0000000
UNMAPPED = 0x5000000000
SEED = 0x2468ACE5  # decodes in slot 4


class Image:
    """sparse 4 KiB-page address space; every op is also appended to steps."""

    def __init__(self, steps):
        self.pages = {}
        self.steps = steps

    def map(self, address, size):
        self.steps.append({"op": "map", "address": address, "size": size})
        for page in range(address & ~(PAGE - 1), address + size, PAGE):
            self.pages.setdefault(page, bytearray(PAGE))

    def write(self, address, data):
        data = bytes(data)
        self.steps.append({"op": "write", "address": address, "hex": data.hex()})
        for i, value in enumerate(data):
            page = (address + i) & ~(PAGE - 1)
            self.pages.setdefault(page, bytearray(PAGE))[(address + i) - page] = value

    def read(self, address, size):
        out = bytearray()
        for i in range(size):
            page = self.pages.get((address + i) & ~(PAGE - 1))
            if page is None:
                return None
            out.append(page[(address + i) & (PAGE - 1)])
        return bytes(out)

    def u8(self, a, v): self.write(a, struct.pack("<B", v))
    def u16(self, a, v): self.write(a, struct.pack("<H", v))
    def u32(self, a, v): self.write(a, struct.pack("<I", v & 0xFFFFFFFF))
    def i32(self, a, v): self.write(a, struct.pack("<i", v))
    def u64(self, a, v): self.write(a, struct.pack("<Q", v & MASK64))
    def f64s(self, a, values): self.write(a, struct.pack(f"<{len(values)}d", *values))


class FakeKernel:
    """stands in for ctypes.windll.kernel32 while a scenario runs."""

    def __init__(self, image):
        self.image = image

    def __enter__(self):
        self.saved = ctypes.windll.kernel32
        image = self.image
        fake = types.SimpleNamespace()

        def open_process(access, inherit, pid):
            return 0x4242

        def close_handle(handle):
            return 1

        def read_process_memory(handle, address, buffer, size, count):
            address = getattr(address, "value", address) or 0
            data = image.read(address, int(size))
            if data is None:
                count._obj.value = 0
                return 0
            ctypes.memmove(buffer, data, len(data))
            count._obj.value = len(data)
            return 1

        fake.OpenProcess = open_process
        fake.CloseHandle = close_handle
        fake.ReadProcessMemory = read_process_memory
        ctypes.windll.kernel32 = fake
        return self

    def __exit__(self, *exc):
        ctypes.windll.kernel32 = self.saved


class Names:
    """the encrypted FName pool (FNP:25-40): one block, 4-byte entry stride."""

    def __init__(self, image):
        self.image = image
        self.next_index = 1
        self.ids = {}
        image.map(POOL, 0x40)
        image.u64(POOL + 0x10, NAME_BLOCK)
        image.map(NAME_BLOCK, 0x4000)

    def raw(self, length, wide, plain):
        index = self.next_index
        entry = NAME_BLOCK + 4 * index
        self.image.u16(entry + 4, (length << 1) | int(wide))
        if length < 256:
            data = bytes(b ^ length ^ NAME_KEY[i & 3] for i, b in enumerate(plain))
        else:
            data = bytes((b ^ NAME_KEY[i & 3]) & 0xFF for i, b in enumerate(plain))
        self.image.write(entry + 6, data)
        self.next_index = (4 * index + 6 + len(data) + 3) // 4
        return index

    def add(self, text, wide=False):
        if (text, wide) in self.ids:
            return self.ids[(text, wide)]
        plain = text.encode("utf-16-le" if wide else "utf-8")
        index = self.raw(len(plain) // 2 if wide else len(plain), wide, plain)
        self.ids[(text, wide)] = index
        return index

    def empty(self):
        """an index whose header is zero (length 0)."""
        index = self.next_index
        self.next_index += 2
        return index


def encode_owner(target):
    from read_world_state import decode_array_pointer
    seed = SEED
    mixed_seed = seed ^ ((seed ^ (seed >> 15)) >> 12) ^ ((seed << 25) & 0xFFFFFFFF)
    mixed = (0x2545F4914F6CDD1D * mixed_seed) & MASK64
    slot, high = mixed % 7, mixed >> 32
    assert slot == 4
    owner = bytearray(0x80)
    struct.pack_into("<I", owner, 0x78, seed)
    struct.pack_into("<Q", owner, 0x40 + 8 * slot, (target ^ seed) ^ ((high + 8) & 0xFFFFFFFF))
    assert decode_array_pointer(bytes(owner)) == target
    return bytes(owner)


class Client:
    """a synthetic 13.06 client: every object a production read walks."""

    def __init__(self, steps, num_elements=0x20000, num_chunks=2):
        image = self.image = Image(steps)
        self.names = Names(image)
        self.next_object = 0x200000000
        self.classes = {}
        self.a = {}
        image.write(OWNER, encode_owner(ARRAY))
        image.map(ARRAY, 0x20)
        image.u64(ARRAY, CHUNK_TABLE)
        image.u32(ARRAY + 0x10, 0x400000)
        image.u32(ARRAY + 0x14, num_elements)
        image.u32(ARRAY + 0x18, 0x14)
        image.u32(ARRAY + 0x1C, num_chunks)
        image.map(CHUNK_TABLE, 8 * max(num_chunks, 2))
        for chunk in range(max(num_chunks, 2)):
            image.u64(CHUNK_TABLE + 8 * chunk, CHUNK0 + chunk * 0x10000000 if chunk < 2 else 0x1E0000000)
        # tag scan window (GTN:48) in chunk 0, engine slot in chunk 1
        image.map(CHUNK0 + 24 * 0xEA40, 24 * (0xFB00 - 0xEA40))
        image.map(CHUNK1 + 24 * 0x07E2, 24)
        self.build()

    def alloc(self, size=0x1000):
        address = self.next_object
        self.next_object += max(0x10000, (size + 0xFFFF) & ~0xFFFF)
        self.image.map(address, size)
        return address

    def uclass(self, name, index=0):
        if name not in self.classes:
            cls = self.alloc()
            self.image.u32(cls + 0x18, self.names.add(name))
            self.image.u32(cls + 0xC, index)
            self.classes[name] = cls
        return self.classes[name]

    def uobject(self, name=None, cls=None, size=0x1000, index=0):
        obj = self.alloc(size)
        if name is not None:
            self.image.u32(obj + 0x18, self.names.add(name))
        if cls is not None:
            self.image.u64(obj + 0x10, self.uclass(cls))
        self.image.u32(obj + 0xC, index)
        return obj

    def state_machine(self, own_name, states, predicted, authoritative, starting):
        """RES:77-87 TMap (24-byte elements, state object first), +0x1B0/+0x1C8/+0x1E8."""
        im = self.image
        sm = self.uobject(own_name)
        data = self.alloc(24 * max(len(states), 1))
        pointers = {}
        for i, name in enumerate(states):
            if name is None:
                continue
            state = self.uobject(name, size=0x1000)
            pointers.setdefault(name, []).append(state)
            im.u64(data + 24 * i, state)
        im.u64(sm + 0x160, data)
        im.u32(sm + 0x168, len(states))
        for offset, name in ((0x1B0, predicted), (0x1C8, authoritative), (0x1E8, starting)):
            if name is not None:
                im.u64(sm + offset, pointers[name][-1])
        return sm, data, pointers

    def build(self):
        im, names, a = self.image, self.names, self.a
        engine = a["engine"] = self.uobject("AresGameEngine", "AresGameEngine", index=0x107E2)
        im.u64(CHUNK1 + 24 * 0x07E2, engine)
        viewport = a["viewport"] = self.uobject()
        world = a["world"] = self.uobject()
        game_instance = a["game_instance"] = self.uobject()
        players = self.alloc(0x10)
        local_player = a["local_player"] = self.uobject()
        controller = a["controller"] = self.uobject("PlayerController_0", "AresPlayerController_C", index=0x2000)
        im.u32(self.classes["AresPlayerController_C"] + 0xC, 0x3EB21)
        pawn = a["pawn"] = self.uobject("Wushu_PC_C_0", "Wushu_PC_C", size=0x2000)
        im.u32(self.classes["Wushu_PC_C"] + 0xC, 0x3F000)
        im.u64(engine + 0xBD0, viewport)
        im.u64(viewport + 0x80, world)
        im.u64(world + 0x1D8, game_instance)
        im.u64(game_instance + 0x40, players)
        im.u64(players, local_player)
        im.u64(local_player + 0x38, controller)
        im.u64(controller + 0x4B0, pawn)
        # camera: PlayerCameraManager POV (RES:66-74)
        manager = a["camera_manager"] = self.uobject(size=0x2000)
        im.u64(controller + 0x528, manager)
        im.f64s(manager + 0x17C0, (3581.5, -12019.25, 1360.0, -4.5, 92.25, 0.0))
        # hud widget + cinematic flags (H:2001-2004)
        widget = a["widget"] = self.uobject()
        im.u64(pawn + 0xB80, widget)
        im.u8(widget + 0x11C, 1)
        im.u32(controller + 0x690, 0x1)
        im.i32(pawn + 0x14F4, 8)
        self.build_inventory()
        self.build_input()
        self.build_moves()
        self.build_net_cache()
        self.build_projectile()
        self.build_tags()

    def build_inventory(self):
        im, a = self.image, self.a
        pawn = a["pawn"]
        inventory = a["inventory"] = self.uobject(size=0x1000)
        im.u64(pawn + 0xBF8, inventory)
        im.u32(inventory + 0x2F0, 9)

        def slot(index, item):
            item_slot = self.uobject()
            im.u64(inventory + 0x1B8 + 8 * index, item_slot)
            im.u64(item_slot + 0x38, item)
            return item_slot

        # slot 1: the classic, main machine + zoom machine, ammo components
        pistol = a["pistol"] = self.uobject("BasePistol_C_0", "BasePistol_C", size=0x2000)
        sm, _, states = self.state_machine(
            "EquippableStateMachine",
            ["InactiveState", "EquipState", "IdleState", "FiringState", "ReloadState"],
            "FiringState", "IdleState", "InactiveState")
        a["pistol_sm"], a["pistol_states"] = sm, states
        zoom, _, zoom_states = self.state_machine(
            "ZoomStateMachine", ["ZoomStateInactive", "ZoomStateUnzoomed", "ZoomStateLevel1"],
            "ZoomStateUnzoomed", "ZoomStateInactive", "ZoomStateInactive")
        a["zoom_sm"] = zoom
        im.u64(pistol + 0xE00, sm)
        machines = self.alloc(0x40)
        im.u64(machines, sm)
        im.u64(machines + 8, 0)
        im.u64(machines + 16, zoom)
        im.u64(pistol + 0xE30, machines)
        im.u32(pistol + 0xE38, 3)
        a["pistol_machines"] = machines
        rng = a["rng"] = self.uobject()
        im.u64(states["FiringState"][0] + 0x4A0, rng)
        im.u32(rng + 0xEC, 0xC0FFEE11)
        magazine = a["magazine"] = self.uobject("MagazineAmmo")
        im.write(magazine + 0x10C, struct.pack("<ii", 11, 12))
        im.i32(magazine + 0x12C, 12)
        reserve = a["reserve"] = self.uobject("ReserveAmmo")
        im.write(reserve + 0x10C, struct.pack("<ii", 36, 36))
        im.i32(reserve + 0x12C, 36)
        im.u64(pistol + 0x1208, magazine)
        im.u64(pistol + 0x1200, reserve)
        slot(1, pistol)
        im.u64(inventory + 0x278, pistol)
        im.u64(inventory + 0x260, 0)
        # slot 2: knife with a machine whose predicted pointer is not a state
        knife = a["knife"] = self.uobject("Melee_0", "Ability_Melee_Base_C", size=0x2000)
        knife_sm, _, _ = self.state_machine(
            "EquippableStateMachine", ["InactiveState", "EquipState", None, "IdleState"],
            None, "InactiveState", "InactiveState")
        im.u64(knife_sm + 0x1B0, UNMAPPED)
        im.u64(knife + 0xE00, knife_sm)
        a["knife_sm"] = knife_sm
        slot(2, knife)
        # slot 3: cloudburst, its machine has an empty TMap at first (Q4 race)
        smoke = a["smoke"] = self.uobject("Smoke_0", "Ability_Wushu_4_Smoke_C", size=0x2000)
        smoke_sm, smoke_data, _ = self.state_machine("EquippableStateMachine", [], None, None, None)
        im.u64(smoke + 0xE00, smoke_sm)
        a["smoke_sm"], a["smoke_data"] = smoke_sm, smoke_data
        charge = a["smoke_charge"] = self.uobject("EquipmentCharge")
        im.write(charge + 0x10C, struct.pack("<ii", 1, 2))
        im.i32(charge + 0x188, 2)
        im.u64(smoke + 0x1240, charge)
        slot(3, smoke)
        # slot 4: updraft with no state machine (class still cached)
        updraft = a["updraft"] = self.uobject("Updraft_0", "Ability_Wushu_Q_Updraft_C", size=0x2000)
        slot(4, updraft)
        # slot 5: no item slot; slot 9: an empty item slot
        slot(9, 0)

    def build_input(self):
        im, a = self.image, self.a
        component = a["input"] = self.uobject(size=0x2000)
        im.u64(a["pawn"] + 0xB90, component)
        im.u8(component + 0x12C, 1)
        im.u32(component + 0x130, 11)
        im.u32(component + 0xA84, 1000)
        im.u32(component + 0x1198, 7)

    def build_moves(self):
        im, a = self.image, self.a
        pawn, controller = a["pawn"], a["controller"]
        movement = a["movement"] = self.uobject(size=0x2000)
        vtable = self.alloc(0x1000)
        im.u64(movement, vtable)
        im.u64(vtable + 2944, IMAGE_BASE + 0x747DAB0)
        im.u64(vtable + 2920, IMAGE_BASE + 0x747D000)
        im.u64(pawn + 0x4F8, movement)
        im.u8(pawn + 0x224, 2)
        im.u8(pawn + 0x128, 3)
        im.u64(pawn + 0x4A0, controller)
        im.u64(controller + 0x518, pawn)
        im.u64(controller + 0x510, 0x40)
        im.u8(movement + 0xC1, 0x20)
        im.u8(movement + 0x13D8, 0)
        pd = a["prediction"] = self.uobject()
        im.u64(movement + 0x7E8, pd)
        array = a["saved_array"] = self.alloc(0x1000)
        move_vtable = self.alloc(0x100)
        im.u64(move_vtable + 16, IMAGE_BASE + 0x7470000)
        a["moves"] = []
        for i in range(300):
            move = self.alloc(0x1000) if i < 3 else None
            a["moves"].append(move)
        for i, number in enumerate((201, 202, 203)):
            move = a["moves"][i]
            im.u64(move, move_vtable)
            im.u32(move + 0x1C, number)
            im.write(move + 0x3C, struct.pack("<f", 1.5 + i))
            im.f64s(move + 1080, (0.0, 1.0, 0.0))
            im.f64s(move + 592, (3490.0, -12020.0 + i, 1294.15))
            im.u64(array + 16 * i, move)
        im.u64(pd + 0x20, array)
        im.u32(pd + 0x28, 3)
        im.u32(pd + 0x68, 96)
        last = self.alloc(0x100)
        im.u32(last + 0x1C, 200)
        im.u64(pd + 0x50, last)
        a["move_vtable"] = move_vtable

    def build_net_cache(self):
        """FClassNetCacheMgr TSet (RRI:80-118): child -> parent chain."""
        im, a = self.image, self.a
        driver = a["net_driver"] = self.uobject(size=0x1000)
        im.u64(a["world"] + 0x40, driver)
        manager = a["cache_mgr"] = self.uobject()
        im.u64(driver + 0x198, manager)
        entries = a["cache_set"] = self.alloc(0x1000)
        im.u64(manager + 8, entries)
        im.u32(manager + 0x10, 3)
        hud_fn = self.uobject()
        im.u64(hud_fn + 0xE0, HUD_THUNK)
        other_fn = self.uobject()
        im.u64(other_fn + 0xE0, IMAGE_BASE + 0x6244D00)
        pawn_fn = self.uobject()
        im.u64(pawn_fn + 0xE0, IMAGE_BASE + 0x1234560)
        a["hud_fn"], a["other_fn"] = hud_fn, other_fn
        # parent: base 0, count 150, ClientSetHUD at entry 52
        parent = a["parent_cache"] = self.uobject()
        parent_fields = self.alloc(24 * 150)
        im.u32(parent, 0)
        im.u64(parent + 0x20, parent_fields)
        im.u32(parent + 0x28, 150)
        im.u64(parent_fields + 24 * 52, hud_fn | 1)
        im.u32(parent_fields + 24 * 52 + 8, 52)
        im.u64(parent_fields + 24 * 60, other_fn)  # untagged: not a function
        im.u32(parent_fields + 24 * 60 + 8, 60)
        # child: base 150, count 28
        child = a["child_cache"] = self.uobject()
        child_fields = self.alloc(24 * 28)
        im.u32(child, 150)
        im.u64(child + 8, parent)
        im.u64(child + 0x20, child_fields)
        im.u32(child + 0x28, 28)
        im.u64(child_fields + 24 * 3, other_fn | 1)
        im.u32(child_fields + 24 * 3 + 8, 153)
        a["child_fields"], a["parent_fields"] = child_fields, parent_fields
        # pawn class cache: one function
        pawn_cache = a["pawn_cache"] = self.uobject()
        pawn_fields = self.alloc(24 * 4)
        im.u32(pawn_cache, 40)
        im.u64(pawn_cache + 0x20, pawn_fields)
        im.u32(pawn_cache + 0x28, 4)
        im.u64(pawn_fields + 24 * 2, pawn_fn | 1)
        im.u32(pawn_fields + 24 * 2 + 8, 42)
        im.u64(entries, 0x0000000500012345)
        im.u64(entries + 8, 0)
        im.u64(entries + 24, 0x0000000700000000 | 0x3EB21)
        im.u64(entries + 32, child)
        im.u64(entries + 48, 0x0000000900000000 | 0x3F000)
        im.u64(entries + 56, pawn_cache)

    def build_projectile(self):
        im, a = self.image, self.a
        connection = a["connection"] = self.uobject(size=0x2000)
        im.u64(a["net_driver"] + 0xC0, connection)
        channels = a["channels"] = self.alloc(8 * 10010)
        im.u64(connection + 0x13F8, channels)
        im.u32(connection + 0x1400, 10010)
        actor = a["projectile"] = self.uobject("Projectile_0", "Projectile_Wushu_4_Smoke_C", size=0x1000)
        channel = self.uobject()
        im.u64(channel + 0x78, actor)
        im.u64(channels + 8 * 10000, channel)
        root = self.uobject()
        im.u64(actor + 0x290, root)
        im.f64s(root + 0x320, (3700.0, -11900.0, 1300.0))
        movement = a["projectile_movement"] = self.uobject()
        im.u64(actor + 0x610, movement)
        im.u32(movement + 0x198, 1)
        im.f64s(movement + 0x228, (3710.0, -11890.0, 1253.8))
        im.f64s(movement + 0x258, (0.0, 0.0, 1.0))
        # channel 10001: an actor without a root; 10002: without movement; 10003: channel with no actor
        bare = self.uobject("Projectile_1", "Projectile_Wushu_4_Smoke_C")
        channel = self.uobject()
        im.u64(channel + 0x78, bare)
        im.u64(channels + 8 * 10001, channel)
        stuck = self.uobject("Projectile_2", "Projectile_Wushu_4_Smoke_C")
        root = self.uobject()
        im.u64(stuck + 0x290, root)
        im.f64s(root + 0x320, (1.0, 2.0, 3.0))
        channel = self.uobject()
        im.u64(channel + 0x78, stuck)
        im.u64(channels + 8 * 10002, channel)
        im.u64(channels + 8 * 10003, self.uobject())

    def build_tags(self):
        """GameplayTagsManager node map + CommonlyReplicatedTags (GTN:42-106)."""
        im, a, names = self.image, self.a, self.names
        settings = a["tag_settings"] = self.uobject("Default__GameplayTagsSettings", "GameplayTagsSettings")
        im.i32(settings + 0xBC, 16)
        cdo = self.uobject("Default__GameplayTagsManager", "GameplayTagsManager")
        manager = a["tag_manager"] = self.uobject("GameplayTagsManager_0", "GameplayTagsManager")
        broken = self.uobject()
        im.u32(broken + 0x18, names.empty())
        no_class = self.uobject("NoClass")
        im.u64(CHUNK0 + 24 * 0xEA41, UNMAPPED)
        im.u64(CHUNK0 + 24 * 0xEA48, broken)
        im.u64(CHUNK0 + 24 * 0xEA49, no_class)
        im.u64(CHUNK0 + 24 * 0xEA50, settings)
        im.u64(CHUNK0 + 24 * 0xF000, cdo)
        im.u64(CHUNK0 + 24 * 0xF100, manager)
        a["tag_slots"] = {"settings": 0xEA50, "cdo": 0xF000, "manager": 0xF100}
        tags = ["AbilityTuning.Wushu.Q.MaxCharges", "AbilityTuning.Wushu.4.SmokeDuration",
                "AbilityTuning.Wushu.E.PrimeDuration", "AbilityTuning.Wushu.E.MaxCharges",
                "AbilityTuning.Wushu.4.SmokeMaxCharges", "AbilityTuning.Wushu.E.DashVelocityMultiplier",
                "aBility.Zeta", "Ability", "Ability.Cooldown"]
        data = a["tag_data"] = self.alloc(0x28 * 16)
        nodes = a["tag_nodes"] = []
        for i, tag in enumerate(tags):
            k = i if i < 4 else i + 1  # slot 4 stays empty
            node = self.uobject()
            im.u16(node + 0x50, 0xFFFF)
            nodes.append(node)
            im.u32(data + 0x28 * k, names.add(tag))
            im.u32(data + 0x28 * k + 4, 0)
            im.u64(data + 0x28 * k + 0x10, node)
        im.u64(manager + 0x130, data)
        im.u32(manager + 0x138, len(tags))
        im.u32(manager + 0x13C, len(tags) + 1)
        common = a["tag_common"] = self.alloc(0x100)
        im.u32(common, names.add("Ability.Cooldown"))
        im.u64(manager + 0x180, common)
        im.u32(manager + 0x188, 1)


# ---------------------------------------------------------------- responder snippets


def responder_source():
    import os
    path = Path(os.environ["ARIES_PROJECT_ROOT"]) / "local_backend" / "range_udp_handshake.py"
    return path.read_text(encoding="utf-8")


def find_statement(tree, predicate):
    """(parent body, index) of the first statement matching predicate."""
    for node in ast.walk(tree):
        for field in ("body", "orelse", "finalbody", "handlers"):
            body = getattr(node, field, None)
            if not isinstance(body, list):
                continue
            for index, statement in enumerate(body):
                if isinstance(statement, ast.stmt) and predicate(statement):
                    return body, index
    raise LookupError("responder statement not found")


def assigns(statement, name):
    if not isinstance(statement, ast.Try) or not statement.body:
        return False
    first = statement.body[0]
    return (isinstance(first, ast.Assign) and isinstance(first.targets[0], ast.Name)
            and first.targets[0].id == name)


class Responder:
    """the inline oracle code of run_probe, compiled from its own source."""

    def __init__(self):
        source = responder_source()
        tree = ast.parse(source)
        node = next(n for n in ast.walk(tree) if isinstance(n, ast.FunctionDef) and n.name == "read_firing_seed")
        self.firing_source = textwrap.dedent(ast.get_source_segment(source, node, padded=True))
        body, index = find_statement(tree, lambda s: assigns(s, "hr_handle"))
        self.hud = compile(ast.Module(body=body[index - 2:index + 1], type_ignores=[]), "hud_refresh", "exec")
        body, index = find_statement(tree, lambda s: assigns(s, "ult_pawn"))
        self.ult = compile(ast.Module(body=[body[index - 1], body[index]], type_ignores=[]), "ultimate", "exec")

    def firing_seed_fn(self):
        namespace = {"firing_rng_components": {}}
        exec(self.firing_source, namespace)
        return namespace["read_firing_seed"]

    def hud_refresh(self, reader):
        import read_world_state as rws
        namespace = {"state_reader": reader, "event": {}, "rws_read_bytes": rws.read_bytes, "struct": struct}
        exec(self.hud, namespace)
        return {"collapsed": namespace["hr_collapsed"], "cinematic": namespace["hr_cinematic"],
                "error": "hud_refresh_error" in namespace["event"]}

    def ultimate(self, reader):
        import read_world_state as rws
        namespace = {"state_reader": reader, "event": {}, "rws_read_bytes": rws.read_bytes, "struct": struct}
        exec(self.ult, namespace)
        return {"ult_required": namespace["ult_required"], "error": "ultimate_points_error" in namespace["event"]}


# ---------------------------------------------------------------- scenario runner


def plain(value):
    if isinstance(value, tuple):
        return [plain(v) for v in value]
    if isinstance(value, list):
        return [plain(v) for v in value]
    return value


class Run:
    def __init__(self, name, **client_args):
        self.steps = []
        self.client = Client(self.steps, **client_args)
        self.a = self.client.a
        self.image = self.client.image
        self.reader = None
        self.responder = RESPONDER
        self.firing_seed = self.responder.firing_seed_fn()
        self.name = name

    def call(self, fn, args, thunk):
        with FakeKernel(self.image):
            try:
                result = {"ok": thunk()}
            except Exception as error:  # every python error type is a failed read
                result = {"error": type(error).__name__}
        self.steps.append({"call": fn, "args": args, **result})
        return result

    def with_reader(self, fn, args, thunk):
        """H:1591-1598: the reader is created lazily and retried until it exists."""
        def run():
            from read_equippable_state import EquippableStateReader
            if self.reader is None:
                self.reader = EquippableStateReader(1, IMAGE_BASE)
            return thunk(self.reader)
        return self.call(fn, args, run)

    # every production read, by fixture name
    def derive_name_key(self):
        from read_fname_pool import derive_name_key
        return self.call("derive_name_key", {}, lambda: derive_name_key(0x4242, IMAGE_BASE).hex())

    def decode_name(self, index):
        from read_fname_pool import decode_name
        return self.call("decode_name", {"index": index}, lambda: decode_name(0x4242, IMAGE_BASE, index, NAME_KEY))

    def chain(self, fn):
        import read_equippable_state as res
        return self.call(fn, {}, lambda: getattr(res, fn)(0x4242, IMAGE_BASE))

    def camera(self):
        from read_equippable_state import read_camera
        return self.call("read_camera", {}, lambda: plain(read_camera(0x4242, IMAGE_BASE)))

    def slot_machines(self, slots):
        from read_equippable_state import read_slot_machines

        def run(reader):
            machines = read_slot_machines(reader, slots)
            return [{"slot": m.slot, "item": m.item, "class_name": m.class_name, "state_machine": m.state_machine,
                     "predicted": m.predicted, "authoritative": m.authoritative, "is_current": m.is_current}
                    for m in machines.values()]
        return self.with_reader("read_slot_machines", {"slots": slots}, run)

    def cached_states(self, sm):
        return self.with_reader("cached_state_names", {"state_machine": sm},
                                lambda reader: None if sm not in reader._names
                                else [[n, p] for n, p in reader._names[sm].items()])

    def uncached_states(self, sm):
        from read_equippable_state import state_names
        return self.with_reader("state_names", {"state_machine": sm}, lambda reader: [
            [n, p] for n, p in state_names(reader.handle, IMAGE_BASE, sm, reader.key).items()])

    def item_machines(self, item):
        from read_equippable_state import read_item_machines

        def run(reader):
            return [{"name": m.name, "address": m.address, "predicted": m.predicted,
                     "authoritative": m.authoritative, "starting": m.starting,
                     "state_names": list(m.state_names)} for m in read_item_machines(reader, item)]
        return self.with_reader("read_item_machines", {"item": item}, run)

    def resources(self, item, resources):
        from range_equippables import ResourceSubobject
        from read_equippable_state import read_resource_amounts
        subobjects = [ResourceSubobject(name, 0, offset, default_offset=default) for name, offset, default in resources]

        def run(reader):
            amounts = read_resource_amounts(reader, item, subobjects)
            return {name: [v.predicted, v.authoritative, v.default_amount] for name, v in amounts.items()}
        return self.with_reader("read_resource_amounts", {"item": item, "resources": resources}, run)

    def firing(self, sm, key):
        equippable = types.SimpleNamespace(key=key)
        machine = types.SimpleNamespace(state_machine=sm)
        return self.with_reader("read_firing_seed", {"state_machine": sm, "key": key},
                                lambda reader: self.firing_seed(("127.0.0.1", 1), equippable, machine, reader))

    def input_replication(self):
        from read_equippable_state import read_input_replication

        def run(reader):
            r = read_input_replication(reader)
            return None if r is None else {
                "stopped": r.stopped, "requested_correction": r.requested_correction,
                "inventory_correction": r.inventory_correction,
                "current_equippable_class": r.current_equippable_class}
        return self.with_reader("read_input_replication", {}, run)

    def input_bounds(self):
        from capture_input_event_cursor import read_pending_input_bounds

        def run():
            b = read_pending_input_bounds(1, IMAGE_BASE)
            return None if b is None else {"component": b["component"], "stopped": b["stopped"],
                                           "cursor": b["cursor"], "count": b["count"],
                                           "latest": b["latest_generated_event_id"]}
        return self.call("read_pending_input_bounds", {}, run)

    def saved_moves(self):
        from read_move_number import read_saved_moves

        def run():
            m = read_saved_moves(1, IMAGE_BASE)
            return None if m is None else {key: m[key] for key in (
                "count", "oldest", "newest", "last_acked", "numbers", "newest_timestamp_bits", "newest_timestamp",
                "move_handler_gate_open", "dropping_moves")}
        return self.call("read_saved_moves", {}, run)

    def resolve(self, thunk, kind):
        from resolve_rpc_index import resolve
        return self.call("resolve_rpc_index", {"thunk": thunk, "kind": kind},
                         lambda: resolve(1, IMAGE_BASE, thunk, kind))

    def projectile(self, channel):
        from range_projectiles import read_projectile

        def run(reader):
            s = read_projectile(reader, channel)
            return None if s is None else {"class_name": s.class_name, "location": plain(s.location),
                                           "collisions": s.collisions, "hit_location": plain(s.hit_location),
                                           "hit_normal": plain(s.hit_normal)}
        return self.with_reader("read_projectile", {"channel": channel}, run)

    def hud(self):
        return self.with_reader("hud_refresh", {}, self.responder.hud_refresh)

    def ultimate(self):
        return self.with_reader("ultimate_points", {}, self.responder.ultimate)

    def tags(self):
        """H:4054-4059: its own temporary reader, then predict_net_indices."""
        from gameplay_tag_net_index import predict_net_indices
        from read_equippable_state import EquippableStateReader

        def run():
            tag_reader = EquippableStateReader(1, IMAGE_BASE)
            try:
                t = predict_net_indices(tag_reader.handle, IMAGE_BASE, tag_reader.key)
            finally:
                tag_reader.close()
            return {"indices": t.indices, "true_bit_num": t.true_bit_num,
                    "first_bit_segment": t.first_bit_segment, "constructed": t.constructed}
        return self.call("predict_net_indices", {}, run)

    def export(self):
        return {"name": self.name, "image_base": IMAGE_BASE, "steps": self.steps}


PISTOL_RESOURCES = [["MagazineAmmo", 0x1208, 0x12C], ["ReserveAmmo", 0x1200, 0x12C]]  # RE:326-327
SMOKE_RESOURCES = [["EquipmentCharge", 0x1240, 0x188]]                             # RE:272


def everything(run):
    a = run.a
    run.chain("find_world")
    run.chain("find_controller")
    run.chain("find_pawn")
    run.camera()
    run.slot_machines([1, 2, 3, 4, 5, 9])
    for sm in ("pistol_sm", "knife_sm", "smoke_sm"):
        run.cached_states(a[sm])
    run.item_machines(a["pistol"])
    run.resources(a["pistol"], PISTOL_RESOURCES)
    run.resources(a["smoke"], SMOKE_RESOURCES)
    run.firing(a["pistol_sm"], "BasePistol")
    run.input_replication()
    run.input_bounds()
    run.saved_moves()
    run.resolve(HUD_THUNK, "controller")
    run.resolve(IMAGE_BASE + 0x1234560, "pawn")
    run.projectile(10000)
    run.hud()
    run.ultimate()
    run.tags()


def scenario_healthy():
    run = Run("healthy")
    a, im = run.a, run.image
    run.derive_name_key()
    names = run.client.names
    for text in ("AresGameEngine", "FiringState", "Projectile_Wushu_4_Smoke_C"):
        run.decode_name(names.add(text))
    run.decode_name(names.add("Wüshu™", wide=True))
    run.decode_name(names.raw(300, False, b"x" * 300))
    run.decode_name(names.raw(4, False, b"\xff\xfeAB"))
    run.decode_name(names.raw(2, True, "\ud800".encode("utf-16-le", "surrogatepass") + b"A\x00"))
    run.decode_name(names.empty())
    run.decode_name(0x10000000)
    run.decode_name(0x10005)
    everything(run)
    run.resolve(IMAGE_BASE + 0x6244D00, "controller")
    run.resolve(IMAGE_BASE + 0x999, "controller")
    for channel in (10001, 10002, 10003, 10009, 10010, 20000):
        run.projectile(channel)
    # the client has built its tag index (node +0x50 = NetIndex)
    for i, node in enumerate(a["tag_nodes"]):
        im.u16(node + 0x50, 100 + i)
    run.tags()
    return run


def scenario_state_caches():
    run = Run("state_caches")
    a, im = run.a, run.image
    run.slot_machines([3, 1])
    run.cached_states(a["smoke_sm"])
    # the TMap fills in after the first read; the empty result stays cached (Q4)
    states = [run.client.uobject(name) for name in ("InactiveState", "EquipState", "IdleState")]
    for i, state in enumerate(states):
        im.u64(a["smoke_data"] + 24 * i, state)
    im.u32(a["smoke_sm"] + 0x168, 3)
    im.u64(a["smoke_sm"] + 0x1B0, states[1])
    run.slot_machines([3])
    run.cached_states(a["smoke_sm"])
    run.uncached_states(a["smoke_sm"])
    # an item's class changes under the same address: the cached name wins (Q5)
    im.u64(a["pistol"] + 0x10, run.client.uclass("Vandal_C"))
    run.slot_machines([1])
    run.input_replication()
    # the rng component is cached per (machine, key); its seed is re-read
    run.firing(a["pistol_sm"], "BasePistol")
    other_rng = run.client.uobject()
    im.u32(other_rng + 0xEC, 0x1234)
    im.u64(a["pistol_states"]["FiringState"][0] + 0x4A0, other_rng)
    run.firing(a["pistol_sm"], "BasePistol")
    im.u32(a["rng"] + 0xEC, 0xC0FFEE12)
    run.firing(a["pistol_sm"], "BasePistol")
    run.firing(a["pistol_sm"], "OtherKey")
    # unknown machine (no cached states) and a machine with no firing state
    run.firing(a["zoom_sm"], "BasePistol")
    run.item_machines(a["pistol"])
    run.firing(a["zoom_sm"], "BasePistol")
    run.firing(0x123450000, "BasePistol")
    # seed read fails -> None
    im.u64(a["pistol_states"]["FiringState"][0] + 0x4A0, UNMAPPED)
    run.firing(a["pistol_sm"], "Fresh")
    return run


def scenario_firing_names():
    run = Run("firing_names")
    a, im, c = run.a, run.image, run.client
    # duplicate names (first position, last pointer), a null hole, "Firing" prefix fallback
    sm, data, states = c.state_machine(
        "EquippableStateMachine",
        ["InactiveState", "FiringAuto", None, "EquipState", "InactiveState", "FiringBurst"],
        "InactiveState", "FiringAuto", "InactiveState")
    rng = c.uobject()
    im.u32(rng + 0xEC, 77)
    im.u64(states["FiringAuto"][0] + 0x4A0, rng)
    gun = c.uobject("Gun_0", "Phantom_C", size=0x2000)
    im.u64(gun + 0xE00, sm)
    machines = c.alloc(0x40)
    im.u64(machines, sm)
    im.u64(gun + 0xE30, machines)
    im.u32(gun + 0xE38, 1)
    item_slot = c.uobject()
    im.u64(item_slot + 0x38, gun)
    im.u64(a["inventory"] + 0x1B8 + 8 * 6, item_slot)
    run.slot_machines([6, 1])
    run.cached_states(sm)
    run.uncached_states(sm)
    run.item_machines(gun)
    run.firing(sm, "Phantom")
    # rng pointer 0 is not cached
    sm2, _, states2 = c.state_machine("EquippableStateMachine", ["FiringState"], "FiringState", None, None)
    gun2 = c.uobject("Gun_1", "Odin_C", size=0x2000)
    im.u64(gun2 + 0xE00, sm2)
    im.u64(item_slot + 0x38, gun2)
    run.slot_machines([6])
    run.firing(sm2, "Odin")
    rng2 = c.uobject()
    im.u32(rng2 + 0xEC, 5)
    im.u64(states2["FiringState"][0] + 0x4A0, rng2)
    run.firing(sm2, "Odin")
    # item machine array edges
    for count in (0, 9, 8):
        im.u32(gun + 0xE38, count)
        run.item_machines(gun)
    im.u64(gun + 0xE30, 0)
    run.item_machines(gun)
    # a machine that cannot be named fails the whole read
    bad = c.uobject()
    im.u32(bad + 0x18, c.names.empty())
    im.u64(machines + 8, bad)
    im.u64(gun + 0xE30, machines)
    im.u32(gun + 0xE38, 2)
    run.item_machines(gun)
    return run


def scenario_unpossessed():
    run = Run("unpossessed")
    run.image.u64(run.a["controller"] + 0x4B0, 0)
    everything(run)
    run.resolve(HUD_THUNK, "pawn")
    return run


def scenario_no_controller():
    run = Run("no_controller")
    run.image.u64(run.a["local_player"] + 0x38, 0)
    everything(run)
    return run


def scenario_no_world():
    run = Run("no_world")
    run.image.u64(run.a["viewport"] + 0x80, 0)
    everything(run)
    return run


def scenario_header_rules():
    """each python caller kept its own GObjects header check (spec 09 3.2)."""
    out = []
    for name, args in (("header_no_chunks", {"num_chunks": 0}),
                       ("header_one_chunk", {"num_chunks": 1}),
                       ("header_few_elements", {"num_elements": 0x800}),
                       ("header_engine_elements", {"num_elements": 0x107E2}),
                       ("header_many_chunks", {"num_chunks": 0x15}),
                       ("header_too_many_chunks", {"num_chunks": 0x101}),
                       ("header_huge_elements", {"num_elements": 0x400001})):
        run = Run(name, **args)
        run.derive_name_key()
        everything(run)
        out.append(run)
    return out


def scenario_name_key():
    """R3: derive_name_key's own checks (FNP:53-64). the reader is retried by every
    call until one creation succeeds (H:1591-1598); the tag read derives its own."""
    run = Run("name_key")
    a, im, names = run.a, run.image, run.client.names
    engine, anchor = a["engine"], names.add("AresGameEngine")
    im.u32(engine + 0xC, 0x107E3)  # the engine's object index moved
    run.derive_name_key()
    run.slot_machines([1])
    run.tags()
    im.u32(engine + 0xC, 0x107E2)
    for bad in (names.add("AresGameEngine", wide=True),  # wide anchor
                names.add("AresGameEngin"),              # 13 chars
                names.add("AresGameEngins"),             # key from "Ares" fine, full name differs
                names.add("XresGameEngine"),             # wrong key, full name differs
                0x10005,                                 # name block 1 is null
                names.empty()):                          # length 0
        im.u32(engine + 0x18, bad)
        run.derive_name_key()
        run.slot_machines([1])
    run.tags()
    im.u32(engine + 0x18, anchor)
    run.derive_name_key()
    run.slot_machines([1])
    run.tags()
    # the open reader keeps its key; the tag read's own derivation fails
    im.u32(engine + 0xC, 0x107E3)
    run.tags()
    run.slot_machines([1])
    return run


def scenario_chunk_table_edges():
    """derive_name_key reads only the engine's chunk pointer (FNP:50-51); every other
    walk reads the whole table (RES:37, RRI:66, CIE:30, RMN:59, GTN:46)."""
    out = []
    # entry 0 on an unmapped page, entry 1 (the engine's chunk) on the next one
    run = Run("chunk_table_first_unreadable")
    im = run.image
    table = 0x1A0004FF8
    im.map(table + 8, 8)
    im.u64(table + 8, CHUNK1)
    im.u64(ARRAY, table)
    run.derive_name_key()
    everything(run)
    out.append(run)
    # three chunks, the third pointer on an unmapped page
    run = Run("chunk_table_last_unreadable")
    im = run.image
    table = 0x1A0006FF0
    im.map(table, 16)
    im.u64(table, CHUNK0)
    im.u64(table + 8, CHUNK1)
    im.u64(ARRAY, table)
    im.u32(ARRAY + 0x1C, 3)
    run.derive_name_key()
    everything(run)
    out.append(run)
    return out


def scenario_moves():
    run = Run("moves")
    a, im = run.a, run.image
    pd, array, moves = a["prediction"], a["saved_array"], a["moves"]
    run.saved_moves()
    im.u64(pd + 0x50, 0)
    run.saved_moves()
    im.u32(pd + 0x28, 0)
    run.saved_moves()
    im.u32(pd + 0x28, 4097)
    run.saved_moves()
    im.u32(pd + 0x28, 4096)
    run.saved_moves()  # 256 slots read, 3 non-null
    # null in the middle and in the last slot: no timestamp
    im.u32(pd + 0x28, 4)
    im.u64(array + 16, 0)
    run.saved_moves()
    # 300 saved moves: only the first 256 are read
    for i in range(300):
        move = run.client.alloc(0x1000) if moves[i] is None else moves[i]
        moves[i] = move
        im.u64(move, a["move_vtable"])
        im.u32(move + 0x1C, 1000 + i)
        im.write(move + 0x3C, struct.pack("<f", 0.25 * i))
        if i < 256:
            im.u64(array + 16 * i, move)
    im.u32(pd + 0x28, 300)
    run.saved_moves()
    # an unreadable saved move fails the read
    im.u64(array + 16 * 7, UNMAPPED)
    run.saved_moves()
    im.u64(array + 16 * 7, moves[7])
    # the diagnostic reads can fail the call too (RMN:69-79)
    im.u64(a["movement"], UNMAPPED)
    run.saved_moves()
    im.u64(a["movement"], run.client.alloc(0x1000))
    run.saved_moves()
    im.u64(pd + 0x20, 0)
    run.saved_moves()
    im.u64(a["movement"] + 0x7E8, 0)
    run.saved_moves()
    return run


def scenario_rpc():
    run = Run("rpc")
    a, im = run.a, run.image
    child, parent, entries = a["child_cache"], a["parent_cache"], a["cache_set"]
    run.resolve(HUD_THUNK, "controller")
    # a match in the child and the parent: the parent (later) wins
    im.u64(a["child_fields"] + 24 * 5, a["hud_fn"] | 1)
    im.u32(a["child_fields"] + 24 * 5 + 8, 155)
    run.resolve(HUD_THUNK, "controller")
    # cycle: the depth bound stops the walk
    im.u64(parent + 8, child)
    run.resolve(HUD_THUNK, "controller")
    im.u64(parent + 8, 0)
    # a tagged entry whose function is unreadable fails the call
    im.u64(a["parent_fields"] + 24 * 70, UNMAPPED | 1)
    run.resolve(HUD_THUNK, "controller")
    im.u64(a["parent_fields"] + 24 * 70, 0)
    # parent count out of range: skipped, chain continues
    im.u32(parent + 0x28, 5000)
    run.resolve(HUD_THUNK, "controller")
    im.u32(parent + 0x28, 150)
    # unreadable entries array: readable() raises
    im.u64(child + 0x20, UNMAPPED)
    run.resolve(HUD_THUNK, "controller")
    im.u64(child + 0x20, 0)
    run.resolve(HUD_THUNK, "controller")
    # no set entry for the class
    im.u64(entries + 24, 0x0000000700000000 | 0x3EB22)
    run.resolve(HUD_THUNK, "controller")
    # matching key with a null cache pointer
    im.u64(entries + 24, 0x0000000700000000 | 0x3EB21)
    im.u64(entries + 32, 0)
    run.resolve(HUD_THUNK, "controller")
    im.u64(entries + 32, child)
    for count in (0, 0x10001):
        im.u32(a["cache_mgr"] + 0x10, count)
        run.resolve(HUD_THUNK, "controller")
    im.u32(a["cache_mgr"] + 0x10, 3)
    im.u64(a["cache_mgr"] + 8, 0)
    run.resolve(HUD_THUNK, "controller")
    im.u64(a["cache_mgr"] + 8, entries)
    # class index read failure (actor class unreadable)
    im.u64(a["controller"] + 0x10, UNMAPPED)
    run.resolve(HUD_THUNK, "controller")
    return run


def scenario_input():
    run = Run("input")
    a, im = run.a, run.image
    component, inventory = a["input"], a["inventory"]
    im.u32(component + 0xA84, 0xFFFFFFF0)
    im.u32(component + 0x1198, 0x20)
    run.input_bounds()
    im.u32(component + 0x1198, 4097)
    run.input_bounds()
    im.u32(component + 0x1198, 4096)
    im.u8(component + 0x12C, 0)
    run.input_bounds()
    run.input_replication()
    # desired (+0x260) wins over current (+0x278)
    im.u64(inventory + 0x260, a["knife"])
    run.input_replication()
    im.u64(inventory + 0x260, 0)
    im.u64(inventory + 0x278, 0)
    run.input_replication()
    # an equippable whose class name cannot be decoded
    im.u64(inventory + 0x278, a["camera_manager"])
    run.input_replication()
    im.u64(inventory + 0x278, a["pistol"])
    im.u64(a["pawn"] + 0xBF8, 0)
    run.input_replication()
    run.slot_machines([1])
    im.u64(a["pawn"] + 0xBF8, inventory)
    im.u64(a["pawn"] + 0xB90, 0)
    run.input_replication()
    run.input_bounds()
    return run


def scenario_projectile():
    run = Run("projectile")
    a, im = run.a, run.image
    run.projectile(10000)
    im.u32(a["projectile_movement"] + 0x198, 0)
    run.projectile(10000)
    im.u64(a["projectile"] + 0x610, UNMAPPED)
    run.projectile(10000)
    im.u64(a["connection"] + 0x13F8, 0)
    run.projectile(10000)
    im.u64(a["net_driver"] + 0xC0, 0)
    run.projectile(10000)
    im.u64(a["world"] + 0x40, 0)
    run.projectile(10000)
    run.resolve(HUD_THUNK, "controller")
    return run


def scenario_resources():
    run = Run("resources")
    a, im = run.a, run.image
    run.resources(a["pistol"], [["MagazineAmmo", 0x1208, 0x12C], ["ReserveAmmo", 0x1208, 0x12C],
                                ["EquipmentCharge", 0x1240, 0x188], ["MagazineAmmo", 0x1200, 0x12C]])
    im.write(a["magazine"] + 0x10C, struct.pack("<ii", -1, -2147483648))
    im.i32(a["magazine"] + 0x188, -7)
    run.resources(a["pistol"], [["MagazineAmmo", 0x1208, 0x188]])
    im.u64(a["pistol"] + 0x1208, UNMAPPED)
    run.resources(a["pistol"], PISTOL_RESOURCES)
    run.resources(a["pistol"], [["ReserveAmmo", 0x1200, 0x12C]])
    return run


def scenario_hud():
    run = Run("hud")
    a, im = run.a, run.image
    run.hud()
    im.u32(a["controller"] + 0x690, 0x3)
    run.hud()
    im.u32(a["controller"] + 0x690, 0x0)
    im.u8(a["widget"] + 0x11C, 2)
    run.hud()
    im.u64(a["pawn"] + 0xB80, 0)
    run.hud()
    im.u64(a["pawn"] + 0xB80, UNMAPPED)
    run.hud()
    im.u64(a["pawn"] + 0xB80, a["widget"])
    im.u8(a["widget"] + 0x11C, 1)
    run.hud()
    im.i32(a["pawn"] + 0x14F4, 50)
    run.ultimate()
    im.i32(a["pawn"] + 0x14F4, -3)
    run.ultimate()
    im.u64(a["camera_manager"] + 0x17C0, 0)
    im.u64(a["controller"] + 0x528, 0)
    run.camera()
    return run


def scenario_tags():
    run = Run("tags")
    a, im, c = run.a, run.image, run.client
    manager, data, nodes = a["tag_manager"], a["tag_data"], a["tag_nodes"]
    run.tags()
    # NetIndexFirstBitSegment below the true bit count
    im.i32(a["tag_settings"] + 0xBC, 3)
    run.tags()
    im.i32(a["tag_settings"] + 0xBC, 16)
    # duplicate names sort by number; the last one wins the index
    dup = c.uobject()
    im.u16(dup + 0x50, 0xFFFF)
    im.u32(data + 0x28 * 4, c.names.add("Ability"))
    im.u32(data + 0x28 * 4 + 4, 3)
    im.u64(data + 0x28 * 4 + 0x10, dup)
    im.u32(manager + 0x138, 10)
    run.tags()
    # non-ascii tag (python str.lower) and a common tag that is missing
    im.u32(data + 0x28 * 4, c.names.add("Ärger.Tag"))
    im.u32(data + 0x28 * 4 + 4, 0)
    im.u32(a["tag_common"] + 12, c.names.add("Nope.Tag"))
    im.u32(a["tag_common"] + 24, c.names.add("Ability"))
    im.u32(manager + 0x188, 3)
    run.tags()
    # a common tag past the end of the list (IndexError)
    im.u32(manager + 0x188, 11)
    for i in range(3, 11):
        im.u32(a["tag_common"] + 12 * i, c.names.add("Ability"))
    run.tags()
    im.u32(manager + 0x188, 1)
    # an undecodable node name is skipped -> count mismatch
    im.u32(data + 0x28 * 4, c.names.empty())
    run.tags()
    im.u32(manager + 0x138, 9)
    run.tags()
    # common tag that cannot be decoded propagates
    im.u32(a["tag_common"], c.names.empty())
    run.tags()
    im.u32(a["tag_common"], c.names.add("Ability.Cooldown"))
    # constructed index on the first sorted node, partial on others
    im.u16(nodes[7] + 0x50, 4)
    run.tags()
    # node map unreadable
    im.u64(manager + 0x130, UNMAPPED)
    run.tags()
    im.u64(manager + 0x130, data)
    # empty node map with count 0: entries[0] raises
    im.u32(manager + 0x138, 0)
    im.u32(manager + 0x13C, 0)
    run.tags()
    im.u32(manager + 0x13C, 10)
    im.u32(manager + 0x138, 9)
    # manager missing from the scan window
    im.u64(CHUNK0 + 24 * a["tag_slots"]["manager"], 0)
    run.tags()
    # an unreadable object slot outside the try propagates
    im.u64(CHUNK0 + 24 * a["tag_slots"]["manager"], manager)
    im.u64(ARRAY, CHUNK_TABLE)
    im.u64(CHUNK_TABLE, UNMAPPED)
    run.tags()
    return run


def scenario_tag_sort_keys():
    """the sort key is python 3.12's str.lower (GTN:87): characters newer than its
    unicode 15.0 tables keep their case, so these pairs sort by text, not number."""
    run = Run("tag_sort_keys")
    a, im, c = run.a, run.image, run.client
    manager, data = a["tag_manager"], a["tag_data"]
    extra = [(4, "Ꟍ.Tag", 1, False),          # newer capital: python keeps it
             (10, "ꟍ.Tag", 0, False),         # its newer small form
             (11, "\U00010d50x", 1, True),         # supplementary plane, wide name
             (12, "\U00010d70x", 0, True),
             (13, "ΟΔΟΣ", 0, False),  # final sigma
             (14, "AʕΣ.Tag", 0, False)]  # 0295 is cased only in python's tables
    for k, text, number, wide in extra:
        node = c.uobject()
        im.u16(node + 0x50, 0xFFFF)
        im.u32(data + 0x28 * k, c.names.add(text, wide=wide))
        im.u32(data + 0x28 * k + 4, number)
        im.u64(data + 0x28 * k + 0x10, node)
    im.u32(manager + 0x138, 9 + len(extra))
    im.u32(manager + 0x13C, 15)
    run.tags()
    # junk capacity: python fails on the first unreadable piece of the block
    im.u32(manager + 0x13C, 0xFFFFFFFF)
    run.tags()
    im.u64(manager + 0x130, UNMAPPED)
    run.tags()
    return run


def scenario_garbage_pointers():
    """pointers near 2**64: python ints do not wrap, so base + offset fails the bounds
    check; the rust reads must fail the same way (and never overflow)."""
    run = Run("garbage_pointers")
    a, im = run.a, run.image
    huge = 0xFFFFFFFFFFFFFFF0
    run.slot_machines([1])
    im.u64(a["pistol"] + 0xE00, huge)
    run.item_machines(a["pistol"])
    run.resources(a["pistol"], PISTOL_RESOURCES)
    im.u64(a["pistol"] + 0x1208, huge)
    run.resources(a["pistol"], PISTOL_RESOURCES)
    im.u64(a["pistol_machines"], huge)
    run.item_machines(a["pistol"])
    im.u64(a["inventory"] + 0x1B8 + 8 * 2, huge)
    run.slot_machines([2])
    im.u64(a["inventory"] + 0x260, huge)
    run.input_replication()
    im.u64(a["connection"] + 0x13F8, huge)
    run.projectile(10000)
    im.u64(a["prediction"] + 0x20, huge)
    run.saved_moves()
    im.u64(a["child_cache"] + 8, huge)
    run.resolve(HUD_THUNK, "controller")
    im.u64(a["tag_data"] + 0x28 * 7 + 0x10, huge)
    run.tags()
    im.u64(a["pawn"] + 0xB80, huge)
    run.hud()
    im.u64(a["controller"] + 0x528, huge)
    run.camera()
    im.u64(a["controller"] + 0x4B0, huge)
    run.ultimate()
    run.input_bounds()
    run.chain("find_pawn")
    im.u64(a["local_player"] + 0x38, huge)
    run.hud()
    run.chain("find_pawn")
    im.u64(a["game_instance"] + 0x40, huge)
    run.chain("find_controller")
    im.u64(a["viewport"] + 0x80, huge)
    run.chain("find_world")
    run.projectile(10000)
    return run


def section_oracle_reads():
    import os
    sys.path.insert(0, str(Path(os.environ["ARIES_PROJECT_ROOT"]) / "range_authority_probe"))
    global RESPONDER
    RESPONDER = Responder()
    runs = [scenario_healthy(), scenario_state_caches(), scenario_firing_names(), scenario_unpossessed(),
            scenario_no_controller(), scenario_no_world(), *scenario_header_rules(), scenario_name_key(),
            *scenario_chunk_table_edges(),
            scenario_moves(), scenario_rpc(), scenario_input(), scenario_projectile(), scenario_resources(),
            scenario_hud(), scenario_tags(), scenario_tag_sort_keys(), scenario_garbage_pointers()]
    return [run.export() for run in runs]


SIGMA = "Σ"
LOWER_SAMPLES = [
    "A" + SIGMA, SIGMA, "A" + SIGMA + "B", "A." + SIGMA, "A'" + SIGMA, "ABCDEFGHIJKLMNOPQRSTUVWXYZ" + SIGMA,
    "ΟΔΟ" + SIGMA, "ΟΔΟ" + SIGMA + ".Tag", "A" + SIGMA * 3, SIGMA * 3,
    "Ꟍ" + SIGMA, "AꟌ" + SIGMA, "ʕ" + SIGMA, "Aʕ" + SIGMA, "A\U0001171e" + SIGMA,
    "\U0001171e" + SIGMA, SIGMA + "Ꟍ", "A" + SIGMA + "ʕ", "A" + SIGMA + "\U0001171eB",
    "A" + SIGMA + "\U0001171eʕ", "İ", "ẞ", "Ꟍꟍ.Tag", "\U00010d50x\U00010d70",
    "AbilityTuning.Wushu.4.SmokeDuration", "aBility.Zeta",
]


def section_oracle_lower():
    """python 3.12 str.lower, the tag sort key (GTN:87): every code point it changes,
    every code point's final-sigma class as runs (2: final after "A" + c, i.e. c is
    case-ignorable or cased; 1: final after c alone, i.e. cased and not ignorable),
    and whole strings where sigma depends on its neighbours."""
    changed, runs = [], []
    for cp in range(0x110000):
        if 0xD800 <= cp <= 0xDFFF:
            continue
        c = chr(cp)
        if c.lower() != c:
            changed.append([cp, c.lower()])
        kind = (2 * (("A" + c + SIGMA).lower()[-1] == "ς")
                + ((c + SIGMA).lower()[-1] == "ς"))
        if runs and runs[-1][1] + 1 == cp and runs[-1][2] == kind:
            runs[-1][1] = cp
        else:
            runs.append([cp, cp, kind])
    return {"python": sys.version.split()[0], "changed": changed, "sigma_runs": runs,
            "samples": [[s, s.lower()] for s in LOWER_SAMPLES]}


RESPONDER = None
SECTIONS = {"oracle_reads": section_oracle_reads, "oracle_lower": section_oracle_lower}
