"""oracle R1: check_live_packet_keys.read_current_handler_keys against synthetic
address spaces (no live process). its module-level kernel32 is swapped for a fake
whose VirtualQueryEx reports scripted regions and whose ReadProcessMemory fails
on any unreadable page, like a real one. keys here are synthetic bytes."""

import ctypes
import types

PAGE = 0x1000
IMAGE_BASE = 0x7FF6A0000000
NEEDLE = IMAGE_BASE + 0xA5F3BB0  # H:1472-1473
SCAN_END = 0x7FFFFFFF0000
MEM_COMMIT, MEM_RESERVE, MEM_FREE = 0x1000, 0x2000, 0x10000
MEM_PRIVATE, MEM_MAPPED, MEM_IMAGE = 0x20000, 0x40000, 0x1000000
GIB = 1 << 30


class Space:
    """sorted regions [base, size, state, kind, protect, readable] + byte writes."""

    def __init__(self):
        self.regions = []
        self.pages = {}

    def region(self, base, size, state=MEM_COMMIT, kind=MEM_PRIVATE, protect=0x04, readable=True):
        self.regions.append([base, size, state, kind, protect, readable])
        self.regions.sort()

    def write(self, address, data):
        for i, value in enumerate(bytes(data)):
            page = (address + i) & ~(PAGE - 1)
            self.pages.setdefault(page, bytearray(PAGE))[(address + i) - page] = value

    def handler(self, address, send_key, receive_key, needle=NEEDLE):
        self.write(address, needle.to_bytes(8, "little"))
        self.write(address + 0x28, send_key)
        self.write(address + 0x48, receive_key)

    def query(self, address):
        """like VirtualQueryEx: from the page holding address to the end of its run
        of equal pages (a scripted region, or the free gap around it)."""
        page = address & ~(PAGE - 1)
        for base, size, state, kind, protect, _ in self.regions:
            if base <= address < base + size:
                return page, base + size - page, state, kind, protect
            if base > address:
                return page, base - page, MEM_FREE, 0, 0x01
        return page, SCAN_END + 0x10000 - page, MEM_FREE, 0, 0x01

    def readable(self, address):
        for base, size, *_, readable in self.regions:
            if base <= address < base + size:
                return readable
        return False

    def read(self, address, size):
        out = bytearray(size)
        page = address & ~(PAGE - 1)
        while page < address + size:
            if not self.readable(page):
                return None
            data = self.pages.get(page)
            if data is not None:
                lo, hi = max(page, address), min(page + PAGE, address + size)
                out[lo - address:hi - address] = data[lo - page:hi - page]
            page += PAGE
        return bytes(out)

    def export(self):
        return {"regions": self.regions,
                "writes": [[page, bytes(data).hex()] for page, data in sorted(self.pages.items())]}


def fake_kernel(space):
    fake = types.SimpleNamespace()

    def open_process(access, inherit, pid):
        return 0x4242

    def close_handle(handle):
        return 1

    def virtual_query(handle, address, info, size):
        address = getattr(address, "value", address) or 0
        base, length, state, kind, protect = space.query(address)
        info = info._obj
        info.base_address = base or None
        info.region_size = length
        info.state = state
        info.memory_type = kind
        info.protect = protect
        return ctypes.sizeof(info)

    def read_process_memory(handle, address, buffer, size, count):
        address = getattr(address, "value", address) or 0
        data = space.read(address, int(size))
        if data is None:
            count._obj.value = 0
            return 0
        ctypes.memmove(buffer, data, len(data))
        count._obj.value = len(data)
        return 1

    fake.OpenProcess = open_process
    fake.CloseHandle = close_handle
    fake.VirtualQueryEx = virtual_query
    fake.ReadProcessMemory = read_process_memory
    return fake


def key(start):
    return bytes((start + i) & 0xFF for i in range(32))


def scenarios():
    out = []

    # spec 09 §10.3: a decoy whose 112-byte re-read runs into an unmapped page
    s = Space()
    s.region(0x300000000, PAGE)
    s.write(0x300000FF8, NEEDLE.to_bytes(8, "little"))
    s.region(0x300100000, 2 * PAGE)
    s.handler(0x300100040, key(0x10), key(0xA0))
    out.append(("spec_decoy", s))

    # protections and types the filter skips, then an execute-read-write hit
    s = Space()
    for i, (state, kind, protect) in enumerate(((MEM_COMMIT, MEM_PRIVATE, 0x104), (MEM_COMMIT, MEM_PRIVATE, 0x01),
                                                (MEM_COMMIT, MEM_IMAGE, 0x04), (MEM_RESERVE, MEM_PRIVATE, 0x04),
                                                (MEM_COMMIT, MEM_MAPPED, 0x04), (MEM_COMMIT, MEM_PRIVATE, 0x10))):
        base = 0x400000000 + i * 0x100000
        s.region(base, 0x10000, state, kind, protect)
        s.handler(base + 0x100, key(i), key(0x80 + i))
    s.region(0x500000000, 0x10000, protect=0x40)
    s.handler(0x500000200, key(0x30), key(0xC0))
    out.append(("filters", s))

    # a needle across a 1 MiB chunk boundary is missed; the next one is found
    s = Space()
    s.region(0x600000000, 0x300000)
    s.handler(0x600100000 - 4, key(0x40), key(0x50))
    s.handler(0x600200100, key(0x41), key(0x51))
    out.append(("chunk_straddle", s))

    # first valid hit in ascending order wins, even inside one chunk
    s = Space()
    s.region(0x700000000, 0x20000, protect=0x02)
    s.handler(0x700010008, key(0x60), key(0x70))
    s.handler(0x700000010, key(0x61), key(0x71))
    out.append(("first_wins", s))

    # failed reads do not count toward the 4096 MiB cap (CLK:77)
    s = Space()
    s.region(0x800000000, 5 * GIB, readable=False)
    s.region(0x980000000, 0x10000)
    s.handler(0x980000800, key(0x90), key(0xE0))
    out.append(("unreadable_region", s))

    # a handler-like object whose bytes stop short of 112 readable bytes, then none
    s = Space()
    s.region(0xA00000000, PAGE)
    s.write(0xA00000F90 + 0x30, NEEDLE.to_bytes(8, "little"))
    out.append(("not_found", s))

    # a different vtable value is not a hit; zero-length and odd regions are walked
    s = Space()
    s.region(0xB00000000, 0x10000)
    s.handler(0xB00000100, key(0x11), key(0x22), needle=NEEDLE + 8)
    s.region(0xB00010000, 0x10000)
    s.handler(0xB0001FF00, key(0x12), key(0x23))
    out.append(("wrong_vtable", s))
    return out


def section_oracle_keys():
    import check_live_packet_keys as clk
    saved = clk.kernel32
    cases = []
    # the time limit is compared as float seconds (CLK:52, 63): a negative or nan limit
    # scans nothing, an infinite one never times out. repr() so json keeps nan and inf.
    limits = {"spec_decoy": [30.0, -1.0, 0.0, float("nan"), float("inf")]}
    try:
        for name, space in scenarios():
            clk.kernel32 = fake_kernel(space)
            for max_seconds in limits.get(name, [30.0]):
                try:
                    handler, send_key, receive_key = clk.read_current_handler_keys(1, NEEDLE, max_seconds)
                    result = {"ok": {"handler": handler, "client_send": send_key.hex(), "server_send": receive_key.hex()}}
                except Exception as error:
                    result = {"error": type(error).__name__}
                cases.append({"name": name, "needle": NEEDLE, "max_seconds": repr(max_seconds), **space.export(), **result})
    finally:
        clk.kernel32 = saved
    return cases


SECTIONS = {"oracle_keys": section_oracle_keys}
