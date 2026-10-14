# Aries port plan v2 (decided 2026-10-14)

This revises `PLAN.md` (2026-10-13) after the overnight Range work and supersedes its architecture section. The decisions are in section 7.

## 1. Where the working system is today

The Python stack now runs a playable Range against the pinned 13.06 client:

| Feature | How |
|---|---|
| Spawn, movement, HUD (bottom HUD, Range HUD), escape menu | responder setup chain + backend |
| Weapons: fire, alt-fire, reload, ADS, sniper zoom levels | generic state-machine mirror + ammo resources |
| Buy menu: skinned guns, shields, replacing a gun | ServerPurchase decode, gun loader, channel close |
| Abilities: Updraft, Tailwind (tuning), Cloudburst/Blade Storm flows | ability activation, AbilityTuning in the open bunch |
| Practice content | level streaming, settings (infinite ammo) |
| Bots | dynamic TrainingBot actors, hits, death, respawn |

How it is built, and why a port is worth it:

| Part | Size | Problem |
|---|---|---|
| `range_udp_handshake.py` | 4.8k lines; 55 timed `elif` setup stages; 107 CLI flags | Hand-sequenced reliable channel sequences: one wrong offset buffers a channel forever. **No reliable retransmission.** State lives in dozens of dicts keyed by client address. |
| `range_server_bunch.py` and builders | 49 hand-written bunch builders | Every bit width is hand-coded; most past bugs were here. |
| Authority source | ReadProcessMemory of the client (predicted states, ammo, RNG seed, camera) | Works, but it is an external memory-reading hack with hardcoded offsets, and it is slow to extend. |
| `engine_bridge.cpp` | file-queue command bridge, 52 commands, versions v65–v75 | Each change is a new DLL version; game-thread commands can stutter the game. |
| `backend.py` | about 53 HTTP routes, RMS, admin API | Fine, but Python and single-file. |
| Launch | `launch.ps1` plus 26 PowerShell scripts | Hard to manage; the backend must be started separately. |
| Tests | 269 unit tests, 1 captured packet fixture | Good builder coverage, very little real-traffic coverage. |

## 2. The key decision: what the Rust server is

**Option A: network game server (recommended).** The client stays a normal network client. The Rust server speaks the UE 5.3 net protocol on our AES-GCM transport, owns the world model and replicates it. The C++ `internal` module becomes an in-process **oracle**: state reads, line traces, effects, and the dev menu. It is not the authority.
- For: this path is proven tonight end to end. The client's own multiplayer code (prediction, then server confirmation) does the gameplay. Multiplayer later is natural, because it is a real server.
- Against: the server cannot simulate physics. Anything that needs the world (hit traces, where a smoke lands, movement validation) must come from the oracle, not be faked.

**Option B: local authority (the 2026-10-13 plan).** The Range runs standalone inside the client, and `internal` drives the engine's own server code.
- For: real game logic, no protocol encoding.
- Against: it is still blocked. Input and movement were dead after the force-spawn, and the native player setup is unsolved. It throws away the working protocol work. Multiplayer would need a different design anyway.

**Recommendation: A.** Keep B's lesson: never fake what the game can compute. Ask the oracle instead.

## 3. Target architecture (Option A)

### `server`: Rust workspace (replaces the Python responder and backend)

| Crate | Contents |
|---|---|
| `bits` | UE bit archive: `SerializeInt`, packed uint, FString, FName (hardcoded and string), quantized vectors, compressed rotator, NetQuantize*, gameplay-tag net index. |
| `net` | Stateless challenge and handshake, AES-GCM key schedule, packet header and ack history, connection. **Channel manager with reliable sequencing, retransmission and ack tracking**, open/close with close reasons, partial bunches (lifts the 8 kbit cap that blocks skinned two-attachment guns). |
| `rep` | NetGUID registry (export once per connection, path to GUID), rep-layout tables as data files (class, property, handle, kind), RPC field tables (from FClassNetCache dumps), content-block and RPC encoders and decoders, dynamic and stable subobjects. |
| `world` | Authoritative model per connection: actors, components, inventory slots, equippables, state machines and mirror policy, resources, damage sections, bots. **Change tracking drives replication** (shadow state like FRepLayout), instead of one-shot sends at hand-picked sequence numbers. |
| `range` | Game-mode logic: bring-up as a **declarative dependency graph** (each step waits for the acks or client facts it needs, not a timer), shop and attachments, abilities and tuning, settings, streaming, bots, HUD fixes. |
| `platform` | HTTPS on 127.0.0.1:58089 (axum + rustls) for the ~53 routes, RMS websocket, admin API, persistence compatible with today's JSON state. |
| `oracle` | IPC client to `internal`: named pipe, length-prefixed messages, request/response plus subscriptions (predicted states, resources, RNG seed, camera, widget state) and actions (line trace, effect spawn). |
| `aries-server` (bin) | One TOML config with feature toggles, replacing the 107 flags. |

### `internal`: C++20 DLL (replaces the engine bridge versions and the ReadProcessMemory scripts)
- A **game-thread task queue** with a per-frame time budget, and no heavy polling. Stutter rule: the oracle pushes cached snapshots; the server never blocks the game thread.
- Name-based SDK lookups, with an offset table generated from dumper7 and shared with Rust as one generated data file.
- Oracle services: equippable predicted and auth states, resources, firing RNG seed, camera POV, HUD widget states, line traces (bot hits, smoke landing point), tag net indices.
- The dev menu (ImGui), ported from `dev_menu.cpp`.

### `loader`: C++ exe (replaces `launch.ps1`, `start_single_instance_backend.ps1` and the helper scripts)
- Invariants: pinned exe hash, firewall containment, single game instance, loopback-only backend, startup gate.
- Starts `aries-server`, launches the game, injects `internal`, supervises, and offers "enter Range" (the admin route).

### `sdk`: C++ static lib
- Dumper7 13.06 SDK plus helpers.

## 4. Migration strategy (never break the working Range)

1. **Golden fixtures first.** A Python exporter writes every builder's output (bytes and bit count) for representative inputs to JSON. Rust tests must match bit for bit. This turns 49 builders into 49 regression suites for free.
2. **Real traffic fixtures.** Record decrypted client packets (logins, movement, input, ServerPurchase, settings, level visibility) into fixture files. Decoders are tested against real bytes, not only round-trips.
3. **Data, not code, for tables.** Pull every handle and field index out of comments and constants into data files (`rep_tables/*.toml`), each with a "verified by" note. Both Python and Rust read them during the transition.
4. **Shadow mode.** The Rust server runs beside the Python responder, decodes the same incoming packets and logs what it would send. A diff tool compares that with Python's actual sends. Switch over only when the full bring-up and a scripted play session diff clean.
5. **Platform services port independently.** HTTP routes are compared request by request against `backend.py` using the existing test fixtures.

## 5. Phases and exit tests

| Phase | Work | Exit test |
|---|---|---|
| P0 Freeze and fixtures | Golden exporter, packet recorder, extract `rep_tables`, list every bring-up step with its dependencies | Fixtures committed; Python tests still pass |
| P1 `bits` + `net` | Transport, handshake, channels with retransmission, packet decode | The live client connects to Rust and stays connected (keepalive), with shadow decode matching Python |
| P2 `rep` + bring-up | NetGUIDs, rep tables, declarative bring-up graph | The client spawns into the Range with pawn, HUD and Classic, served by Rust |
| P3 Gameplay parity | Mirror, input correction, ammo, purchases (with attachments), abilities and tuning, settings, streaming, bots | Scripted play test: buy, fire, ADS, abilities, bots, escape menu, all from Rust |
| P4 `platform` | Port `backend.py` | The client reaches the menu and enters the Range with Python stopped |
| P5 `internal` + `loader` | Oracle IPC replaces ReadProcessMemory and the bridge; loader replaces PowerShell | One-click launch; no ReadProcessMemory in the server; no frame hitches (measured) |
| P6 Retire Python | Archive `local_backend`, keep its tests as fixtures | — |
| Later | Multiplayer: second connection, relay, oracle traces for lag-compensated hits | Separate design |

P1–P3 and P4 can run in parallel: Claude on `net`/`rep`/`range`, codex on `platform` plus the IDA work for the oracle.

## 6. Rules carried over
- Never guess a bit width, handle or index: read it from IDA or live data and record the evidence next to it.
- Never poll heavy work on the game thread. Measure frame times before and after.
- Never send a reference the client cannot resolve. A null attachment entry or an unknown GUID can crash the client.
- Containment stays: pinned exe, loopback only, firewall rules, single instance, no surprise relaunches of a live session.

## 7. Decisions (2026-10-14, by the user)
1. **Option A**: the Rust server is the network game server, and `internal` is the in-process oracle plus the dev menu.
2. **Both ports in parallel**: Claude ports the game server (`bits`/`net`/`rep`/`world`/`range`); codex ports `platform` (`backend.py`).
3. **Loader in C++**.
4. **IPC over a named pipe**.
