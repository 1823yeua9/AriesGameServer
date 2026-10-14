# 09 Oracle: live client memory reads

Spec for the Rust `oracle` crate. It covers every read of the live client process that the production
responder makes, the decoders those reads need (GObjects, FName), how often each read runs, what
happens when it fails, and whether the read is needed at all. Test vectors are in section 10.

Source abbreviations (paths under `C:\Users\Cayden\Desktop\valorant proj`):

| tag | file |
|---|---|
| H | `local_backend/range_udp_handshake.py` (`run_probe` from line 715) |
| RWS | `range_authority_probe/read_world_state.py` (imported as `rws`) |
| RRI | `local_backend/resolve_rpc_index.py` |
| FNP | `local_backend/read_fname_pool.py` |
| RES | `local_backend/read_equippable_state.py` |
| CIE | `local_backend/capture_input_event_cursor.py` |
| RMN | `local_backend/read_move_number.py` |
| CLK | `local_backend/check_live_packet_keys.py` |
| ILPN | `local_backend/inspect_live_packet_notify.py` |
| GTN | `local_backend/gameplay_tag_net_index.py` |
| RP | `local_backend/range_projectiles.py` |
| RE | `local_backend/range_equippables.py` |
| RRPS | `local_backend/read_range_pawn_state.py` |
| LR | `local_backend/launch_range.py` |

All multi-byte values are little-endian. "u64 [x]" means an 8-byte read at address x. Offsets are for
the pinned 13.06 client and come from the cited line. None were inferred for this document.

---

## 1. Summary

Production reads, in the order a session reaches them. The cadence column gives the Python cadence.
Section 7 maps each read to the 128 Hz Rust tick.

| # | read | python entry | trigger and cadence (Python) | wire effect | needed? |
|---|---|---|---|---|---|
| R1 | Ares handler keys (memory scan) | CLK:42-82 via H:1465-1482 | every received datagram until it succeeds; one attempt scans up to 30 s or 4 GiB; 0.03-7.8 s observed (12 scans, every one succeeded on the first datagram) | every packet: decrypt C->S, encrypt S->C | yes (no protocol source) |
| R2 | packet-notify counters (2nd scan + chain) | ILPN:13-128 via H:2115-2126 | once per connection, just before the challenge reply; 0.05-9.8 s observed | none (event keys only) | no, diagnostic |
| R3 | reader open + FName key | RES:93-96, FNP:43-64 via H:1591-1598 | first authority pass; retried every pass until it succeeds | gates all oracle features | yes |
| R4 | slot state machines | RES:132-164 | mirror every >= 50 ms; ability stage 3 and purchase stage 5 retries every 0.3 s | AuthCurrentState mirror, registration | yes |
| R5 | state names of a machine | RES:77-87 | the first time each state-machine pointer is seen (cached) | state GUID allocation order, export names | yes (could be pinned per class) |
| R6 | item machine array | RES:177-202 | registration (plan) and mirror pass for items with extra machines | extra-machine GUIDs + mirror (observed: guns `ZoomStateMachine`, Tailwind E `StateMachine_Priming`, Updraft Q `BoostStateMachine`) | yes |
| R7 | resource amounts | RES:212-236 | mirror pass, per equippable with resources | AuthResourceAmount | yes |
| R8 | firing seed | H:946-964 | mirror pass, current equippable with resources (range_bots on) | bot hits/damage | replaceable (input events) |
| R9 | camera POV | RES:66-74 | per detected shot; per Cloudburst throw transition | bot hits, projectile spawn transform | replaceable (ServerMove) |
| R10 | input replication | RES:247-268 | every >= 0.2 s after the inventory chain | resume correction | yes |
| R11 | pending input bounds | CIE:42-65 via H:1519-1531 | per authenticated ch8 datagram that decodes to exactly one input stream; read **after** that datagram arrived | rejects input ACK | yes for parity (filter); must not come from a pre-receipt snapshot (§7.2) |
| R12 | ult requirement | H:1824-1845 | every authority pass until sent once | NumUltimatePoints | pinnable (8 observed) |
| R13 | bottom-HUD widget + cinematic flag | H:1991-2007 | every >= 1.0 s while fewer than 6 refreshes have been sent | SpawnedCharacter refresh | yes |
| R14 | client projectile snapshot | RP:308-343 via H:1061-1066 | every >= 50 ms per flying Cloudburst | smoke zone location/time | yes |
| R15 | ClientSetHUD field index | RRI:48-126 via H:3782-3791 | once per connection at the HUD-set stage | HUD RPC header bits | pinnable (52/178 observed) |
| R16 | gameplay-tag net indices | GTN:42-106 via H:4049-4068 | once per responder process (<= 3600 s, §5), at starting equippable i = 2, `ability_activation` on | ability open bodies | pinnable after one capture |
| R17 | saved moves | RMN:45-179 via H:4424-4430 | inside the >= 0.1 s session-ACK branch, for a source after ClientRestart, when >= 50 ms since the last move ACK send | move ACK value, initial correction | replaceable (ServerMove) |
| R18 | process liveness | H:148-163 via H:1447-1453 | on a recv timeout (0.25 s idle), at most once per 1 s | ends the run | yes |
| R0 | process discovery + image base | LR:56-98, LR:110-120 | launcher, once; `find_client` again after every responder exit under `--keepalive` (LR:304-322) | none | yes |

Order inside one received datagram (Python, every send takes the next server sequence, so the Rust tick must
consume oracle values in this order): R1 (keys missing only) -> `parse_handshake` fails -> R11 input ACK
(H:1499-1563) -> authority pass (H:1566-2078: R3 reader, mirror R4 then per equippable R9 smoke throw, R8/R9 bot
shots, R7, R6, main state; R10; Range HUD from `hud_rpc_cache`; R12; purchases incl. R4/R6 at stage 5; R13; R14;
abilities incl. R4/R6 at stage 3) -> control elif chain (H:2107-2172): R2 before the challenge reply, or the
session-ACK branch with the bring-up stage chain (R15, R16) and then R17 (H:4412).

---

## 2. Process attach, handles, read primitive

### 2.1 Discovery (launcher, LR:56-120)

- `CreateToolhelp32Snapshot(0x2)`, case-insensitive match on `VALORANT-Win64-Shipping.exe` (LR:24, 60-78).
  0 matches -> not running. More than 1 -> error `Expected one VALORANT client, found N` (LR:81-82).
- Image base: `OpenProcess(0x0400 | 0x0010)`, `EnumProcessModules`, take `modules[0]` (LR:86-96).
- Project check: the process path must equal
  `<repo>\game req\VALORANT_GAME\VALORANT\live\ShooterGame\Binaries\Win64\VALORANT-Win64-Shipping.exe` (LR:110-120).
- Passed to the responder as `--live-client-pid`, `--live-client-image-base 0x..` (LR:217-218). H refuses any
  stale-base fallback (H:1470-1471).

### 2.2 Handles and access masks

| opener | mask | lifetime | source |
|---|---|---|---|
| `resolve_rpc_index._mk` | `0x10 \| 0x1000` (VM_READ, QUERY_LIMITED) | `EquippableStateReader`: process lifetime (global, H:885, 1592-1595). `resolve()`: per call. `read_pending_input_bounds`: per call. Tag reader: per call (H:4054-4059) | RRI:28-39 |
| `read_move_number._reader` | `0x10 \| 0x1000` | per `read_saved_moves` call | RMN:31-42 |
| CLK `kernel32` (own WinDLL, `use_last_error`) | `0x0410` (QUERY_INFORMATION, VM_READ; `VirtualQueryEx` needs it) | per scan call | CLK:31, 44, 116 |
| `client_process_alive` | `0x00100000` (SYNCHRONIZE) | per check | H:155-163 |
| `inspect_packet_notify` (R2, diag) | `0x0410`, plus the one inside its own key scan | per call | ILPN:16-19 |

Rust: one persistent `0x0410` handle covers the scan and every read. The liveness check can use a
`SYNCHRONIZE` handle opened once. Opening fewer handles changes no read result.

### 2.3 Read primitive (RWS:64-82)

```
read_bytes(h, addr, size):
  if addr < 0x10000 or addr > 0x00007FFFFFFFFFFF or size > 0x1000: ValueError   (RWS:65-66)
  ReadProcessMemory; if !ok or bytes_read != size: OSError                      (RWS:68-73)
read_u64 / read_u32 = unpack("<Q"/"<I", read_bytes(...))                       (RWS:77-82)
```

- No partial reads. Every failure raises. `CIE.read_u64/read_u32` (CIE:15-20) add `or 0`, which only maps 0 to 0, so they
  also raise.
- `RRI.resolve`'s `readable(p)` (RRI:58) is `bool(p) and read_bytes(p, 8) is not None`. `read_bytes` never returns
  None, so an unreadable non-null pointer **raises** instead of returning False.
- Biggest single read: `GTN._read_block` splits into 0x1000 chunks (GTN:37-39). CLK uses its own RPM with 1 MiB
  chunks (section 4.1).
- Rust: `fn read(&self, addr: u64, buf: &mut [u8]) -> Result<(), ReadError>` with the same bounds
  (`addr >= 0x10000`, `addr <= 0x7FFF_FFFF_FFFF`, `len <= 0x1000` for walker reads) and exact-length success.

### 2.4 Liveness (H:148-163, H:1439-1454)

Checked only in the `socket.timeout` branch (0.25 s with no datagram), at most once per second:
`OpenProcess(SYNCHRONIZE)`. A failed open means dead. Otherwise the process is alive iff
`WaitForSingleObject(h, 0) == 0x102` (WAIT_TIMEOUT). When dead, print
`{"status": "live_client_exited", "live_client_pid": pid}` and leave the receive loop. The output JSON is then
written with `open(path, "x")` (H:4581-4584). Production note: launch.ps1 passes `--seconds 3600 --keepalive`, so
the receive loop also ends at the 3600 s deadline, and the restarted responder reuses the same output path. Every
restarted run therefore ends with an uncaught `FileExistsError` at H:4581 (9 tracebacks, all after
`elapsed_seconds` 3600.0, in `responder_20928…out`). `live_client_exited` never appears in that log. Rust: check every
1 s from the oracle thread, independent of traffic. When the client dies, shut down the same way.

---

## 3. Core decoders

### 3.1 GObjects owner decode (RWS:18-61)

`owner = read_bytes(image_base + 0xD3E7140, 0x80)` (RWS:10, used at RRI:60, RES:31/49, CIE:24, RMN:53, FNP:44, GTN:43).
Python re-decodes on **every** walk. All arithmetic below is on u64 unless marked u32.

```
seed        = u32 owner[0x78..0x7C]
mixed_seed  = seed ^ ((seed ^ (seed >> 15)) >> 12) ^ ((seed << 25) & 0xFFFFFFFF)        // u32
mixed       = 0x2545F4914F6CDD1D.wrapping_mul(mixed_seed as u64)
slot        = mixed % 7
high        = mixed >> 32
encoded     = u64 owner[0x40 + 8*slot ..]
s1          = ((high + 2*slot) % 63) + 1
s2          = ((high + slot) % 63) + 1
slot 0: d = reverse_bits(encoded ^ !( high.wrapping_sub(1) & 0xFFFFFFFF ))
slot 1: d = swap_adjacent_bits(encoded.wrapping_sub((high + 2) & 0xFFFFFFFF))
slot 2: d = rotr(rotr(encoded, s1), s2)
slot 3: d = rotl(rotr(encoded, s1), s2)
slot 4: d = encoded ^ ((high + 8) & 0xFFFFFFFF)
slot 5: d = swap_adjacent_bits(encoded) ^ !((high + 5) & 0xFFFFFFFF)
slot 6: d = reverse_bits(encoded ^ !((high + 12) & 0xFFFFFFFF))
result = d ^ seed
```

- `!x` is the 64-bit complement of the zero-extended u32 (Python `~x & MASK64`, RWS:48, 58, 60).
- `swap_adjacent_bits(v) = ((v >> 1) & 0x5555555555555555) | ((v << 1) & 0xAAAAAAAAAAAAAAAA)` (RWS:26-27).
- `reverse_bits` (RWS:30-35) is **not** a full 64-bit reverse: swap adjacent bits, then 2-bit pairs (masks 0x3333/0xCCCC),
  then nibbles (0x0F0F/0xF0F0), then bytes within 16-bit lanes (0x00FF/0xFF00), then `rotr(v, 32)`. Copy the five
  steps exactly. Do not use `u64::reverse_bits`.
- `s1`, `s2` are always in 1..=63.

### 3.2 GObjects array and object lookup

Decoded pointer -> 0x20-byte header, `struct "<Q8xIIII"` (RWS:127-129):

| off | type | field |
|---|---|---|
| 0x00 | u64 | chunk table pointer |
| 0x08 | 8 bytes | skipped |
| 0x10 | u32 | MaxElements |
| 0x14 | u32 | NumElements |
| 0x18 | u32 | MaxChunks |
| 0x1C | u32 | NumChunks |

`read_object(index)`: `(chunk, i) = divmod(index, 0x10000)`. If `chunk >= len(chunks)`, ValueError. Otherwise
`u64 [chunks[chunk] + 24*i]` (RWS:85-89). The chunk table is read one u64 per chunk.

Header validation differs per caller:

| caller | check | on failure |
|---|---|---|
| RRI.resolve (RRI:64) | `1 <= nc <= 0x14 and 0x800 < ne <= 0x400000` | return None |
| RMN.read_saved_moves (RMN:57) | same as RRI | return None |
| FNP.derive_name_key (FNP:48) | `0x107E2 < ne <= 0x400000 and 1 <= nc <= 0x14` | ValueError |
| RES.find_pawn / find_world (RES:35, 53) | `1 <= nc <= 256` only | ValueError |
| CIE.find_input_component (CIE:28) | `1 <= nc <= 256` only | ValueError |
| GTN.find_tags_manager (GTN:45-46) | none | n/a |
| RWS.main, RRPS (diag) | `1 <= nc <= 0x14 and 0x800 < ne <= MaxElements <= 0x400000 and nc <= MaxChunks` | ValueError |

Rust: a single walker that applies the FNP rule (the strictest rule a production path needs). The reader cannot exist
without passing that rule (R3), so on a live process this differs from Python only when memory is corrupt. Reading the
chunk table in one `8*nc`-byte read is fine.

Object indices used: `0x107E2` GEngine (AresGameEngine) (RRI:67, RES:38/56, CIE:32, RMN:60, FNP:21). The range
`0xEA40..0xFB00` is scanned for the tags manager (GTN:48). Diag only: `5`, `0x1475`, `0x1A9B`, `0x3EAD0`, `0x6C8D`,
`0x70B5` (RWS:134-161).

### 3.3 World chain (RES:30-63, RRI:67-73, RMN:60-66, CIE:32-39)

```
engine     = read_object(0x107E2)
viewport   = u64 [engine + 0xBD0]
world      = u64 [viewport + 0x80]
gi         = u64 [world + 0x1D8]
localplayer= u64 [u64 [gi + 0x40]]          // LocalPlayers.Data[0]
controller = u64 [localplayer + 0x38]
pawn       = u64 [controller + 0x4B0]        // 0 when not possessed
netdriver  = u64 [world + 0x40]
```

No step checks for null except `find_pawn` (`... if controller else 0`, RES:44). A null intermediate leads to a read below
0x10000, which raises ValueError. Cost per walk is `10 + NumChunks` reads (12 measured with 2 chunks).

### 3.4 FName pool (FNP:20-71)

```
pool        = image_base + 0xD548540
entry(idx)  : require 0 <= idx < 0x10000000                                   (FNP:26)
              block = u64 [pool + 0x10 + 8*(idx >> 16)]                      (FNP:28-30)
              require 0x10000 <= block <= 0x7FFFFFFFFFFF                     (FNP:31)
              e = block + 4*(idx & 0xFFFF)
              header = u16 [e + 4]; len = header >> 1; wide = header & 1     (FNP:34-36)
              require 1 <= len <= 1023                                       (FNP:37)
              enc = bytes [e + 6 .. + len*(2 if wide else 1)]                (FNP:39)
decode      : out[i] = enc[i] ^ len ^ key[i & 3]   for every BYTE i          (FNP:69-70)
              wide -> UTF-16LE, else UTF-8 strict                             (FNP:71)
```

- Key derivation (FNP:43-64): walk GObjects (rule above), `engine = u64 [chunk1 + 24*0x7E2]`, require
  `u32 [engine + 0xC] == 0x107E2`, `idx = u32 [engine + 0x18]`, the entry must be narrow with `len == 14`.
  `key[i] = enc[i] ^ 14 ^ b"AresGameEngine"[i]` for i in 0..4. Then fully decode and require `== "AresGameEngine"`.
- Name of an object = `decode(u32 [obj + 0x18])` (ComparisonIndex). The Number at +0x1C is **ignored** everywhere
  except tag sorting (GTN:76, 87).
- Edge cases to copy: `len` is the char count, also for wide names, and is XORed into every byte. If `len >= 256`,
  Python's `bytes(...)` raises `ValueError: bytes must be in range(0, 256)`, so such names fail to decode. A narrow name
  with a byte >= 0x80 that is not valid UTF-8 raises `UnicodeDecodeError`. Rust: return an error in both cases, as
  Python does.

---

## 4. Read specifications

### 4.1 R1 Ares handler keys (CLK:42-82, H:765-770, 1465-1482)

Purpose: the two 32-byte AES-GCM keys of the client's Ares packet handler. `client_send_key` decodes C->S datagrams
(`inspect_client_ares_packet`, `decode_client_ares_packet`, H:445, 541). `receive_key_override` encrypts every S->C frame.

Algorithm:
- `needle = u64 LE (image_base + 0xA5F3BB0)` (handler vtable RVA, H:1472-1473).
- `OpenProcess(0x0410)`. `addr = 0x10000`. Loop while `addr < 0x7FFFFFFF0000`, elapsed `< max_seconds (30.0)` and
  scanned `< 4096 MiB` (the size cap is checked per region only) (CLK:48-52).
- `VirtualQueryEx`. If it returns 0, stop. `next = (BaseAddress or addr) + RegionSize`. If `next <= addr`, stop (CLK:53-58).
- Scan the region iff `State == 0x1000 && Type == 0x20000 && !(Protect & 0x101) && (Protect & 0xEE)` (CLK:59-60).
- Read the region in `min(1 MiB, remaining)` chunks with no overlap, so a needle that straddles a chunk boundary is
  missed. A failed chunk read is skipped. Search every **byte** offset (`data.find`) (CLK:62-78).
- For each hit in ascending order: re-read 112 bytes at the hit. If the read returns 112 bytes and the first 8 equal the
  needle, return `(hit, bytes[0x28:0x48], bytes[0x48:0x68])`. Otherwise try the next hit (CLK:70-76).
- Nothing found -> `RuntimeError` (CLK:82).

Assignment: `handler_address, client_send_key, receive_key_override = read_current_handler_keys(...)` (H:1474-1475).
The **first** key (+0x28) is the client send key. The second (+0x48) is the key used to build server frames.

Cadence/failure (H:1465-1482): the read runs before `parse_handshake` on **every** datagram while
`client_send_key is None`, so even the initial handshake waits for it. On `OSError | RuntimeError | ValueError`:
`event["handler_state"] = "not_ready"`, the event is logged and the **datagram is dropped** (`continue`). On success it
prints the plain line `read-only Ares handler=0x… receive-key fingerprint=<sha256(receive_key).hex()[:12]>` (H:1482).
The keys are cached for the life of the process and are never re-read, even for a new client source port.

Observed: `handler_elapsed_ms` 5579 on the first datagram (`runtime/responder_24252_phase4_match_state.out` line 3,
the plain fingerprint line is line 2). In `responder_20928…out` the 11 process starts scanned in 32-7782 ms. Every scan
in both logs succeeded on the first datagram: `handler_state` / `not_ready` occur 0 times.
Fingerprint `107b74a379f0` in every logged run (01_transport.md §7 Keys).

Exact Python datagram semantics while the keys are missing: the datagram that triggers a **successful** scan is
processed normally afterwards (it is not dropped), and datagrams that arrive during the blocking scan wait in the OS
socket buffer and are processed in order once it returns. Only a datagram whose own scan attempt **raised** is dropped
as `not_ready`.

Rust: run the scan on the oracle thread, never on the tick thread. Starting it at attach and retrying about every
1 s is a design choice. The claim that the handler only exists after the client connects to the Range is
unverified: Python never scanned before the first datagram. For parity, hold (bounded queue, arrival order)
the datagrams that arrive while a scan is in flight and process them when it succeeds. Drop as `not_ready` only when a
scan fails. Keep the byte-offset search, the 1 MiB non-overlapping chunks and first-hit-wins.
Log the fingerprint, never the key bytes (01_transport.md §18 Q1).

### 4.2 R2 packet-notify inspection (ILPN:13-128, H:2115-2126) — diagnostic

Production reaches it because `session_ack_probe` is in the challenge-branch condition (H:2111, 2115). It runs once per
connection, when the 77-bit authenticated packet arrives after the Hello. It calls `read_current_handler_keys` again (a
full second scan), then walks:
`handler+8 -> packet handler; +448: (components Q, count i32, require 2..8); components+16 -> stateless component;
+0x38 -> NetDriver; driver+192 -> connection; conn+0x70 package map ...; conn+5144 -> incoming channel sequence
array (i32 [0] = control); conn+5112 channels (Q) + count (i32, require 1..65536); conn+6840 notify, counters at
notify+588 (u16 in_sequence & 0x3FFF, …)`, plus log-verbosity bytes at `image+0xD414578` and `image+0xD39B290`
(ILPN:31-127, full chain in section 8).

Only two values are used: `event["pre_control_sequence"]` and `event["pre_in_sequence"]` (H:2125-2126). On
`OSError | RuntimeError | ValueError`: `event["pre_state_error"] = type(error).__name__`, then continue (H:2120-2123).

Cost: it blocks the challenge reply. `handler_elapsed_ms` 4797 on that event (`responder_24252…out` line 1768). In
`responder_20928…out` it ran 16 times (once per session, never `pre_state_error`), 46-9844 ms.

Rust: **do not port.** It has no wire effect. Note that dropping it removes a ~5 s delay before the control-type-3
reply, a timing change to confirm in the first live run. If it is wanted for debugging, make it an asynchronous
oracle query whose result is only logged.

### 4.3 R3 `EquippableStateReader` (RES:90-118, H:1587-1598)

Created lazily in the authority pass (gate H:1566-1568: keys present, pid and base set, source in
`client_restart_sent_sources`). It holds one handle and runs `derive_name_key` (R3 = FNP §3.4). It is stored globally in
`pistol_mirror_reader["reader"]` (H:885) and shared by every source. On any exception: `state_reader = None`,
`event["equippable_reader_error"] = repr(e)[:200]`, and creation is retried on the next pass. Every oracle feature in the
pass checks `state_reader is not None`.

Caches on the reader (never invalidated):
- `_names: {sm_ptr: {state_name: state_ptr}}`, insertion order = live TMap order (RES:97, 110-111, 156-157, 193-194).
- `_class_names: {object_ptr: name}` for items, machines, resource components and projectile actors
  (RES:98, 147-152, 189-192, 226-229; RP:330-334).

`reader.read()` (RES:103-118) is not on the production path. Only the CLI uses it.

### 4.4 R5 state names (RES:77-87)

```
data  = u64 [sm + 0x160]; count = u32 [sm + 0x168]
if data == 0 or not 0 < count <= 64: return {}
for i in 0..count: state = u64 [data + 24*i]; if state: names[decode(u32 [state + 0x18])] = state
```

- The 24-byte stride and "first u64 = state object" come from RES:5-6, 84. Holes in the TMap are not checked: a non-null
  junk value raises during decode and fails the whole read. A duplicate name keeps its first position and takes the
  last pointer.
- **Order matters for the wire.** State GUIDs are allocated in this dict order. The oracle must return names in TMap
  index order 0..count-1, skipping nulls. Exact allocation sequence on the per-source allocator at registration:
  1. Extra machines first (`plan_extra_machines`, H:1163 for abilities, H:1302 for purchases): for each machine of
     R6 in item-array order that passes the filter (`address != main sm` and `starting` set), one GUID for the
     machine, then one per state in that machine's TMap order (H:1113-1118, kwargs evaluated in order).
  2. Then the main machine's states in TMap order, **excluding `InactiveState`**:
     - abilities: `names = [n for n in _names[sm] if n != "InactiveState"]` (H:1159-1160), allocated by the dict
       comprehension at H:1164. InactiveState already has `load.inactive_guid` (range_abilities.py:65).
     - purchases: `gun_equippable` allocates only names not already in `load.state_guids`
       (range_gun_loader.py:231-233). InactiveState was allocated earlier, at stage 3 (`gun_initial_state_body` ->
       `gun_equippable`, range_gun_loader.py:227-230, 243-245).
- Quirk: an empty result (TMap not yet populated) is cached for that SM forever (RES:110-111, 156-157). See Q4.
  Registration then proceeds with zero state names, and that machine is never mirrored. This was not observed in
  production (every `ability_ready` / `purchase_ready` in `responder_20928…out` has a stable non-trivial count, e.g.
  Smoke 11, Dash 15, CycloneBoost 12, Dagger 35, Vandal 9, Operator 10).

### 4.5 R4 slot state machines (RES:132-164)

```
pawn = find_pawn(); inv = u64 [pawn + 0xBF8] (0 if pawn == 0); if inv == 0: return {}
current = u64 [inv + 0x278]
for slot in slots:
  item_slot = u64 [inv + 0x1B8 + 8*slot]; item = u64 [item_slot + 0x38] (0 if item_slot == 0); skip if 0
  class_name = cached or decode(u32 [u64 [item + 0x10] + 0x18])          // cached per item pointer
  sm = u64 [item + 0xE00]; skip if 0
  names = cached or state_names(sm)
  predicted     = name_of(u64 [sm + 0x1B0])   // None if pointer not in names
  authoritative = name_of(u64 [sm + 0x1C8])
  is_current    = item == current
```

Callers and cadence:
- Mirror (H:1603-1613): `slots = [e.slot for e in mirror_ready_equippables(source)]` (H:1316-1331), every >= 0.05 s per
  source (`equippable_mirror_polled` is set before the read). In production the slot order is pistol 1, melee 2,
  C 3, Q 4, E 5, X 9 (once registered), then purchases. On exception: `machines = {}`,
  `event["equippable_mirror_error"]`, and nothing is sent for any equippable this pass.
- Ability stage 3 (H:1144-1157) and purchase stage 5 (H:1286-1300): one slot. On exception the event is
  `ability_register_error` or `purchase_register_error`. `None` or a class mismatch increments `registered_tries`,
  retries after 0.3 s, and gives up on the 21st failure (`> 20`) with `*_register_gave_up`.

A machine is accepted only if `machine.class_name == equippable.class_name` (H:1617, 1150, 1293).

### 4.6 R6 item machines (RES:177-202)

`array = u64 [item + 0xE30]`, `count = u32 [item + 0xE38]`. If `array == 0` or `count` is not in 1..=8, return []. For each
`machine = u64 [array + 8*i]` (skip 0): `name = decode(u32 [machine + 0x18])` (cached), `names = state_names(machine)`,
`predicted = [machine + 0x1B0]`, `authoritative = [machine + 0x1C8]`, `starting = [machine + 0x1E8]`,
`state_names = tuple(names)`.

- `plan_extra_machines` (H:1102-1118): keeps machines with `address != main sm` and `starting` not None. Allocates
  `guid` first, then one GUID per state in `state_names` order. On exception: `event["extra_machines_error"]`, return [].
- Mirror (H:1669-1697): `{m.name: m}`. On exception: `{}` + `machine_mirror_error`. Then each extra gets `predicted None`,
  so nothing is sent. Only for equippables with `mirror_states` true and non-empty `extra_machines`.
- Extra machines seen in production: guns `ZoomStateMachine` (44 of 56 registrations), Tailwind
  `StateMachine_Priming`, Updraft `BoostStateMachine`. Cloudburst and Blade Storm have none.

### 4.7 R7 resource amounts (RES:212-236)

For each `ResourceSubobject` (RE:22-37): `component = u64 [item + item_offset]` (skip 0). Its decoded name (cached) must
equal `resource.name`, else skip. Then `(predicted, authoritative) = "<ii" [component + 0x10C]` and
`default = i32 [component + default_offset]`, where `default_offset` is 0x12C for ammo and 0x188 for ability charges.

| equippable | resources (name, item_offset, default_offset) | source |
|---|---|---|
| BasePistol (slot 1) | MagazineAmmo 0x1208, ReserveAmmo 0x1200, default 0x12C | RE:326-327 |
| bought guns | MagazineAmmo 0x1208, ReserveAmmo 0x1200, default 0x12C | range_gun_loader.py:238-239 |
| Cloudburst C (index 2, slot 3) | EquipmentCharge 0x1240, default 0x188 | RE:272 |
| Updraft Q (index 4, slot 4) | EquipmentCharge 0x12A0, default 0x188 | RE:273 |
| Tailwind E (index 3, slot 5) | SignatureAbilityResource 0x1290, default 0x188 | RE:274 |

Mirror pass only (H:1635-1666). On exception: `{}` + `resource_mirror_error`. The values feed `resource_target`
(RE:217-242).

### 4.8 R8 firing seed (H:944-964, 1622-1632)

Gate: `range_bots and machine.is_current and equippable.resources`, in the mirror pass. `firing = names["FiringState"]`,
or else the first name in TMap order that starts with `"Firing"`. If there is none, return None.
`rng = u64 [firing + 0x4A0]` (0 -> None), cached in `firing_rng_components[(sm_ptr, equippable.key)]` forever.
`seed = u32 [rng + 0xEC]`. Any exception -> None, silently. A change from the previous seed value
(`firing_seeds[(source, key)]`) counts as **one** shot per poll (`bot_shots(..., 1, ...)`), however many shots
happened in the 50 ms.

### 4.9 R9 camera (RES:66-74)

`controller = find_controller()`, `manager = u64 [controller + 0x528]` (0 -> None).
`"<6d" [manager + 0x17C0]` = location x, y, z, then pitch, yaw, roll (RES:67-68, 73-74).
- `bot_shots` (H:978-984): exception -> `bot_camera_error`, the shot is lost. None -> return.
- `cloudburst_state` (H:1022-1032): `previous_smoke_state` is updated **before** the read (H:1023), so a failed camera
  read loses that throw (`cloudburst_camera_error`).

### 4.10 R10 input replication (RES:247-268, H:1713-1747)

`pawn = find_pawn()`, `input = u64 [pawn + 0xB90]`, `inv = u64 [pawn + 0xBF8]`. If either is 0 -> None.
`eq = u64 [inv + 0x260] or u64 [inv + 0x278]` (desired before current). `class = decode(u32 [u64 [eq + 0x10] + 0x18])`
(not cached). `stopped = u8 [input + 0x12C] != 0`, `requested = u32 [input + 0x130]`, `inventory = u32 [inv + 0x2F0]`.

Gate: `input_correction_responder`, `auth_correct_rep_index >= 5`, starting equippables done, and >= 0.2 s since the last
poll. On exception: None + `input_correction_error`. When stopped, and >= 1 s since the last send, the correction index is
`max(requested, inventory, auth_correct_rep_index) + 1`. The GUID comes from
`purchased_class_guids[class] or equippable_guid_for_class(class)` (range_server_bunch.py:803-810).

### 4.11 R11 pending input bounds (CIE:42-65, H:1499-1531)

New handle per call. `input = find_input_component()` (world chain, then `[pawn + 0xB90]`; 0 -> None).
`stopped = u8 [input + 0x12C]` (unused), `cursor = u32 [input + 0xA84]`, `count = u32 [input + 0x1198]`. If
`count > 4096`, ValueError. `latest = (cursor + count) & 0xFFFFFFFF`.

Gate: `input_ack_cursor_trial`, source after ClientRestart, keys present, `channel_index == 8`, and the authenticated
datagram decodes to exactly one input stream. `OSError | ValueError | struct.error` gives None. If the result is None or
`last_event_id > latest`, the stream is dropped and `event["input_ack_rejected_event_id"]` is set. This filter matters: the
largest production log (`responder_20928…out`, still being appended by a running responder; counted 2026-10-14 17:20)
has 70 rejections over 29 distinct IDs, all between 9.5e7 and 3.75e9 (3738288983 ×38, 3261836246 ×5, 27 others ×1),
against 180980 accepted input ACKs whose IDs range 1..338975. Without the filter, `last_input_ack_numbers` would jump to
about 3.7e9 and block every later input ACK.

Timing is part of the contract: Python reads the bounds synchronously **after** the datagram arrived, so the client's
generated bound is always >= every event ID the packet carries and a legitimate stream is never rejected. A bound
taken before the datagram arrived can be lower than the packet's newest ID (the client generated and sent it after the
read). That rejects legitimate streams, and under continuous input it can starve input ACKs. See §7.2.

### 4.12 R12 ult requirement (H:1824-1845)

Gate: `range_money_probe`, source in `range_money_sent_sources`, not yet sent, reader present. Checked on **every**
authority pass until it is sent. `ult = i32 [find_pawn() + 0x14F4]`. On exception: 0 + `ultimate_points_error`. Sent iff
`0 < ult < 50`. Observed: 8 in all 16 sessions of `responder_20928…out` (`sent_ultimate_points` ×16).

### 4.13 R13 HUD refresh (H:1984-2023)

Gate: `hud_refresh`, reader present, starting equippables done, >= 1.0 s since the last poll, `hud_refresh_counts < 6`.
`hud_refresh_polled` is set before the read. The poll keeps running every 1 s for the whole session, because a session
sends one refresh and the count stays below 6 (`sent_hud_refresh: 1` ×16, never 2, in `responder_20928…out`).
```
controller = find_controller(); pawn = u64 [controller + 0x4B0] (0 if !controller)
widget = u64 [pawn + 0xB80] (0 if !pawn)                  // Widget_BottomHUD (comment H:1985)
collapsed = widget != 0 and u8 [widget + 0x11C] == 1
cinematic = (u32 [controller + 0x690] & 2) != 0           // read even when controller == 0 -> ValueError
```
On exception: `(collapsed, cinematic) = (False, True)` + `hud_refresh_error`. A refresh is sent iff `collapsed and !cinematic`.

### 4.14 R14 projectile snapshot (RP:308-343, H:1046-1100)

```
world = find_world()
driver = u64 [world + 0x40] if world else 0            // RP:319, guarded: world 0 -> None, no exception
conn = u64 [driver + 0xC0] if driver else 0            // RP:320, guarded
if !conn: None
channels = u64 [conn + 0x13F8]; count = u32 [conn + 0x1400]
if !channels or !(0 <= ch < count): None
actor = u64 [u64 [channels + 8*ch] + 0x78]; if !actor: None       // (actor channel 0 -> None)
class = cached or decode(u32 [u64 [actor + 0x10] + 0x18])
root = u64 [actor + 0x290]; location = "<3d" [root + 0x320] if root else None
movement = u64 [actor + 0x610]
if !movement or location is None: Snapshot(class, location, 0, (0,0,0), (0,0,0))
collisions = u32 [movement + 0x198]
hit = 0x48 bytes [movement + 0x228]; hit_location = "<3d" hit[0x00]; hit_normal = "<3d" hit[0x30]
```

Called for each flight in state `flying` every >= 0.05 s per source (`projectile_ticks`). `ch` is our channel number
(10000+, RP:60). On exception: `cloudburst_read_error`, and the snapshot is None. `flight_stop_point` (RP:346-360) then
uses `last_location` or the ballistic fallback. The class must equal `Projectile_Wushu_4_Smoke_C` (RP:348).

### 4.15 R15 RPC field index (RRI:48-126, H:3769-3817)

```
new handle (_mk); GObjects rule (RRI form) else None (RRI:64-65)
… world chain in this read order: engine, viewport, world, netdriver = [world+0x40], gi, lp, controller,
  pawn = u64 [controller + 0x4B0] (read for both kinds, unguarded)        (RRI:67-73)
actor = controller (kind "controller") or pawn; if !actor: None           (RRI:74-76)
cls = u64 [actor + 0x10]
u32 [actor + 0xC]  (read, unused: `actor_class_index`, RRI:78; can still fail the call)
mgr = u64 [netdriver + 0x198]; set = u64 [mgr + 0x8]; n = u32 [mgr + 0x10]
require set != 0 and 0 < n <= 0x10000 else None
cls_index = u32 [cls + 0xC]   (read after the set check, RRI:110)
chosen = first i in 0..n with (u64 [set + 24*i] & 0xFFFFFFFF) == cls_index -> u64 [set + 24*i + 8]; none -> None
   (no TSet hole check: every one of the n 24-byte slots is read until the first match)
scan_chain(c = chosen), depth < 16, while c != 0 and readable(c):
   base = u32 [c]; count = u32 [c + 0x28]; entries = u64 [c + 0x20]
   if count and 0 < count <= 4096:
      class_max = max(class_max, base + count)
      if readable(entries):
        for j in 0..count: tag = u64 [entries + 24*j]
           if tag & 1: fn = tag & !1; if readable(fn) and u64 [fn + 0xE0] == thunk: field_index = u32 [entries + 24*j + 8]
   c = u64 [c + 0x8]
return None if field_index is None else {field_index, class_max_index: class_max}
```

- The scan does not stop at the first match: a later (parent) cache match overwrites an earlier one (RRI:96-104).
- `class_max` is the maximum `base + count` over the whole chain. The wire uses `SerializeInt(field, class_max + 1)`.
- Production call: `thunk = image_base + 0x4B6B340`, kind `"controller"`, once per source at the HUD-set stage
  (H:3784-3786). The result is stored in `hud_rpc_cache[source]` and reused by the Range HUD rebuild (H:1793).
- Failure: `None` raises `ValueError("ClientSetHUD absent from live controller cache")`, and any read error propagates.
  Neither is caught: it is inside the `except ValueError` handler of the `parse_handshake` try (H:1483-1485), and
  `main()` calls `run_probe` without a try (H:4915), so the **responder process exits** with a traceback and no
  output JSON. Under launch.ps1's `--keepalive`, launch_range restarts the responder for the same pid/base
  (LR:304-322): every process-scope cache is lost (R1 rescans on the next datagram) and the client's live session
  gets no further replies from the new process. Recommended Rust divergence: log a warning and retry the stage next
  tick (04_bringup_b §10.4 Q2).
- Observed: field 52, max 178 in 16/16 sessions (`responder_20928…out`, `hud_rpc_field_index` ×16). Never slow enough to
  log `handler_elapsed_ms` (< 25 ms). MOVE_ACK_FINDINGS.md:101-105 documents the chain.

### 4.16 R16 gameplay-tag net indices (GTN:42-106, H:4045-4068)

Find the manager: scan GObjects indices `0xEA40..0xFB00` (chunk 0). For each non-null object, `name = decode([obj+0x18])`
and `class = decode([[obj+0x10]+0x18])`. Decode exceptions are skipped. Settings: class `GameplayTagsSettings` and a
name starting with `Default__`. Manager: class `GameplayTagsManager` and a name not starting with `Default__`. Stop when
both are found. Otherwise ValueError (GTN:48-64). A read failure on the object pointer itself (outside the try) propagates.

```
data = u64 [mgr + 0x130]; count = u32 [mgr + 0x138]; capacity = u32 [mgr + 0x13C]
raw = read capacity * 0x28 bytes at data (in 0x1000 pieces)
entry k: comparison = u32 raw[0x28k], number = u32 raw[0x28k+4], node = u64 raw[0x28k+0x10]; skip node == 0
         name = decode(comparison) (exceptions skip the entry)
require len(entries) == count
sort entries by (name.lower(), number)                                // Python str.lower
order = names in sorted order
common = u64 [mgr + 0x180]; ccount = u32 [mgr + 0x188]
for i in 0..ccount: name = decode(u32 [common + 12*i]); if name in order: swap(order[index(name)], order[i])
true_bits = ceil(log2(len(order) + 1))                                 // integer: bit length of len(order)
first_segment = min(true_bits, i32 [settings + 0xBC])
constructed = u16 [entries[0].node + 0x50] != 0xFFFF                  // entries[0] = first in sorted order
indices = constructed ? {name: u16 [node + 0x50]} : {name: position in order}
```

- `math.ceil(math.log2(n + 1))` equals the bit length of `n` for n >= 1. Rust: `64 - n.leading_zeros()`.
- Caller (H:4045-4068): gate `ability_activation and equip_i in starting_ability_tunings` (2, 3, 4;
  range_abilities.py:148-155), once per **process** (`tag_net_index_cache`, H:935). It uses its own temporary
  `EquippableStateReader` (new handle, new key), closed afterwards. It raises if `first_bit_segment < true_bit_num`. Any
  exception: `ability_open_tuning_error`, and the plain `build_starting_equippable_open(i)` is sent instead, which is a
  different wire body. Retry semantics: the cache is filled as soon as `predict_net_indices` returns (H:4056), before
  the segment check (H:4061) and before the tag lookup in `build_ability_open_with_tuning` (KeyError,
  range_abilities.py:177). So only reader-open and read failures are retried at the next tuning index. A segment or
  missing-tag failure repeats from the cache for i = 3, 4 and for every later session of the same process.
- Tags actually used: `AbilityTuning.Wushu.4.SmokeDuration`, `…4.SmokeMaxCharges`, `…E.DashVelocityMultiplier`,
  `…E.PrimeDuration`, `…E.MaxCharges`, `…Q.MaxCharges` (range_abilities.py:148-155). A missing name -> KeyError ->
  same fallback.
- Observed: `ability_open_tuning [i, 10, true]` for i = 2, 3, 4 in every session (16 × 3 in `responder_20928…out`; width
  10, constructed). The six indices were never logged. Cost: `handler_elapsed_ms` 141 (`responder_24252…out` line 3133),
  156-157 ms for the 11 per-process reads in `responder_20928…out`. The other 5 sessions reused the process cache.

### 4.17 R17 saved moves (RMN:45-179, H:4408-4535)

New handle per call. On the production path (module imported, so the `__name__ == "__main__"` block at RMN:84-113 is
skipped):
```
GObjects rule (RRI form) else None; world chain; pawn = [controller + 0x4B0]; if !pawn: None
-- values read but unused in production (each read can still fail the call):
u8 [pawn+0x224], u8 [pawn+0x128], u64 [pawn+0x4A0], u64 [pawn+0x510], u64 [pawn+0x518],
u64 [controller+1304], u64 [controller+1296], movement = u64 [pawn+0x4F8], vt = u64 [movement],
u64 [vt+2944], u64 [vt+2920], u8 [movement+0xC1], u8 [movement+0x13D8]
pd = u64 [movement + 0x7E8]; if !pd: None
count = u32 [pd + 0x28]; u32 [pd + 0x68] (unused); base = u64 [pd + 0x20]
last = u64 [pd + 0x50]; last_acked = u32 [last + 0x1C] if last else None
require 0 <= count <= 4096 and base != 0 else None
for i in 0..min(count, 256): mp = u64 [base + 16*i]; if mp:
    first non-null mp: u64 [mp] (vtable), u64 [vtable + 16]
    numbers.append(u32 [mp + 0x1C])
    if i == min(count,256)-1: ts_bits = u32 [mp + 0x3C]; "<ddd" [mp + 1080]; "<ddd" [mp + 592]
if !numbers: None
→ count, oldest = numbers[0], newest = numbers[-1], last_acked, newest_timestamp (f32 of ts_bits or None)
```

Precision: `newest` is the number of the last **non-null** entry among the first `min(count, 256)` slots, so with
`count > 256` it is slot 255's number, not the real newest. `newest_timestamp_bits` is read only when slot
`min(count,256)-1` itself is non-null, otherwise it is None.

Gate (H:4412-4414): `move_ack_probe and source in client_restart_sent_sources and now - last_move_ack_times[source]
>= 0.05`.

Use (`move_ack_value_source = "number"`): `ack_value = newest`. Send iff `newest > last_move_ack_numbers[source]` (initial
-1). The first send with `count > 0` carries the placement correction to `move_correction_location`. Every later send is
`build_input_ack_rpc_payload(0xFFFFFFFF, newest, 41, 53)` (H:4437-4495). Event keys: `move_ack_newest_number, _timestamp,
_oldest, _last_acked, _count`. `OSError | ValueError | struct.error` -> `{}` and no send this pass.

Cadence: the block sits inside the session-ACK branch (H:2172, indentation 28 under the `if decoded_packet` at 24), which
fires at most every 0.1 s per source. It runs when `>= 0.05 s` have passed since the last move-ACK **send**
(`last_move_ack_times` is set only on send, H:4535). So it runs on every session-ACK pass while no new move exists:
at most about 10 Hz.

Rust minimal read set with the same result on a healthy process: `pd`, `count`, `base`, `last`, the 16-byte array as
one `16*min(count,256)`-byte read (at most 0x1000), then `u32 [mp+0x1C]` for the first and last non-null entries and
`u32 [mp+0x3C]` of slot `min(count,256)-1` when that slot is non-null. Dropping the unused reads removes failure modes
only. A snapshot older than the triggering datagram only lowers `newest`, which the strictly-increasing gate tolerates.

### 4.18 Dead and diagnostic reads (not ported)

- `resolve(…, image_base + 0x6244D00, "controller")` ClientGamePhaseBegin (H:3829-3835): `client_game_phase_begin_probe` OFF.
- `resolve(csvt_thunk)` ClientSetViewTarget (H:4376-4383): `csvt_probe` OFF.
- `newest_saved_location` (+592) and `newest_move_direction` (+1080): only for follow/track modes, which are OFF.
- `newest_timestamp_bits` as the ACK value (`move_ack_value "timestamp"`) and `move_ack_fixed`: OFF.
- RMN:84-113 (`__main__`-only movement queue/fixed-step dump): never runs when imported.
- `read_range_pawn_state.py`: CLI diagnostic, never imported by the responder. Offsets in section 8 only for reference.
- `capture_input_event_cursor.capture()` (CIE:68-103, adds `+0x1190`): CLI.
- `read_world_state.main` (engine class 0x1475, game mode world+0x170, classes 0x1A9B/0x3EAD0, functions 0x6C8D/0x70B5): CLI.
- `check_live_packet_keys.scan_process`, `candidate_keys`, `read_current_handler_receive_key`: CLI or unused.
- `EquippableStateReader.read`, `read_fname_pool.main`, `read_equippable_state.main`, `gameplay_tag_net_index.main`, RRI `__main__`: CLI.
- Launcher health check `read_saved_moves` loop (LR:340-358): skipped by `--no-health-check` (launch.ps1 passes it).
- `--base-pistol-initial-cosmetic-trial`: becomes env `RANGE_BASE_PISTOL_INITIAL_COSMETIC_TRIAL=1` (H:4908-4909; read
  at H:737-739). It reads skin JSON files, not memory. Not an oracle concern.
- Env `RANGE_ACR_ACTOR_INSPECTION_PROBE`, `RANGE_EQUIP_STATE_TRIAL`, `VALORANT_RANGE_CAPTURE_LOGIN`: OFF, no memory reads.

---

## 5. Cross-cutting state

| state | scope | key -> value | source |
|---|---|---|---|
| `client_send_key`, `receive_key_override` | process | 32 bytes each, set once | H:767-768, 1474 |
| `pistol_mirror_reader["reader"]` | process | handle + FName key + caches | H:885, 1592-1595 |
| `reader._names` | process | sm_ptr -> ordered {state: ptr}, never invalidated, caches `{}` | RES:97 |
| `reader._class_names` | process | object_ptr -> name, never invalidated | RES:98 |
| `firing_rng_components` | process | (sm_ptr, equippable.key) -> rng ptr | H:944, 961 |
| `tag_net_index_cache["indices"]` | process | TagNetIndex | H:935, 4051-4060 |
| `hud_rpc_cache` | source | (field_index, class_max) | H:907, 3791 |
| `firing_seeds` | (source, key) | last seed | H:941, 1626-1632 |
| `equippable_mirror_polled`, `input_correction_polled`, `input_correction_sent_times`, `hud_refresh_polled`, `hud_refresh_counts`, `projectile_ticks`, `last_move_ack_times`, `last_move_ack_numbers`, `last_input_ack_numbers` | source | monotonic seconds or counters | H:892-897, 942-943, 967, 1359-1361 |
| `projectile_fields[source].previous_smoke_state` | source | last predicted smoke state | RP:95, H:1023 |
| `Flight.opened / seen / last_location / state` | per Cloudburst flight | R14 consumer state: `seen` + `last_location` set by every good snapshot; "lost" uses `last_location`; unseen for >= 0.3 s -> ballistic fallback | RP:70-86, 346-360 |
| job `registered_tries` | per job | int, starts 0 (H:1895, 1902, 2073), give up at > 20 | H:1151-1157, 1294-1299 |
| `last_client_alive_check` | process | monotonic seconds of the last R18 check | H:1439, 1447-1449 |

"Process" scope in production means one responder run: launch.ps1 passes `--seconds 3600 --keepalive`, so the run ends
at the 3600 s deadline and launch_range restarts it (LR:304-322). In `responder_20928…out`: 11 process starts, 16
sessions. Every restart loses the keys, the reader and its address caches, the tag cache and all per-source state. R1
rescans on the next datagram. A long-lived Rust server has no such reset, so its process-scope caches live across many
sessions. That makes the address-cache hazard below worse; see Q5.

Address-keyed caches are a hazard. A freed item whose address is reused by a new object keeps its old class name.
Bought guns are the realistic case. An unverified candidate cause for the 4 `purchase_register_gave_up` events in
`responder_20928…out` (Phantom ×2, Odin, Outlaw). See Q5.

---

## 6. Failure mapping (what the game logic does when the oracle fails)

| read | exception -> | absent -> | event key |
|---|---|---|---|
| R1 keys | drop datagram | (scan returns or raises) | `handler_state: "not_ready"` |
| R2 notify | continue | n/a | `pre_state_error` |
| R3 reader | all oracle features skip this pass | n/a | `equippable_reader_error` |
| R4 mirror | no mirror sends this pass | slot skipped | `equippable_mirror_error` |
| R4 registration | retry 0.3 s, give up at 21 | same | `*_register_error`, `*_register_gave_up` |
| R6 plan | no extra machines | [] | `extra_machines_error` |
| R6 mirror | no extra-machine sends | predicted None | `machine_mirror_error` |
| R7 | no resource sends | resource skipped | `resource_mirror_error` |
| R8 | no shot | None | none |
| R9 bots / smoke | shot lost / throw lost | return | `bot_camera_error`, `cloudburst_camera_error` |
| R10 | no correction | None | `input_correction_error` |
| R11 | stream rejected | None | `input_ack_rejected_event_id` |
| R12 | retry next pass | 0 -> retry | `ultimate_points_error` |
| R13 | treat as (False, True) | widget 0 -> not collapsed | `hud_refresh_error` |
| R14 | snapshot None | None | `cloudburst_read_error` |
| R15 | **process exits** (recommend retry) | **process exits** | none |
| R16 | plain open sent | n/a | `ability_open_tuning_error` |
| R17 | no ACK this pass | `{}` | none |

Production error rates: none of the `*_error` keys (including `pre_state_error`) and no `handler_state` appear in
`responder_20928…out` (1.30 GB, 16 sessions, 11 process runs; re-counted 2026-10-14 17:20 while a responder was still
appending) or in `responder_24252…out`. The only non-oracle failures there are the 9 `FileExistsError` exits of §2.4.

---

## 7. Rust oracle at 128 Hz

The user requirement is a 128 tps external server with a console log. The tick (7.8125 ms) must never wait on a memory
read: a key scan takes seconds, a tag table about 140 ms, a full snapshot about 100-300 RPM calls.

### 7.1 Threads

- **Oracle thread** (normal priority) owns the handle, the FName key and the caches. It runs a loop paced at 128 Hz.
  Each iteration (a) serves queued on-demand queries and (b) takes one snapshot of the current watch list.
  It publishes `Arc<Snapshot>` (latest wins) with `taken_at`, `read_us` and `rpm_calls`.
- **Tick thread** reads the latest snapshot at the start of the tick. It runs every Python timer exactly as specified in
  05_authority_loop.md §9 (thresholds in seconds, `>=`, `last = now` on fire). It sends the watch list and queries over
  a channel.
- Python reads synchronously inside the pass that consumes the value. The snapshot is at most one tick plus the read
  time old. The client changes state once per client frame, so this matches within one frame. Edge-triggered logic
  (seed change, smoke-throw transition) still samples at the Python cadence: compare the values seen at poll times, not
  every snapshot.
- **Exception: R11 cannot come from the snapshot.** Its contract is "read after this datagram arrived" (§4.11). The
  bound must come from a read whose start time is later than the datagram's receive time: an on-demand
  `InputBounds` query (§7.3) or the first snapshot with `taken_at` after the receive time. The input ACK for that
  datagram waits for it. It costs 16 reads with 2 chunks (counted offline), well under a tick.
- Python reads a slot's state names the first time a mirror poll or registration touches that slot. A 128 Hz
  snapshot can touch a new item earlier, while its TMap may still be empty, and Python would cache that `{}`
  forever (§4.4, Q4). Put a slot on the watch list only when Python would read it (the
  `mirror_ready_equippables` slots while the 50 ms mirror is due, plus the slot of a registration stage that is due), or
  do not cache an empty result (Q4).

### 7.2 Snapshot (always read once the reader exists)

| field | from | consumer |
|---|---|---|
| `controller`, `pawn`, `inventory`, `current_equippable` | §3.3, RES:137-140 | everything |
| `camera: Option<([f64;3],[f64;3])>` | R9 | bot shots, Cloudburst |
| `slots: Vec<SlotMachine{slot, item, class_name, sm, predicted, authoritative, is_current}>` | R4 for watch-list slots, in order | mirror, registration |
| `resources: Vec<(item, name) -> (predicted, authoritative, default)>` | R7 for watch-list resources | mirror |
| `item_machines: item -> Vec<Machine>` | R6 for watch-list items | mirror extra machines |
| `firing_seed: sm -> u32` | R8 for the current equippable | bots |
| `input_replication` | R10 | input correction |
| ~~`input_bounds`~~ | R11 | **not a snapshot field**: on-demand `InputBounds` after receipt (§7.1, §7.3) |
| `saved_moves: {count, oldest, newest, last_acked, newest_timestamp}` | R17 | move ACK (a stale value only lowers `newest`; tolerated) |
| `hud: {collapsed, cinematic}` | R13 | HUD refresh |
| `ult_required: i32` | R12 | ult points |
| `projectiles: channel -> ProjectileSnapshot` | R14 for watch-list channels | Cloudburst |

Every field is `Result<T, OracleError>` carrying the error text, so the tick applies section 6 exactly. Field
failures are independent. That matches Python, where each feature has its own try. Exception: in Python one failure
inside `read_slot_machines` fails **all** slots of that call, so snapshot the slot list as one unit.

### 7.3 On-demand queries

| query | reply | when the tick sends it |
|---|---|---|
| `HandlerKeys` | `{address, send_key, receive_key}` | at attach. The oracle retries itself about every 1 s until success. Datagrams that arrive while it is in flight are held, not dropped (§4.1) |
| `InputBounds` | `Option<{cursor, count, latest}>` or error | on every authenticated ch8 datagram that decodes to exactly one input stream (gates §4.11). The input ACK for that datagram waits for the reply |
| `ResolveRpc{thunk_rva: 0x4B6B340, kind: Controller}` | `Option<{field_index, class_max}>` | when the HUD-set stage becomes eligible. The stage waits for the reply |
| `TagNetIndices` | `TagNetIndex` | Python reads at the i = 2 stage (>= 1.0 s after i = 0: stages are >= 0.5 s apart, H:4007; read at H:4049-4059) and observed `constructed = true` there. An earlier read may find the index not yet built (`constructed = false`) and return the *predicted* order, which has never been checked against the client. Either query at the i = 2 stage (the stage waits about 150 ms) or query early and re-query at i = 2 if `constructed` was false |
| `StateNames{sm}` | ordered names | implicit in the slot snapshot (cache) |
| `ItemMachines{item}` | `Vec<Machine>` | registration (plan_extra_machines) |
| `PacketNotify` | counters | optional, debug only |

### 7.4 Backend trait

```rust
pub trait MemoryBackend: Send {
    fn read(&self, addr: u64, buf: &mut [u8]) -> Result<(), ReadError>;   // exact length, §2.3 bounds
    fn regions(&self, from: u64) -> Option<Region>;                         // VirtualQueryEx (key scan)
    fn alive(&self) -> bool;                                                // WaitForSingleObject(h, 0) == 0x102
}
```

`RpmBackend` now. `PipeBackend` later: the in-process `internal` DLL answers the same snapshot and query schema over a
named pipe. Offsets and decoders stay identical. In-process reads are as non-atomic as RPM (multi-field reads such as the
6-double camera or the `<ii` resource pair can tear between client frames, as in Python).

### 7.5 Console logging (oracle part)

The server allocates a console (DESIGN.md) and logs to it and to `logs/aries_server_<pid>.log`.
- info: `oracle attach pid=… image_base=0x…`; `oracle fname key validated`; `oracle handler 0x… recv_fp=<12 hex>
  scan_ms=… scanned_mb=…`, keys never printed; `oracle reader ready`; `oracle hud_rpc field=52 max=178`;
  `oracle tag_index n=… bits=… first_segment=… constructed=… ms=…`; `oracle client exited`.
- warn: the first occurrence of each Python error key from section 6, then at most one line per key per 5 s with a count.
  Also a snapshot older than 2 ticks, and key-scan attempts still failing after 10 s.
- debug: per-snapshot `rpm_calls`, `read_us`, and watch-list changes.
- trace: the full snapshot.
- Keep the JSON-lines event sink with the Python key names (05_authority_loop.md §8) so logs diff against Python.

---

## 8. Offset table (every offset, with source)

Prod = read on the production path. "unused" = read in production but the value is not used.

| # | base | offset | type | meaning | source | prod |
|---|---|---|---|---|---|---|
| 1 | image | 0xD3E7140 | 0x80 bytes | GObjects owner block | RWS:10; RRI:60; RES:31,49; CIE:24; RMN:53; FNP:44; GTN:43 | yes |
| 2 | owner | 0x78 | u32 | seed | RWS:39 | yes |
| 3 | owner | 0x40+8*slot | u64 | encoded pointer | RWS:44 | yes |
| 4 | gobjects | 0x00 / 0x10 / 0x14 / 0x18 / 0x1C | u64 / u32 ×4 | chunks, MaxElements, NumElements, MaxChunks, NumChunks | RWS:127-129; RRI:63; FNP:47; RES:34 | yes |
| 5 | chunk table | 8*i | u64 | chunk i | RRI:66; RES:37; RMN:59 | yes |
| 6 | chunk | 24*(index & 0xFFFF) | u64 | UObject* (FUObjectItem stride 24) | RWS:85-89; FNP:51-52 | yes |
| 7 | GObjects | index 0x107E2 | object | GEngine (AresGameEngine) | RRI:67; RES:38; FNP:21; RMN:60; CIE:32 | yes |
| 8 | GObjects | index 0xEA40..0xFB00 | scan | tags manager/settings search | GTN:48 | yes |
| 9 | UObject | 0x0C | u32 | object index (engine check FNP:53; RRI:78 reads the actor's own index, unused) | FNP:53; RRI:78 | yes |
| 10 | UObject | 0x10 | u64 | class | RES:149,262; RRI:77; RP:333; GTN:54 | yes |
| 11 | UObject | 0x18 | u32 | name ComparisonIndex | FNP:55; RES:86,151,191,228,263; RP:333; GTN:53 | yes |
| 12 | image | 0xD548540 | pool | FName pool; block table at +0x10+8*block | FNP:20,30 | yes |
| 13 | name block | 4*(idx & 0xFFFF) +4 / +6 | u16 / bytes | header (len<<1 \| wide) / encrypted chars | FNP:29-39 | yes |
| 14 | image | 0xA5F3BB0 | value | Ares handler vtable (needle, not read) | H:1472; ILPN:16 | yes |
| 15 | handler | 0x28 / 0x48 | 32 bytes each | client send key / receive key (server frames) | CLK:75; H:1474 | yes |
| 16 | image | 0x4B6B340 | value | ClientSetHUD ExecFunction (compared) | H:3784 | yes |
| 17 | image | 0x6244D00 | value | ClientGamePhaseBegin ExecFunction | H:3833 | no |
| 18 | engine | 0xBD0 | u64 | GameViewport | RES:39; RRI:68; RMN:61 | yes |
| 19 | viewport | 0x80 | u64 | World | RES:40; RRI:68 | yes |
| 20 | world | 0x40 | u64 | NetDriver | RRI:69; RP:319; RWS:146 | yes |
| 21 | world | 0x1D8 | u64 | OwningGameInstance | RES:41; RRI:70 | yes |
| 22 | gameinstance | 0x40 | u64 | LocalPlayers data ([0] used) | RES:42; RRI:71 | yes |
| 23 | localplayer | 0x38 | u64 | PlayerController | RES:43; RRI:72 | yes |
| 24 | controller | 0x4B0 | u64 | Pawn | RES:44; RRI:73; RMN:18,66; H:2001 | yes |
| 25 | controller | 0x528 | u64 | PlayerCameraManager | RES:70 | yes |
| 26 | controller | 0x690 | u32 | flags, bit 1 (&2) = cinematic mode | H:1989, 2004 | yes |
| 27 | controller | 1296 (0x510), 1304 (0x518) | u64 | gate flags / pawn gate | RMN:74-75 | unused |
| 28 | cameramanager | 0x17C0 | 6×f64 | POV location xyz, rotation pitch/yaw/roll | RES:67-68,73 | yes |
| 29 | pawn | 0xBF8 | u64 | Inventory | RES:105,137,253 | yes |
| 30 | pawn | 0xB90 | u64 | input state component | RES:252; CIE:39 | yes |
| 31 | pawn | 0xB80 | u64 | Widget_BottomHUD | H:1985,2002 | yes |
| 32 | pawn | 0x14F4 | i32 | ult points required | H:1826,1837 | yes |
| 33 | pawn | 0x4F8 | u64 | CharacterMovement | RMN:19,76 | yes |
| 34 | pawn | 0x224 / 0x128 | u8 | Role / RemoteRole | RMN:69-70 | unused |
| 35 | pawn | 0x4A0 / 0x510 / 0x518 | u64 | controller gate / alt gates | RMN:71-73 | unused |
| 36 | widget | 0x11C | u8 | visibility, 1 = Collapsed | H:2003 | yes |
| 37 | inventory | 0x278 | u64 | CurrentEquippable | RES:106,140,259 | yes |
| 38 | inventory | 0x260 | u64 | equippable being switched to (desired) | RES:256-259 | yes |
| 39 | inventory | 0x1B8+8*slot | u64 | ItemSlots[slot] (UItemSlot*) | RES:143 | yes |
| 40 | inventory | 0x2F0 | u32 | CorrectionIndex | RES:267 | yes |
| 41 | itemslot | 0x38 | u64 | Contents | RES:134,144 | yes |
| 42 | item | 0xE00 | u64 | EquippableStateMachine | RES:107,153 | yes |
| 43 | item | 0xE30 / 0xE38 | u64 / u32 | machine array data / count (1..8) | RES:180-182 | yes |
| 44 | item | 0x1208 / 0x1200 | u64 | MagazineAmmo / ReserveAmmo component | RE:324-327; range_gun_loader.py:238-239 | yes |
| 45 | item | 0x1240 / 0x12A0 / 0x1290 | u64 | C EquipmentCharge / Q EquipmentCharge / E SignatureAbilityResource | RE:272-274 | yes |
| 46 | resource | 0x10C / 0x110 | i32 | PredictedResourceAmount / AuthResourceAmount | RES:207-208,232 | yes |
| 47 | resource | 0x12C / 0x188 | i32 | default seed: Ammo / MaxCharges | RES:209,233-234; RE:36,268 | yes |
| 48 | sm | 0x160 / 0x168 | u64 / u32 | States data (24-byte elements, first u64 = state) / count (1..64) | RES:5-6,78-84 | yes |
| 49 | sm | 0x1B0 | u64 | ClientPredictedCurrentState.State | RES:6,116,161 | yes |
| 50 | sm | 0x1C8 | u64 | AuthCurrentState.State | RES:7,117,162 | yes |
| 51 | sm | 0x1E8 | u64 | starting state | RES:200 | yes |
| 52 | state | 0x18 | u32 | name | RES:86 | yes |
| 53 | FiringState | 0x4A0 | u64 | RNG component | H:947,958 | yes |
| 54 | rng | 0xEC | u32 | networked random seed | H:947,962 | yes |
| 55 | input | 0x12C | u8 | replication stopped | RES:248,265; CIE:53 | yes |
| 56 | input | 0x130 | u32 | correction index recorded when stopped | RES:248,266 | yes |
| 57 | input | 0xA84 | u32 | event cursor | CIE:54 | yes |
| 58 | input | 0x1198 | u32 | pending event count (<= 4096) | CIE:55,59 | yes |
| 59 | input | 0x1190 | u64 | event data | CIE:80 | no (CLI) |
| 60 | movement | 0x0 -> vt+2944 (0xB80), vt+2920 (0xB68) | u64 | vtable slots (RVA logging) | RMN:77-79 | unused |
| 61 | movement | 0xC1 | u8 | bit 0x20 move-handler gate | RMN:114,153 | unused |
| 62 | movement | 0x13D8 | u8 | dropping moves | RMN:115 | unused |
| 63 | movement | 0x7E8 | u64 | client prediction data | RMN:20,116 | yes |
| 64 | pd | 0x20 | u64 | saved-move array (16-byte entries, first u64 = FSavedMove*) | RMN:21,121,134 | yes |
| 65 | pd | 0x28 | u32 | saved-move count | RMN:22,119 | yes |
| 66 | pd | 0x50 | u64 | last acked FSavedMove* | RMN:23,122 | yes |
| 67 | pd | 0x68 | u32 | overflow comparison value | RMN:120 | unused |
| 68 | savedmove | 0x1C | u32 | move number (ACK key) | RMN:24,123,140 | yes |
| 69 | savedmove | 0x3C | u32 (f32 bits) | timestamp (event only) | RMN:28,142 | event |
| 70 | savedmove | 0x0 -> vt+16 | u64 | vtable / prepare slot | RMN:137-139 | unused |
| 71 | savedmove | 1080 (0x438) / 592 (0x250) | 3×f64 | move direction / saved location | RMN:144,146 | unused (follow/track OFF) |
| 72 | movement | 5104, 5304, 5312, 5316, 5424, 416, 424; manager 1224-2464; world+0x178 -> +0xCE8 -> +0xD8 | mixed | fixed-step diagnostics | RMN:85-113 | no (`__main__`) |
| 73 | netdriver | 0x198 | u64 | FClassNetCacheMgr | RRI:20,80 | yes |
| 74 | cachemgr | 0x08 / 0x10 | u64 / u32 | TSet data / count (<= 0x10000) | RRI:81-83 | yes |
| 75 | set entry | stride 24: +0 / +8 | u64 / u64 | key (low32 = class index) / FClassNetCache* | RRI:112-116 | yes |
| 76 | netcache | 0x00 / 0x08 / 0x20 / 0x28 | u32 / u64 / u64 / u32 | base / parent / fields data / count (<= 4096) | RRI:22-24,92-95,105 | yes |
| 77 | field entry | stride 24: +0 / +8 | u64 / u32 | tagged UField* (LSB 1 = UFunction) / field index | RRI:25,99-104 | yes |
| 78 | UFunction | 0xE0 | u64 | ExecFunction | RRI:21,103 | yes |
| 79 | class | 0x0C | u32 | class object index (set key match) | RRI:110 | yes |
| 80 | netdriver | 0xC0 | u64 | ServerConnection | RP:320 | yes |
| 81 | connection | 0x13F8 / 0x1400 | u64 / u32 | Channels data / count | RP:323 | yes |
| 82 | actor channel | 0x78 | u64 | Actor | RP:310,327 | yes |
| 83 | actor | 0x290 | u64 | RootComponent | RP:335 | yes |
| 84 | root | 0x320 | 3×f64 | location (projectile) | RP:336 | yes |
| 85 | actor | 0x610 | u64 | FiniteSpeedMovement | RP:310,337 | yes |
| 86 | fsm | 0x198 | u32 | NumCollisions | RP:311,340 | yes |
| 87 | fsm | 0x228 / 0x258 | 3×f64 | last hit location / normal | RP:311,341-343 | yes |
| 88 | tagsmgr | 0x130 / 0x138 / 0x13C | u64 / u32 / u32 | GameplayTagNodeMap data / count / capacity | GTN:23,70-72 | yes |
| 89 | nodemap entry | stride 0x28: +0 / +4 / +0x10 | u32 / u32 / u64 | comparison / number / node | GTN:73-77 | yes |
| 90 | tagsmgr | 0x180 / 0x188 | u64 / u32 | CommonlyReplicatedTags data / count; stride 12, comparison at +0 | GTN:24,89-92 | yes |
| 91 | tagsmgr | 0x28C | n/a | declared, never read | GTN:25 | no |
| 92 | tagsettings | 0xBC | i32 | NetIndexFirstBitSegment | GTN:26,98 | yes |
| 93 | tag node | 0x50 | u16 | NetIndex (0xFFFF = not built) | GTN:100,104 | yes |
| 94 | handler | 0x08 | u64 | packet handler | ILPN:32 | no (R2) |
| 95 | packet handler | 448 (0x1C0) | u64 + i32 | components data / count (2..8) | ILPN:33-36 | no (R2) |
| 96 | components | 16 | u64 | stateless component | ILPN:37 | no (R2) |
| 97 | stateless | 0x38 | u64 | NetDriver | ILPN:38 | no (R2) |
| 98 | netdriver | 192 (0xC0) / 376 (0x178) / 608 (0x260) | u64 | connection / pending object / control handler | ILPN:39,88,92 | no (R2) |
| 99 | connection | 0x70 / 168 / 316 / 476 | u64 / u32 / u32 / u8 | package map / max packet / state / challenge state | ILPN:40,104,122-123 | no (R2) |
| 100 | connection | 5144 (0x1418) | u64 | incoming channel sequence array (i32: [0] control, [3..8]) | ILPN:46-52 | no (R2, `pre_control_sequence`) |
| 101 | connection | 5112 (0x13F8) / 5120 | u64 / i32 | channels / count (1..65536) | ILPN:53-55 | no (R2) |
| 102 | connection | 6840 (0x1AB8) + 588 | 12 bytes | u16 in_seq, +2 last_recv_to_ack, +6 out_seq, +8 out_acked (each & 0x3FFF) | ILPN:95-96,124-127 | no (R2, `pre_in_sequence`) |
| 103 | packagemap | 792 -> +336 (3 bytes); vt+752, vt+776 | mixed | cache flags, serialize methods | ILPN:41-45 | no (R2) |
| 104 | channel | 56 / 60 / vt+896 / actor vt+1192 | u32 / u32 / u64 / u64 | flags / id / methods | ILPN:71-87 | no (R2) |
| 105 | pending object | 472 -> vt+976; control handler vt+24 | u64 | callbacks | ILPN:89-94 | no (R2) |
| 106 | image | 0xD414578 / 0xD39B290 | u8 | log verbosities | ILPN:118-119 | no (R2) |
| 107 | gameinstance | 0x48 | u32 | LocalPlayers count (== 1) | RRPS:69 | no |
| 108 | world | 0x178 | u64 | GameState; +0x488, +0x7FC (4×f32), +0x819, +0xCD0, +0xF38, +0xF88, +0x728/+0x720/+0x730/+0x768/+0x708, +0xC88, +0x1080, +0x1088, +0x10D8 | RRPS:77-110 | no |
| 109 | team state | 0x4B0 (2 bytes), 0x478; team +0xE8, vt+1256 | mixed | roles, components | RRPS:100-115 | no |
| 110 | controller | 0x6A8, 0x4A0, 0x518, 0x5E8, 0x520, 0xC21, 0x470, 0x1120, 0x1128, 0x1129, 0x1170; vt+2736/4440/4432/… | mixed | waiting flag, state, HUD, PlayerState, Range controller fields | RRPS:116-178 | no |
| 111 | playerstate | 0xC00 (16), 0xA20, 0xBD0, 0xBD8, 0x6A8, 0x758, 0x8A0; playerinfo 0x4E8, 0x550; team comp 0xC/0x18/0x28/0xE8, vt+1288/1240 | mixed | PlayerState links | RRPS:136-166 | no |
| 112 | pawn / root / movement | pawn 0x290 root, root 0x170 location, movement 0x271 mode, pawn 0x488/0x4A0, input 0x260, serializer input+0xD8 vt+32, inventory 0x2E8/0x305, slot 0x30/0x40, inventory vt+1248, equippable 0x224/0x210/0x4A8/0x4B0, sm 0x348/0x350, ammo comp +0x12C pair | mixed | diagnostics | RRPS:182-246 | no |

Note on rows 84 and 112: the projectile location is read at `root+0x320` (production). The diagnostic pawn location
is read at `root+0x170`. These are different fields. Do not swap them.

---

## 9. Needed vs replaceable

| read | keep in Rust? | alternative | parity impact of the alternative |
|---|---|---|---|
| R1 keys | yes (scan or internal DLL) | pin the 32+32 bytes in config (receive fingerprint constant `107b74a379f0`) | none if the keys really are constant. Needs the user's decision (key bytes are never stored) |
| R2 notify | **no** | none | logs only. Removes a 0.05-9.8 s pre-challenge stall |
| R3 reader/key | yes | none | n/a |
| R4/R5/R6 state machines | yes | per-class pinned name lists (static content) | GUID order must equal the live TMap order. Pinning needs one capture per class |
| R7 ammo/charges | yes | server-side ammo simulation from fire/reload inputs | behavior change |
| R8 firing seed | yes for parity | count fire activation events from the decoded input stream (`received_activation_events`, H:1533-1539) | shot timing/count changes |
| R9 camera | yes for parity | view rotation/location from ServerMovePacked (06_client_decode.md) | precision changes |
| R10 input replication | yes | infer "stopped" from missing input streams | behavior change |
| R11 input bounds | yes | stricter stream validation | changes which junk streams are rejected |
| R12 ult | optional | constant 8 for Wushu (observed 15/15) | none for Wushu |
| R13 HUD | yes | timer heuristic | behavior change |
| R14 projectile | yes | server-side projectile simulation | behavior change |
| R15 HUD RPC | optional | constants field 52, max 178 (observed 15/15) with a startup self-check read | none |
| R16 tag indices | yes until captured | pin the six indices + width 10 after one live capture | none if pinned values match |
| R17 saved moves | yes for parity | newest move number from the client's ServerMovePacked (04_bringup_b §10.4 Q4) | ACKs the newest *received* move instead of the newest *saved* one |
| R18 liveness | yes | none | n/a |

---

## 10. Test vectors

Generated offline by running the real Python readers against a synthetic process image. A scratch harness replaced
`ctypes.windll.kernel32.OpenProcess/ReadProcessMemory/CloseHandle` and `check_live_packet_keys.kernel32` before importing
the readers, so no real process was touched. RPM served 4 KiB-granular sparse pages (an unmapped page -> RPM returns 0 ->
`read_bytes` raises OSError). `VirtualQueryEx` reported each contiguous run of mapped pages as one region with
`MEM_COMMIT`, `MEM_PRIVATE`, `PAGE_READWRITE` (0x04), and gaps as `MEM_FREE` (0x10000). Python 3.12.3.

### 10.1 `decode_array_pointer` (pure)

Input: a 0x80-byte owner, all zero except u32 `seed` at 0x78 and u64 `encoded` at `0x40 + 8*slot`.

| slot | seed | mixed_seed | mixed | high | encoded | decoded |
|---|---|---|---|---|---|---|
| 0 | 0x13579be1 | 0xd156ae9a | 0xc2f8727cba84b972 | 0xc2f8727c | 0x0123456789abcdef | 0x352dd602282882bc |
| 1 | 0x13579bec | 0xcb56ae97 | 0xeb4a079a1e3e221b | 0xeb4a079a | 0x1032547698badcfe | 0x2031a8ba4de7717d |
| 2 | 0x13579be2 | 0xd756ae99 | 0x058f0b1a1917dc55 | 0x058f0b1a | 0x23016745ab89efcd | 0xcd23016756fc120d |
| 3 | 0x13579bdf | 0xad56aea4 | 0xc888b111c0c55c94 | 0xc888b111 | 0x32107654ba98fedc | 0x86420eca84048404 |
| 4 | 0x13579be5 | 0xd956ae9e | 0xe28babaae0382de6 | 0xe28babaa | 0x45670123cdef89ab | 0x456701233c33b9fc |
| 5 | 0x13579beb | 0xc556ae90 | 0x7e83ca7244441650 | 0x7e83ca72 | 0x54761032dcfe98ba | 0x5746dfce7ed6ca16 |
| 6 | 0x13579be3 | 0xd556ae98 | 0xbdaa3cce8faaff38 | 0xbdaa3cce | 0x67452301efcdab89 | 0x19b535164e4ee4d8 |

### 10.2 FName (pure given entries)

Key `5ac3179e`.
- `AresGameEngine`: header `0x001c` (len 14, narrow). Encrypted `15bf7ce313ac74f511a37ef93aa8`. Derived key `5ac3179e`.
- Narrow `FiringState` (len 11): encrypted `17a16efc3faf4fe130bc79`.
- Wide `Wüshu™` (len 6, header `0x000d`): encrypted `0bc5ed982fc5799829c533b9` (12 bytes, XOR on byte index), decoded
  `"W\u00fcshu\u2122"`.
- Any entry with `len >= 256` -> error (`bytes must be in range(0, 256)`).

### 10.3 Synthetic image and expected results

Image base `0x7FF6A0000000`. Owner seed `0x2468ace5` (slot 4, high `0xe1cc23fa`, encoded `0x165a488e7`) decodes to
GObjects `0x1A0000000`. NumElements 0x20000, NumChunks 2. Name ids: AresGameEngine 1, BasePistol_C 6,
EquippableStateMachine 11, InactiveState 18, EquipState 23, IdleState 27, FiringState 31, ReloadState 36,
ZoomStateMachine 41, ZoomStateInactive 47, ZoomStateUnzoomed 53, ZoomStateLevel1 59, MagazineAmmo 65, ReserveAmmo 70,
Projectile_Wushu_4_Smoke_C 75, Default__GameplayTagsSettings 83, GameplayTagsSettings 92, GameplayTagsManager 99,
the tags 106/116/127/138/148/159/172/177/181 (order in Appendix A), wide name 187.

Expected (pid ignored, `image_base = 0x7FF6A0000000`):

```
derive_name_key            = 5ac3179e
find_world                 = 0x200020000
find_controller            = 0x200070000
find_pawn                  = 0x200080000
read_camera                = ([3581.5, -12019.25, 1360.0], [-4.5, 92.25, 0.0])
read_slot_machines([1,2])  = {1: item 0x200200000, class "BasePistol_C", sm 0x200220000,
                              predicted "FiringState", authoritative "IdleState", is_current true}   // slot 2 empty
state name order (sm 0x200220000) = [InactiveState, EquipState, IdleState, FiringState, ReloadState]
read_item_machines(0x200200000) =
   [EquippableStateMachine @0x200220000 pred FiringState auth IdleState start InactiveState
       names [InactiveState, EquipState, IdleState, FiringState, ReloadState],
    ZoomStateMachine @0x200240000 pred ZoomStateUnzoomed auth ZoomStateInactive start ZoomStateInactive
       names [ZoomStateInactive, ZoomStateUnzoomed, ZoomStateLevel1]]
read_resource_amounts(pistol, BasePistol resources) = {MagazineAmmo: (11, 12, 12), ReserveAmmo: (36, 36, 36)}
read_input_replication     = stopped true, requested 11, inventory 9, class "BasePistol_C"
read_pending_input_bounds  = component 0x2000b0000, stopped 1, cursor 1000, count 7, latest 1007
read_saved_moves           = count 3, oldest 201, newest 203, last_acked 200, numbers [201,202,203],
                             newest_timestamp_bits 1080033280 (3.5), move_handler_gate_open true, dropping false
resolve(thunk 0x7FF6A4B6B340, controller) = field_index 52, class_max_index 178
read_projectile(ch 10000)  = class "Projectile_Wushu_4_Smoke_C", location (3700, -11900, 1300),
                             collisions 1, hit (3710, -11890, 1253.8), normal (0, 0, 1)
read_firing_seed           = 0xc0ffee11
hud_refresh                = collapsed true, cinematic false
ult_required               = 8
predict_net_indices (node+0x50 = 0xFFFF everywhere):
   indices {Ability.Cooldown:0, Ability:1, aBility.Zeta:2, AbilityTuning.Wushu.4.SmokeDuration:3,
            AbilityTuning.Wushu.4.SmokeMaxCharges:4, AbilityTuning.Wushu.E.DashVelocityMultiplier:5,
            AbilityTuning.Wushu.E.MaxCharges:6, AbilityTuning.Wushu.E.PrimeDuration:7,
            AbilityTuning.Wushu.Q.MaxCharges:8}, true_bit_num 4, first_bit_segment 4, constructed false
predict_net_indices (then u16 [0x200560000 + 0x100*i + 0x50] = 100 + i for tag i in Appendix A order):
   indices {Ability:107, Ability.Cooldown:108, aBility.Zeta:106, …4.SmokeDuration:101, …4.SmokeMaxCharges:104,
            …E.DashVelocityMultiplier:105, …E.MaxCharges:103, …E.PrimeDuration:102, …Q.MaxCharges:100},
   true_bit_num 4, first_bit_segment 4, constructed true
read_current_handler_keys(needle 0x7FF6AA5F3BB0) = handler 0x300100040,
   send_key 101112…2f (bytes 0x10..0x2f), receive_key a0a1…bf (bytes 0xa0..0xbf);
   the decoy needle at 0x300000ff8 is skipped (112-byte re-read crosses into an unmapped page)
```

What the vectors exercise: the case-insensitive sort (`aBility.Zeta` between `Ability` and `AbilityTuning…`); the
common-tag swap (`Ability.Cooldown` moved to 0); a null node-map slot (k = 4) skipped; the child -> parent net-cache
walk (child base 150 count 28, parent base 0 count 150, match in the parent at entry 52, max 178); an untagged entry
skipped; first valid hit wins in the key scan.

### Appendix A. Synthetic image write log

Format: `M addr size` maps zeroed memory. `W addr hex` writes bytes. Map every touched 4 KiB page. Apply the lines in
order. Tag order for the constructed vector (`i` = node index, node `0x200560000 + 0x100*i`):
`0 AbilityTuning.Wushu.Q.MaxCharges, 1 …4.SmokeDuration, 2 …E.PrimeDuration, 3 …E.MaxCharges, 4 …4.SmokeMaxCharges,
5 …E.DashVelocityMultiplier, 6 aBility.Zeta, 7 Ability, 8 Ability.Cooldown`.

```
W 0x7ff6ad3e7140 000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000e788a4650100000000000000000000000000000000000000e5ac682400000000
W 0x1a0000000 001000a001000000000000000000000000002000000002002000000002000000
W 0x1a0001000 000000b001000000
W 0x1a0001008 000000c001000000
M 0x1b0000000 0x180000
M 0x1c0000000 0xc000
W 0x7ff6ad548550 000000d001000000
W 0x1d0000008 1c00
W 0x1d000000a 15bf7ce313ac74f511a37ef93aa8
W 0x1c000bd30 0000000002000000
M 0x200000000 0x1000
W 0x20000000c e2070100
W 0x200000018 01000000
W 0x200000bd0 0000010002000000
W 0x200010080 0000020002000000
W 0x200020040 0000030002000000
W 0x2000201d8 0000040002000000
W 0x200040040 0000050002000000
W 0x200040048 01000000
W 0x200050000 0000060002000000
W 0x200060038 0000070002000000
W 0x2000704b0 0000080002000000
W 0x200070528 0000090002000000
W 0x200070690 01000000
W 0x2000917c0 0000000000fbab4000000000a079c7c0000000000040954000000000000012c000000000001057400000000000000000
W 0x200080bf8 00000a0002000000
W 0x200080b90 00000b0002000000
W 0x200080b80 00000c0002000000
W 0x2000814f4 08000000
W 0x2000c011c 01
W 0x2000b012c 01
W 0x2000b0130 0b000000
W 0x2000b0a84 e8030000
W 0x2000b1198 07000000
W 0x2000a02f0 09000000
W 0x2000a0278 0000200002000000
W 0x2000a0260 0000000000000000
W 0x2000a01c0 00002a0002000000
W 0x2002a0038 0000200002000000
W 0x200200010 0000210002000000
W 0x1d000001c 1800
W 0x1d000001e 14ae68f706a668e639a344d1
W 0x200210018 06000000
W 0x200200e00 0000220002000000
W 0x1d0000030 2c00
W 0x1d0000032 09a474e13ca560ea20b052fc2da164c52db669e122b0
W 0x200220018 0b000000
W 0x200220160 0000230002000000
W 0x200220168 05000000
W 0x200230000 00002b0002000000
W 0x1d000004c 1a00
W 0x1d000004e 1ea07bf023a76cf604ba7be732
W 0x2002b0018 12000000
W 0x200230018 00102b0002000000
W 0x1d0000060 1400
W 0x1d0000062 15b868fd209a69f524ac
W 0x2002b1018 17000000
W 0x200230030 00202b0002000000
W 0x1d0000070 1200
W 0x1d0000072 1aae72f200be7fe336
W 0x2002b2018 1b000000
W 0x200230048 00302b0002000000
W 0x1d0000080 1600
W 0x1d0000082 17a16efc3faf4fe130bc79
W 0x2002b3018 1f000000
W 0x200230060 00402b0002000000
W 0x1d0000094 1600
W 0x1d0000096 03ad70fa30ac4fe130bc79
W 0x2002b4018 24000000
W 0x2002201b0 00302b0002000000
W 0x2002201c8 00202b0002000000
W 0x2002201e8 00002b0002000000
W 0x2002b34a0 0000290002000000
W 0x2002900ec 11eeffc0
W 0x1d00000a8 2000
W 0x1d00000aa 10bc68e319a766fa2f9e66ed22ba69eb
W 0x200240018 29000000
W 0x200240160 0000250002000000
W 0x200240168 03000000
W 0x200250000 00002c0002000000
W 0x1d00000c0 2200
W 0x1d00000c2 11bd69e218a667fb2e9b68ee28a66ff92e
W 0x2002c0018 2f000000
W 0x200250018 00102c0002000000
W 0x1d00000d8 2200
W 0x1d00000da 11bd69e218a667fb2e8768f524bd6bea2f
W 0x2002c1018 35000000
W 0x200250030 00202c0002000000
W 0x1d00000f0 1e00
W 0x1d00000f2 0fa377fc06b879e530807de730a029
W 0x2002c2018 3b000000
W 0x2002401b0 00102c0002000000
W 0x2002401c8 00002c0002000000
W 0x2002401e8 00002c0002000000
W 0x200200e30 0000260002000000
W 0x200200e38 02000000
W 0x200260000 0000220002000000
W 0x200260008 0000240002000000
W 0x200201208 0000270002000000
W 0x200201200 0000280002000000
W 0x1d0000108 1800
W 0x1d000010a 1bae7cf32ca675f717a276fd
W 0x200270018 41000000
W 0x20027010c 0b0000000c000000
W 0x20027012c 0c000000
W 0x1d000011c 1600
W 0x1d000011e 03ad6ff023be79d43ca573
W 0x200280018 46000000
W 0x20028010c 2400000024000000
W 0x20028012c 24000000
W 0x2000804f8 00000d0002000000
W 0x200080224 02
W 0x200080128 03
W 0x2000804a0 0000070002000000
W 0x200080510 0000000000000000
W 0x200080518 0000000000000000
W 0x200070518 0000080002000000
W 0x200070510 4000000000000000
W 0x2000d0000 00000e0002000000
W 0x2000e0b80 b0da47a7f67f0000
W 0x2000e0b68 00d047a7f67f0000
W 0x2000d00c1 20
W 0x2000d13d8 00
W 0x2000d07e8 00000f0002000000
W 0x2000f0020 0000100002000000
W 0x2000f0028 03000000
W 0x2000f0068 60000000
W 0x200100000 0000110002000000
W 0x200110000 0000180002000000
W 0x20011001c c9000000
W 0x20011003c 0000c03f
W 0x200110438 0000000000000000000000000000f03f0000000000000000
W 0x200110250 54e3a59b44fbab40a8c64b37a979c7c0273108ac9c389440
W 0x200100010 0010110002000000
W 0x200111000 0000180002000000
W 0x20011101c ca000000
W 0x20011103c 00002040
W 0x200111438 0000000000000000000000000000f03f0000000000000000
W 0x200111250 54e3a59b44fbab40a8c64b372979c7c0273108ac9c389440
W 0x200100020 0020110002000000
W 0x200112000 0000180002000000
W 0x20011201c cb000000
W 0x20011203c 00006040
W 0x200112438 0000000000000000000000000000f03f0000000000000000
W 0x200112250 54e3a59b44fbab40a8c64b37a978c7c0273108ac9c389440
W 0x200180010 000047a7f67f0000
W 0x2000f0050 0000130002000000
W 0x200130000 0000180002000000
W 0x20013001c c8000000
W 0x200070010 0000300002000000
W 0x20030000c 21eb0300
W 0x200030198 0000310002000000
W 0x200310008 0000320002000000
W 0x200310010 03000000
W 0x200320000 4523010005000000
W 0x200320008 00003f0002000000
W 0x200320018 21eb030007000000
W 0x200320020 0000330002000000
W 0x200320030 0100000009000000
W 0x200320038 00103f0002000000
W 0x200330000 96000000
W 0x200330008 0000340002000000
W 0x200330020 0000350002000000
W 0x200330028 1c000000
M 0x200350000 0x2a0
W 0x200350048 0100380002000000
W 0x200350050 99000000
W 0x200340000 00000000
W 0x200340008 0000000000000000
W 0x200340020 0000360002000000
W 0x200340028 96000000
M 0x200360000 0xe10
W 0x2003604e0 0100370002000000
W 0x2003604e8 34000000
W 0x2003604f8 00003a0002000000
W 0x2003700e0 40b3b6a4f67f0000
W 0x2003800e0 101111a1f67f0000
W 0x2000300c0 0000400002000000
W 0x2004013f8 0000410002000000
W 0x200401400 00280000
M 0x200410000 0x14000
W 0x200423880 0000430002000000
W 0x200430078 0000440002000000
W 0x200440010 0000450002000000
W 0x1d0000130 3400
W 0x1d0000132 10ab62ee25ba79ed2cbc52d335aa65f11fed52d72db666e11f9a
W 0x200450018 4b000000
W 0x200440290 0000460002000000
W 0x200460320 0000000000e8ac4000000000003ec7c00000000000509440
W 0x200440610 0000470002000000
W 0x200470198 01000000
W 0x200470228 0000000000fcac40000000000039c7c03333333333979340
W 0x200470258 00000000000000000000000000000000000000000000f03f
W 0x1b0160800 0000520002000000
W 0x1b0162000 0000500002000000
W 0x1d0000150 3a00
W 0x1d0000152 03bb6ce232b27edc18996bee22ae66e23e8a6be4348d6ff733b764e434
W 0x200520018 53000000
W 0x200520010 0000530002000000
W 0x1d0000174 2800
W 0x1d0000176 09b66eef3ebb62f31ab664f91db277fe27b964f9
W 0x200530018 5c000000
W 0x2005200bc 10000000
W 0x1d0000190 2600
W 0x1d0000192 0eb169e839bc65f41db163fe04b16aec2eb576
W 0x200500018 63000000
W 0x200500010 0000510002000000
W 0x200510018 63000000
W 0x200500130 0000540002000000
W 0x200500138 09000000
W 0x20050013c 0a000000
M 0x200540000 0x190
W 0x1d00001ac 4000
W 0x1d00001ae 3b815ed213974eea0f8d5ed01dcd60cb098b42902bcd7adf02a05fdf088452cd
W 0x200540000 6a000000
W 0x200540004 00000000
W 0x200540010 0000560002000000
W 0x200560050 ffff
W 0x1d00001d4 4600
W 0x1d00001d6 38825dd110944de90c8e5dd31ece63c80a8841934dce67d0168b51f90c9255c9108f5a
W 0x200540028 74000000
W 0x20054002c 00000000
W 0x200540038 0001560002000000
W 0x200560150 ffff
W 0x1d0000200 4600
W 0x1d0000202 38825dd110944de90c8e5dd31ece63c80a8841933cce64cf108d51f90c9255c9108f5a
W 0x200540050 7f000000
W 0x200540054 00000000
W 0x200540060 0002560002000000
W 0x200560250 ffff
W 0x1d000022c 4000
W 0x1d000022e 3b815ed213974eea0f8d5ed01dcd60cb098b42903fcd7adf02a05fdf088452cd
W 0x200540078 8a000000
W 0x20054007c 00000000
W 0x200540088 0003560002000000
W 0x200560350 ffff
W 0x1d0000254 4a00
W 0x1d0000256 3e845bd716924bef0a885bd518c865ce0c8e47954bc861d6108d57f61e9e71d31e9455de0c
W 0x2005400c8 94000000
W 0x2005400cc 00000000
W 0x2005400d8 0004560002000000
W 0x200560450 ffff
W 0x1d0000280 5800
W 0x1d0000282 378d52de1f9b42e6038152dc11c16cc705874e9c33c17fd305876dd71a8058db029676c71a9b52c21a865ec0
W 0x2005400f0 9f000000
W 0x2005400f4 00000000
W 0x200540100 0005560002000000
W 0x200560550 ffff
W 0x1d00002b4 1800
W 0x1d00002b6 378d72fe3fbb62bc0caa6ff3
W 0x200540118 ac000000
W 0x20054011c 00000000
W 0x200540128 0006560002000000
W 0x200560650 ffff
W 0x1d00002c8 0e00
W 0x1d00002ca 1ca679f534b069
W 0x200540140 b1000000
W 0x200540144 00000000
W 0x200540150 0007560002000000
W 0x200560750 ffff
W 0x1d00002d8 2000
W 0x1d00002da 0bb16ee223a77ea009bc68e22ebc70e0
W 0x200540168 b5000000
W 0x20054016c 00000000
W 0x200540178 0008560002000000
W 0x200560850 ffff
W 0x200500180 0000550002000000
W 0x200500188 01000000
W 0x200550000 b5000000
W 0x1d00002f0 0d00
W 0x1d00002f2 0bc5ed982fc5799829c533b9
M 0x300000000 0x1000
W 0x300000ff8 b03b5faaf67f0000
M 0x300100000 0x2000
W 0x300100040 b03b5faaf67f0000
W 0x300100048 0000200003000000
W 0x300100068 101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f
W 0x300100088 a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebf
```

Production values that need oracle input on the wire (for the vectors of other specs): HUD RPC 52/178 (04_bringup_b §10.3),
tag width 10 with the six indices unknown (04_bringup_b §10.4 Q1), move numbers per session (03/04 docs).

---

## 11. Open questions

1. **Keys.** Pin them or keep scanning? The fingerprint is constant across all logged runs. Pinning needs the user's
   decision, because the key bytes have never been stored.
2. **R2 removal.** Dropping it makes the challenge reply 0.05-9.8 s faster (observed range). Confirm live that the
   client is fine with the immediate reply.
3. **R15 crash.** Python exits the responder when the ClientSetHUD lookup fails. Recommended: retry next tick (deliberate
   divergence).
4. **Empty state-name cache.** Python caches `{}` for a state machine whose TMap is not yet populated, and that machine
   then never mirrors. Keep this for parity, or do not cache empty results (a divergence only in that race)? A faster
   oracle that reads new items sooner than Python's 50 ms / 0.3 s polls makes the race more likely (§7.1). Not caching
   `{}` is the safer default. It was never observed in production.
5. **Address-keyed caches** (`_class_names`, `_names`, `firing_rng_components`) go stale if memory is reused.
   Possibly behind the 4 `purchase_register_gave_up` events (unverified; 60 `purchase_slotted` = 56 ready + 4 gave up).
   Parity says keep them. A safer option is to key by (address, object index at +0xC). In Python they live at most one
   3600 s responder run (§5). A long-lived Rust server should at least clear them per client session (on the
   connection's first ClientRestart) so they never span sessions.
6. **Tag indices** were never logged. One live capture (`predict_net_indices`, read-only) would let the six indices and
   width 10 be pinned. It needs the user's go-ahead to run against the live client.
7. **Unused reads in R17.** Python fails the move ACK if any diagnostic read fails. Rust reads only the needed fields;
   this diverges only on read failure.
8. **Narrow FName encoding.** Python decodes narrow names as UTF-8 strict, while UE narrow names are Latin-1. A name
   with a byte >= 0x80 fails in Python. None of the names used here are affected.

---

## Verifier corrections

An adversarial re-read of the Python sources (RWS, RRI, FNP, RES, CIE, RMN, CLK, ILPN, GTN, RP, RE, LR, H, launch.ps1)
and the two cited runtime logs made these changes. The logs were re-counted on 2026-10-14 17:20;
`responder_20928…out` is still being appended by a running responder.

Re-verified, no change needed: every test vector in §10. An independent harness stubbed `ctypes.windll.kernel32`
and `check_live_packet_keys.kernel32` before import, replayed Appendix A, and reproduced all 7
`decode_array_pointer` rows, the FName vectors and every §10.3 result, including the handler-key scan. The GObjects
decode, the FName pool and key derivation, the R3-R10/R12/R13 descriptions and offset-table rows 1-106 against their
source lines were also re-checked.

1. **R11 cannot come from a pre-receipt snapshot** (§1, §4.11, §7.1-7.3). Python reads the input bounds synchronously
   after the datagram arrives. A snapshot taken earlier can be below the packet's newest event ID, which rejects
   legitimate streams and can starve input ACKs. `input_bounds` was removed from the snapshot. An `InputBounds`
   on-demand query was added.
2. **R1 datagram handling** (§4.1, §7.3). "Drop datagrams as `not_ready` until the keys exist (parity)" was wrong.
   Python processes the datagram that triggered a successful scan, and the datagrams buffered during it. It drops only
   a datagram whose own scan raised. "Earlier scans fail" was marked unverified: Python never scans before the first
   datagram, and `not_ready` occurs 0 times in both logs.
3. **GUID allocation order at registration** (§4.4). Extra machines (machine GUID, then its states) are allocated
   **before** the main machine's states, and the main states exclude `InactiveState`. For abilities it is
   `load.inactive_guid`. For purchases it was allocated at stage 3.
4. **R16 retry semantics and gate** (§1, §4.16). The cache is filled before the segment check and the tag-name lookup,
   so those failures are never retried. Only open/read failures are. Added the missing `ability_activation` gate.
5. **R16 query timing** (§7.3). An early read (at i = 0) may see `constructed = false` and return the unverified
   predicted order. Read at i = 2 like Python, or re-read there.
6. **R14 null guards** (§4.14). `world == 0` or `driver == 0` returns None (RP:319-320). It does not raise, so there is
   no `cloudburst_read_error`.
7. **R15 pseudo-code** (§4.15, §8 row 9). Added `if !actor: None`, the unused `u32 [actor + 0xC]` read (RRI:78), the
   real read order (`cls_index` after the TSet check) and the missing TSet hole check. Added the keepalive restart
   consequence of the crash.
8. **R17 gate and precision** (§1, §4.17). Added the missing gate `source in client_restart_sent_sources`.
   `newest` is the last non-null of the first 256 slots. The timestamp is read only when slot `min(count,256)-1` is
   non-null.
9. **Process lifetime** (§2.4, §5, §4.15). Production runs `--seconds 3600 --keepalive`. Python's "process" caches
   reset every 3600 s, or on a crash, and every restarted run exits with `FileExistsError` at the output write
   (9 tracebacks). "The output JSON is still written" is true only for the first run per client pid.
10. **Stale observed numbers.** Input-ACK rejections: 43 -> 70 (29 distinct IDs). Sessions: 15 -> 16. R1: 5.6 s ->
    0.03-7.8 s. R2: ~4.8 s -> 0.05-9.8 s. R16: 156-157 ms per process (11 reads). R15 always < 25 ms.
11. **R6 wire effect** (§1, §4.6). Production extras are `ZoomStateMachine` (guns), `StateMachine_Priming` (Tailwind) and
    `BoostStateMachine` (Updraft), not only Zoom.
12. **Empty-TMap cache race** (§7.1, Q4). A 128 Hz watch list can read new items earlier than Python's polls do. Added
    watch-list guidance and a "do not cache `{}`" default.
13. **Cross-cutting state** (§5). Added the `Flight` R14 consumer state, `last_client_alive_check` and the
    `registered_tries` initial value. Q5 now recommends clearing address caches per client session in a long-lived server.
14. **Intra-datagram order** (§1). Added the order in which one datagram's handling consumes oracle values, which fixes
    the server-sequence order of the sends.
15. Minor: the ILPN handle was added to §2.2, and the keepalive `find_client` to R0.
