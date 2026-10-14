# AriesGameServer — port plan

> **Superseded architecture (2026-10-14):** see `PORT_PLAN_v2.md`. The Rust server is the network game server, and `internal` is an in-process oracle plus the dev menu, not the gameplay authority. The phases below are kept for history.

Target: VALORANT 13.06 (`5.3.2-5590001 Ares-Core release-13.06`), the pinned project-local build.
Source of the port: `../local_backend` (Python), `../range_authority_probe` (C++ bridge + dev menu),
`../dumper7/output/.../CppSDK` (13.06 SDK), `../launch.ps1` + ~30 PowerShell scripts.

## Architecture

| Component | Runs | Owns |
|---|---|---|
| `internal` (DLL) | inside the game | **Gameplay authority**: world/GameMode bring-up, player setup, spawn/possess, inventory and equip, firing, movement. Calls the game's own functions. Dev menu sits on top. |
| `server` (Rust, standalone Cargo crate, not in the VS solution) | separate process | **Non-critical services**: authentication, config/patches, skins/entitlements/loadouts, player data (party, presence, name, XP). Later: interpolation/extrapolation (derived from Source engine). |
| `loader` (exe) | launcher | Containment checks, start `server`, launch the game, inject `internal`, supervise. Replaces the PowerShell scripts. |
| `sdk` (static lib, new) | shared | 13.06 SDK from dumper7, name-based lookups, UE containers. |

### Why authority lives in `internal`

The current Python responder fakes a game server from outside. It can't run physics or game logic, so it can only echo acks. Every fake ack or correction fought the client's prediction: snap-back, flicker, slow walking, a silent gun. Inside the game, the engine's own server code (GameMode, CharacterMovement authority path, inventory, weapon fire) can run for real.

Concretely, the Range world runs with **local authority** (standalone or listen NetMode) instead of as a network client of a fake server. The single game process stays a single process.

## What is already known (from `local_backend/COMPATIBILITY.md`, 2026-10-11/12)

| Finding | Status |
|---|---|
| Reflected `GameplayStatics::OpenLevel` to the Range **standalone** loads the map | works |
| Same with `?listen` | `LoadMap: failed to Listen`, `UWorld::NetDriver` stays null |
| `UFixedMoveTimeStepManagerComponent` active rate is 0 on the standalone instance → divide-by-zero at RVA `0x7449822` (`128 / FixedMoveTimeStepsPerSecond`) | worked around by setting the live instance rate to 128; proper initializer unknown |
| GameState stuck in `GameStateIntroComponent_C` (`GoToState ... null NewState`) | `SetOverrideNextState(GameStateShootingRange)` fixes it |
| Spawn prerequisites missing: `RespawnManager` null, no `PlayerInfo`, `DesiredClass` zero, `LoadedCharacterClasses` empty | `LoadCharacterAssetsSynchronous` **on the game thread** loads Wushu; the rest are unsolved |
| Force-spawn (`ChooseSpawnPoint` + deferred spawn + `Possess` + `SetViewTarget`) | visible first-person pawn, but **movement, look and firing dead** |
| Root gap | normal server-side **player setup / input initialization** was skipped |

The reference 1.08 server (`local_backend/reference_ares_gameserver_108`) sets up players via the engine's own path:
`GameMode::HandleStartingNewPlayer` → spawn `GetDefaultPawnClassForController(PC)` → `SetOwner` →
`InitializeHUDForPlayer` → `AuthPossessSpawnedCharacter`. That is the leading candidate for the missing setup.

## Phases

Each phase has an exit test. The Python stack stays the working baseline until Aries reaches parity.

### Phase 0: Feasibility spike (mostly read-only)
- Map the 13.06 native player-setup sequence a real server runs: `PostLogin` / `HandleStartingNewPlayer` / `RestartPlayer` / respawn-manager creation / `PlayerInfo` creation / `AuthPossessSpawnedCharacter`. Record names and RVAs.
- Find what normally sets the fixed-step manager's active rate (the server-side initializer), so we don't have to poke the byte.
- Find why controls were dead after force-spawn: `EnableInput`, `PlayerInput`, `InputComponent` bindings, `bPlayerIsWaiting`, `ClientRestart` handling.
- One contained, user-approved trial: standalone Range + the reference setup sequence.
- **Exit:** a written call sequence that yields a possessed pawn with working input in a standalone Range.

### Phase 1: Skeleton and infrastructure
- Project types: `internal` → DLL; add `sdk` static lib; `loader` stays exe. `server` is the Rust crate in `server/` (`cargo build`).
- `loader`: port `launch.ps1` invariants (pinned exe hash, firewall containment, single game instance, loopback-only backend, startup gate), launch, inject (`Injector` stub).
- `internal`:
  - hook framework (MinHook is already vendored in `range_authority_probe/dev_overlay/vendor`)
  - **game-thread task queue**: work runs in small slices per frame, never long blocking calls
  - logging
  - IPC to `server` (named pipe)
- **Exit:** loader launches the contained game, injects, and `internal` logs from the game thread with no measurable frame hitch.

### Phase 2: Server parity for platform services
- Port `backend.py` contracts: TLS on 127.0.0.1:58089, auth/JWT, config, session heartbeat, entitlements, loadouts, party/presence, store/skins, account XP, display name.
- Port the backend test fixtures (captured request/response shapes) as unit tests.
- **Exit:** client reaches `MainMenuV2` against Aries `server` with the Python backend stopped.

### Phase 3: Local-authority Range
- `internal` handles ENTER RANGE by bringing up the Range with local authority and the Phase 0 player setup, instead of travelling to `127.0.0.1:58100`.
- **Exit:** spawned and possessed Wushu, correct view target, HUD, input live. No UDP responder running.

### Phase 4: Gameplay
- Movement (local authority, no ServerMove/ack loop), jump, look.
- Weapon equip and **fire**, reload, ammo.
- Agent abilities, ESC menu, agent switch (F2).
- **Exit:** walk, jump, look, shoot and switch agents in the Range, with no freezes (measured, not eyeballed).

### Phase 5: Dev menu
- Port v16 tabs (session, player, agents, skins, actors) onto `internal`. Reads are cached and cheap per frame.
- Skin/agent changes go through `server` data, applied in-game by `internal`.

### Phase 6: Retire Python and PowerShell
- Remove the UDP responder and Python backend from the launch path; archive.

### Later: multiplayer
- `server` relays player state between clients; clients render others with Source-style interpolation (interp delay, snapshot buffer) and bounded extrapolation; lag compensation for hit registration.
- Needs its own design pass (what is authoritative where once there are two clients).

## Rules carried over (learned the hard way)
- **Never poll heavy work on the game thread.** The 2026-10-13 Range stutter was a watcher calling a 2.1 s game-thread bridge command every 4.6 s. Read state from outside (ReadProcessMemory) or cache it.
- **Name-based lookups over hardcoded offsets** (`FindObject` / reflection). The build is pinned, but offsets rot.
- **Keep containment**: project-local pinned exe, loopback only, firewall rules, single game instance, Vanguard untouched.
- **Evidence first**: prove a cause from logs/IDA before changing the live game; make the smallest reversible change; measure before/after.
- **Coordinate live sessions** with codex: one owner per running game, no surprise relaunches.

## Open questions
1. ~~`server` language~~ Decided: Rust, standalone Cargo crate in `server/`, removed from the VS solution.
2. ENTER RANGE: intercept the client's normal travel and replace it with local bring-up, or trigger bring-up from the dev menu/loader? (The old "use normal ENTER RANGE, never OpenLevel" rule protected the fake-server path. Here local bring-up is intentional, but it must run the full setup.)
3. `internal` ↔ `server` IPC: named pipe (proposed) or localhost TCP?
4. Single-player only until Phase 6, or design for multiplayer from the start?
5. Work split: suggest codex owns Phase 0 IDA traces (fits the existing IDA/live-test split), Claude owns Phase 1–2 scaffolding and the server port.
