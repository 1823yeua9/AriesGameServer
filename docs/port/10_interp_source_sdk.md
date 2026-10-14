# 10 Interpolation, extrapolation and lag compensation (Source SDK 2013 reference, Aries `interp` design)

Spec for the `interp` crate (`DESIGN.md` crate table) and the places in `world` / `range` that use it.

This area produces no wire bytes of its own. It decides **which position** the existing hit test and builders use. So bit parity comes down to two checks:
1. With today's static bots, every shot must give exactly the Python result. Section 4.7 shows why it does.
2. The bodies sent after a hit come from the existing builders. Section 5.9 has a vector for one.

## 0. Runtime requirements (from the user)

- The external server runs at **128 ticks per second**: `TICK_DT = 1/128 = 0.0078125 s`. This value is exact in binary (f32 and f64), so tick times and multiples of them are exact. Section 3.15 converts every Source constant to ticks.
- **Allocate a console and log to it**: `AllocConsole` when started hidden, plus `logs/aries_server_<pid>.log` (`DESIGN.md:19-20`, `05_authority_loop.md` §8). `AllocConsole` fails when a console already exists; in that case reopen stdout/stderr on `CONOUT$` (05 verifier note 11). Section 4.12 lists what this module logs.

## 1. Sources

Valve SDK: `github.com/ValveSoftware/source-sdk-2013`. On 2025-02-18 master moved `mp/` to the repo root (commit `44f8f18`) and then added the TF2 SDK. Line numbers below come from the last commit before that move, `9ad9aeb83a20c973eb6bb6c2d56a45551ec901e8`, which is the classic `mp/src` tree.

How current master differs for these files:
- `interpolatedvar.h`: identical.
- `player_lagcompensation.cpp`: adds pose parameters to the record. They are stored, then set from the selected record and never interpolated (master `src/game/server/player_lagcompensation.cpp:85-126, 336, 679-690, 862`). It also switches to `intp` index types.
- `c_baseentity.cpp`: the interpolation code is unchanged.

| Abbrev | File (under `mp/src/`) |
|---|---|
| IV | `game/client/interpolatedvar.h` |
| IVC | `game/client/interpolatedvar.cpp` |
| LF | `game/client/lerp_functions.h` |
| CBE | `game/client/c_baseentity.cpp` |
| BC | `game/client/cdll_bounded_cvars.cpp` |
| LC | `game/server/player_lagcompensation.cpp` |
| GI | `game/server/gameinterface.cpp` |
| PL | `game/server/player.cpp` |
| HP | `game/server/hl2mp/hl2mp_player.cpp` |
| BE | `game/server/baseentity.cpp` |
| SD | `game/shared/shareddefs.h` |
| ML | `public/mathlib/mathlib.h` |

The engine (`mp/src/engine`) is not public. The net channel latency, `GetLastTimeStamp` and the `sv_client_*_interp_ratio` defaults are described only through how game code uses them.

License: every algorithm here is described in our own words and formulas. No Source code is copied, and the Rust below is original.

Python refs: `ruh` = `local_backend/range_udp_handshake.py`, `rb` = `range_bots.py`, `rp` = `range_projectiles.py`, `res` = `read_equippable_state.py`, `rmn` = `read_move_number.py`.

## 2. Production tie-in: what the Python does today

The Python has **no** interpolation, extrapolation or lag compensation; nothing in `local_backend` does it. These production features depend on time:

| Feature | Python | Time behaviour today | What `interp` adds |
|---|---|---|---|
| Shots on bots and dummy | seed-change detection `ruh:1622-1632`; `bot_shots` `ruh:969-1012`; hit geometry `rb:289-355` | At most one shot per seed change per 50 ms mirror poll (`ruh:1603-1604`), only while the item `is_current` and has resources (`ruh:1624`); the first seed read per `(source, equippable.key)` only primes `firing_seeds` (`ruh:1628-1630`). Camera read at **poll** time (`ruh:979`), not fire time (05 quirk 13). Targets tested at their current location. | Rewinds targets to where the client saw them at fire time. Bots never move (`rb:136-157`), so the result is identical today. |
| Bot lifecycle | bot tick `ruh:2034-2062`, every >= 0.1 s once Range streaming was sent | Placed once by `place()` (random, `rb:148-157`) and never moved. At most one new bot per bot tick while fewer than `count` are alive and >= 0.3 s since the last spawn (`ruh:2052-2060`). Death sets `alive False` and `died_at` (`ruh:1005-1006`). Retire after 1.5 s closes the channel (`ruh:2047-2051`). The production `command_queue` debug `spawn_bot` also opens a bot at a given location (`ruh:1958-1971`); it is static too. | One record per tick for every opened bot (including `spawn_bot` ones); the track is dropped on retire. |
| Spray-wall dummy | `ruh:2040-2046`; `rb:85-93` | Static at (6190, -11640, 1270), lift 4.0 (`rb:234`), never dies. | Recorded like a bot (static). |
| Player location | `read_saved_moves` in the move-ack block (`ruh:4412-4428`, `rmn:45-177`). That block is nested inside the session-ack branch (`ruh:2172`, `if decoded_packet` at 2174; the move-ack `if` is at indent 28), so it runs only on a session-ack pass (>= 0.1 s since the last session ack, per source) and only when >= 0.05 s have passed since the last move-ack **send** (`last_move_ack_times` is set only on send, `ruh:4535`). Cadence <= 10 Hz; observed session-ack spacing 94-141 ms, median 125 ms, with client silences of 0.7-2.1 s during which nothing runs (`04_bringup_b.md:63`). `newest_saved_location` = 3 doubles at saved move +592 (`rmn:145-146`). It is read only when slot `min(count,256)-1` is non-null, so it can be None while `newest` exists; with `count > 256` it is slot 255, not the real newest (09 §4.17). Move float at +0x3C (`rmn:25-28, 142`, diagnostic only). | Read only to ack the move. The location was used only by the dead follow/track modes. | A position history for later consumers (4.9). |
| Initial placement | The pawn opens at `(3581.634, -12019.322, 1294.153)` (`pawn_spawn_location_probe`, `ruh:2357-2360`). The same point is `move_correction_location` (`launch.ps1:176`), sent as the correction on the first move ack that is sent (`ruh:4454-4486`; `count > 0` always holds when `numbers` is non-empty). | Deliberate discontinuities. | Reset the player-position history at the pawn open and again at the correction send. |
| Cloudburst flight (stage A) | `tick_projectiles` `ruh:1046-1100` (every >= 0.05 s); `read_projectile` `rp:308-343`; `flight_stop_point` `rp:346-360`; `ballistic_fallback` `rp:285-295` | Stops at `hit + normal*65`; at 2.0 s uses the last read; "lost" uses the last read; never seen for 0.3 s uses the ballistic fallback. | A position history per flight (diagnostics now, multiplayer relay later). |
| Client move rate | `local_backend/MOVE_ACK_FINDINGS.md:28-36`: saved-move delta time = integer ticks * 0.0078125, `UFixedMoveTimeStepManagerComponent::FixedMoveTimeStepsPerSecond = 128` (+0xD9). `rmn:97-103` reads the target/active rate bytes at component +0xD8/+0xD9, but only under `__main__` (diagnostic). `PLAN.md:30` is the older standalone experiment where the rate was 0 and was set to 128 by hand. `04_bringup_b.md:513`: `move_ack_count` 5-69 per ack, about 13-21 on a normal ~125 ms tick, up to 69 after a client silence. | The client moves in 128 Hz fixed steps. | The same rate as our tick: one client move is about one server tick. |

**Python clock.** Every `now` and gate above uses `time.monotonic()`. In the production interpreter (CPython 3.12.3, `C:\Program Files\Python312\python.exe`) that is `GetTickCount64()` with resolution 0.015625 s (checked with `time.get_clock_info('monotonic')`; also `04_bringup_b.md:63`). Readings step in 15-16 ms increments, so a `>= 0.05 s` gate passes only after about 62 ms and a `>= 0.1 s` gate after about 109 ms, and then only on the next datagram. Python times are therefore too coarse to reproduce bit for bit. The Rust clock (4.1) is f64 seconds from `Instant`, and the gate thresholds follow 05 §9.

Dead paths in this area (skip; one line each):
- `follow_client_move` / `follow_client_move_once` (`ruh:4465-4479`): chase corrections; failed live.
- `track_client_location_mode` (`ruh:4459-4464`): failed live.
- `pawn_authority_role` (`ruh:4747`): off.
- `move_ack_value_source="timestamp"` (`ruh:4447-4449`): production uses `"number"` (`launch_range.py:51`).
- `move_ack_fixed` (`ruh:4437-4443`): constant-ack diagnostic. No launcher passes it, so it stays None.
- Cloudburst stage B: bending, teleport and stop RPC (`ruh:1069-1081`): off.

## 3. Source SDK reference (algorithms)

### 3.1 Ticks and time (SD:14-20)

- The tick interval is `gpGlobals->interval_per_tick`.
- `TIME_TO_TICKS(dt) = (int)(0.5 + dt / interval)`. The C cast truncates toward zero, so it rounds half up only for dt >= 0.
- `TICKS_TO_TIME(t) = interval * t`, and `ROUND_TO_TICKS(t) = interval * TIME_TO_TICKS(t)`.
- All times are f32 seconds (`curtime`).
- Entity simulation time travels as an 8-bit tick offset from a per-entity network tick base (BE:129-143; `SIMULATION_TIME_WINDOW_BITS = 8`, SD:884). The client unwraps it to within +-127 ticks of its own tick count (CBE:340-366). UE replicates differently, so we don't use this.

### 3.2 Client history: `CInterpolatedVar` (IV)

Each networked field (origin, angles, animation vars) gets a wrapper. Each wrapper holds:
- **History**: a ring of `(changetime, value[count])`, where index 0 is the newest (`CSimpleRingBuffer`, IV:304-429). It grows in steps of 16 and never shrinks.
- The interpolation amount in seconds, the last networked value and time, and per-element looping flags.
- Type flags (IV:30-37): `LATCH_ANIMATION_VAR` (sample time is the animation time), `LATCH_SIMULATION_VAR` (sample time is the simulation time), `EXCLUDE_AUTO_*`, and `INTERPOLATE_LINEAR_ONLY`.

Adding samples:
- **When.** A network update that changed origin, angles or simulation time latches every `LATCH_SIMULATION_VAR`. The sample time is the entity's simulation time, or `curtime` if that is 0 (CBE:2542-2564, 2772-2802, 5936-5964). Predicted and client-created entities use `curtime`.
- **Insert with flush** (IV:698-737). First drop every head sample whose `changetime + 0.0001 > new time`; this covers the server clock moving back and duplicates. Then insert at the head.
- **Prune** after each add (IV:661-668): `RemoveEntriesPreviousTo(curtime - interp - 0.05)`, where 0.05 is `EXTRA_INTERPOLATION_HISTORY_STORED` (IV:41). The rule (IV:781-795): scan from newest to oldest. At the first sample older than the cutoff, keep it plus the next two older ones (Hermite needs them), and drop the rest.
- **Reset** (IV:739-752): clear, then add three copies of the current value at `curtime`.
- `NoteChanged` reports whether the value differs from the head (memcmp). It is used only to skip work.

### 3.3 Sample selection (`GetInterpolationInfo`, IV:798-888)

`target = t - interp`. Walk i = 0 (newest) toward the oldest:
1. A sample with `changetime == 0` ends the walk; zero means "empty".
2. If `target < t_i`: set `newer = i` and continue.
3. Otherwise `t_i <= target`:
   - If there is no `newer` yet, the target is at or past the newest sample. Set `newer = older = 0` and flag "no more changes". The value is then held or extrapolated (3.6).
   - Otherwise `older = i` and `dt = t_newer - t_i`. If `dt > 0.0001`, then `frac = (target - t_i) / dt`. The code caps `frac` at 2.0, but the cap is never reached because `target < t_newer`. Hermite is used when the var is not `LINEAR_ONLY`, sample `i+1` exists, and `t_i - t_{i+1} > 0.0001` (`oldest = i+1`). If `dt <= 0.0001`, `frac` stays 0, which gives the older value.
4. If the walk ends with every sample newer than the target, set `older = newer = oldest` and hold the oldest value. The extrapolation branch is entered, but it needs index `oldest + 1`, which is invalid.
5. If the history is empty, nothing is written.

`Interpolate` (IV:967-1059) then applies Hermite if chosen. Otherwise, if `newer == older`, it tries extrapolation (3.6) and else writes that sample. Otherwise it interpolates linearly. Afterwards it prunes to `currentTime - interp - 0.05`.

### 3.4 Linear, angle and looping lerp

- `Lerp(f, a, b) = a + (b - a) * f` (ML:660-664).
- Angles (`QAngle`): convert both to quaternions, slerp, convert back. Equal inputs return early (ML:690-711).
- Looping floats such as an animation cycle in [0,1) (LF:22-38): if `|b - a| >= 0.5`, add 1 to the smaller one, lerp, and keep the fractional part (add 1 if it is negative).
- `Lerp_Clamp` does nothing except for range-checked vars (LF:78-99).

### 3.5 Hermite with time fixup (LF:41-56, IV:1371-1458)

Inputs are three samples: p0 (oldest, t0), p1 (older, t1) and p2 (newer, t2). The parameter `t = frac` in [0,1) runs over [t1, t2].

The tangents are chord differences in per-segment units: `d1 = p1 - p0`, `d2 = p2 - p1`. With the standard cubic Hermite basis:

```
h00 = 2t^3 - 3t^2 + 1     h01 = -2t^3 + 3t^2
h10 = t^3 - 2t^2 + t      h11 = t^3 - t^2
p(t) = h00*p1 + h01*p2 + h10*d1 + h11*d2
```

The tangents are backward chords, not Catmull-Rom. The curve leaves p1 with slope d1 and arrives at p2 with slope d2.

**Time fixup (IV:1371-1417).** Because d1 is in per-segment units, both segments must have the same length. Let `Dn = t2 - t1` and `Do = t1 - t0`. If `|Dn - Do| > 0.0001` and `Do > 0.0001`, replace p0 with a synthetic sample one `Dn` before p1, on the line through p0 and p1:

```
s   = Dn / Do
p0' = lerp(1 - s, p0, p1)      (= p1 - (p1 - p0) * s)
t0' = t1 - Dn
```

Special cases:
- Angles never use Hermite; they fall back to a slerp of p1 and p2 (LF:103-108).
- Looping vars unwrap p0, p1 and p2 before Hermite (LF:117-157).

The curve passes through p1 at t=0 and p2 at t=1. Velocity is continuous across segments when sampling is uniform. It can overshoot, which is why values are clamped afterwards (IV:1452-1456).

### 3.6 Extrapolation (IV:126-145, 997-1030, 1344-1368; CBE:3220-3226; IVC:29)

Extrapolation happens only when the target is past the newest sample (`newer == older`) and all of these hold:
- Extrapolation is enabled this frame: `cl_extrapolate` is 1 (CBE:76) and the game is not paused (CBE:3223).
- A previous sample (index 1) exists with a nonzero time.
- The interp amount is above 0.000001.
- The last packet timestamp is <= this var's last networked time, so no packet has arrived since this var's last update.

The last condition tells "server stopped sending (choke)" apart from "entity stopped moving". When an entity stops, packets keep arriving but bring no new sample for it, so it does not extrapolate. Only a choke extrapolates.

Formula (`_Extrapolate`), with `old` = sample 1, `new` = sample 0 and `dest = t - interp`:
- If `|t_new - t_old| < 0.001` or `dest <= t_new`, hold `new`.
- Otherwise `amount = min(dest - t_new, E)`, where `E = cl_extrapolate_amount = 0.25 s` (IVC:29).
- `value = lerp(1 + amount / (t_new - t_old), old, new)`, which equals `new + (new - old) * amount / (t_new - t_old)`. This applies to Vector, float and QAngle; other types hold `new`.

So the last segment's velocity continues for at most 0.25 s, then the value freezes.

Separately, when nothing has been received for more than 0.5 s, all interpolation turns off and entities snap to their latest values (CBE:3197-3205).

### 3.7 Derivatives (IV:1062-1147, 1460-1531)

- **Linear**: `(b - a) / (t_b - t_a)`. Zero if the samples coincide or `|dt| < 0.0001`.
- **Hermite**: the derivative of the 3.5 basis, `(6t^2 - 6t, -6t^2 + 6t, 3t^2 - 4t + 1, 3t^2 - 2t)` applied to `(p1, p2, d1, d2)`, divided by `(t2 - t1)`. Uses the same time fixup.
- **Smooth velocity**: `lerp(frac, (p1 - p0)/(t1 - t0), (p2 - p1)/(t2 - t1))`. It runs after the same time fixup (IV:1500), which leaves the first slope unchanged in exact arithmetic.
- **Extrapolated velocity** (`GetDerivative_SmoothVelocity`): the last segment's velocity. With `diff = clamp(dest - t_new, 0, 2E)`, the velocity is unchanged while `diff <= E`, then scaled by `1 - (diff - E)/E`, which reaches 0 at 2E. Its staleness test is `lastTimeStamp <= dest`, a different condition from 3.6.

### 3.8 Interpolation amount (lerp time)

**Client** (BC:63-142): `GetClientInterpAmount = max(cl_interp, cl_interp_ratio / cl_updaterate)`.
- Defaults: `cl_interp` 0.1 (bounded 0..0.5) and `cl_interp_ratio` 2.0.
- When the server sets `sv_client_min_interp_ratio` to anything other than -1, `cl_interp_ratio` is clamped to [min, max] and `cl_interp` is raised to at least `min / cl_updaterate`. Those `sv_` cvars live in the closed engine, so their defaults are not in the SDK.

**Per entity** (CBE:5888-5933): in multiplayer, `interp = TICKS_TO_TIME(TIME_TO_TICKS(client_interp) + 1)`. That is one extra tick, or 2 if the server simulates on alternate ticks.
- Predicted and client-created entities use one tick.
- An optional NPC minimum exists (`cl_interp_npcs`, CBE:5865-5885).

**Server's view** (GI:2838-2869): `m_fLerpTime` comes from the client's userinfo.
- It is `max(cl_interp, ratio / updaterate)`, with ratio 0 treated as 1 and the ratio clamped to `[sv_client_min_interp_ratio, max]` when min != -1. `cl_interp` is taken raw; the ratio clamp gives the same lower bound. `updaterate` is the client's `cl_updaterate`, clamped to `[sv_minupdaterate, sv_maxupdaterate]` (GI:2838-2841).
- It is 0 if `cl_interpolate` is 0.
- It defaults to 0.1 until userinfo arrives (PL:596-597).

Ratio 2 means two update intervals of delay, so one lost or late update still leaves a pair of samples around the target.

### 3.9 Discontinuities on the client

- **Server side.** `Teleport()` bumps a 2-bit counter `m_ubInterpolationFrame` (BE:4443-4446, 7260-7263; `NOINTERP_PARITY_MAX` 4, 2 bits, SD:920-921). The counter is networked (BE:271).
- **Client side.** If the counter changed, or the move parent or attachment changed, the entity goes on a teleport list (CBE:2611-2616, 2928-2937). `ProcessTeleportList` (CBE:3022-3048) then snaps the entity to the last received origin and angles and resets every history to three copies of the current value (CBE:5851-5857).
- **Predicted entities** interpolate at a shifted time: the final predicted time, minus one tick, plus the frame fraction (CBE:2819-2828).

### 3.10 Server lag compensation: recording (LC:20-115, 233-325)

Each player has a linked list of `LagRecord`s, newest at the head. A record holds:
- flags (`LC_ALIVE`);
- origin, angles, and collision mins/maxs (pre-scaled);
- simulation time;
- per animation layer: sequence, cycle, weight and order;
- master sequence and cycle (plus pose parameters on master).

`FrameUpdatePostEntityThink` runs once per server frame, after entities think:
- In single player (`maxClients <= 1`) or with `sv_unlag` 0, it clears all history and stops.
- `dead time = curtime - sv_maxunlag`. `sv_maxunlag` defaults to 1.0, bounded 0..1 (LC:34).
- For each player slot:
  - With no player in the slot, clear the list.
  - Otherwise drop tail records with simtime < dead time.
  - Skip if the head's simtime is >= the player's simtime (no new simulation since the last record).
  - Otherwise add a record at the head with the current state.

### 3.11 Target time (`StartLagCompensation`, LC:328-421)

Guards:
- Only one session at a time.
- Skipped when the shooter's `cl_lagcompensation` is off, in single player, when `sv_unlag` is 0, or when the shooter is a bot or an observer.

Computation:
1. `lerp_ticks = TIME_TO_TICKS(m_fLerpTime)`. There is no +1 tick here; the code comment calls it "view interpolation ticks - 1".
2. `correct = clamp(net outgoing latency + TICKS_TO_TIME(lerp_ticks), 0, sv_maxunlag)`. The latency comes from the engine.
3. `target_tick = cmd.tick_count - lerp_ticks`. `cmd.tick_count` is the tick the client believed it was at when it built the command.
4. If `|correct - TICKS_TO_TIME(current_tick - target_tick)| > 0.2` s, the client's claim is distrusted: `target_tick = current_tick - TIME_TO_TICKS(correct)`.
5. `target time = TICKS_TO_TIME(target_tick)`. Every other player that passes the filter (3.12) is then backtracked.

Beyond the 0.2 s check, the client-claimed target is never clamped explicitly to [now - maxunlag, now]; history pruning bounds it.

### 3.12 Which entities to rewind

Base filter (PL:732-765):
- Skip teammates unless friendly fire is on.
- Skip entities that were not transmitted to (and acked by) this client.
- Rewind entities within `1.5 * maxspeed * sv_maxunlag`. The exact value would be sqrt 2; 1.5 avoids dead zones.
- Otherwise rewind only if the entity is inside a 45 degree cone of the shooter's view (cos 0.707107).

HL2MP **overrides** the filter (HP:619-651) rather than extending it. It replaces the teammate/friendly-fire test with an attack test: the command must have attack pressed, or the last shot must be within 5 commands (HP:623). The transmit, distance and cone tests are kept. So in HL2MP, teammates are rewound too.

Call sites wrap the bullet or melee traces in Start, traces, Finish (HP:586-606; `weapon_hl2mpbasebasebludgeon.cpp:105-114`).

### 3.13 Record selection and blending (`BacktrackPlayer`, LC:423-736)

Walk from newest to oldest, starting with `prev_origin` = the player's current origin:
1. If the record is not alive, abort: no rewind, and the player keeps the current state.
2. If the 2D squared distance between the record origin and `prev_origin` exceeds `teleport_dist^2` (`sv_lagcompensation_teleport_dist` 64, LC:28), abort ("lost track").
3. If the record's simtime is <= target, stop here.
4. Otherwise set `prev_origin` to the record origin and continue.

If the walk runs out, the last record visited (the oldest) is used as is.

**Blend.** If there is a newer neighbour (prev) with `t_rec < target` and `t_rec < t_prev`:
- `frac = (target - t_rec) / (t_prev - t_rec)`;
- origin, angles, mins and maxs are `lerp(frac, rec, prev)`.

Otherwise `rec` is copied. A target at or after the newest record copies the newest record: there is never any extrapolation.

**Animation.**
- The master sequence and cycle are interpolated only when both records have the same master sequence. The cycle is unwrapped across 1 to 0.
- Each layer is interpolated only when its order and sequence match; its weight is lerped.
- Pose parameters (master only) are taken from `rec`, not lerped.

**Optional `sv_unlag_fixstuck`** (default 0, LC:38): if the rewound box ends up in solid, also rewind whoever it overlaps, then move as far as possible toward the target (95% of the free fraction, LC:125).

### 3.14 Apply and restore (LC:582-736, 739-845, 125-164)

Source changes the live entity in place, runs the traces, then puts it back.

**Apply:**
- Angles, if `|delta|^2 > 0.01`.
- Size, if not exactly equal.
- Origin, if `|delta|^2 > 0.01`. This goes last because it relinks the spatial partition.
- Animation, always.
- Save the original values (plus simtime) and the values that were set, and flag the player. Optionally invalidate the bone cache.

**Restore (after the traces):**
- Size and angles are restored only if they still equal what lag compensation set; game code may have changed them during the traces.
- Origin: `delta = current - set`.
  - If the 2D `|delta|^2 < teleport^2`, restore to `original + delta` through a hull trace. If the trace is blocked, move 95% of the free fraction; if fully stuck, leave the entity where it is.
  - Otherwise leave it where game code put it.
- Animation and simtime are restored.

### 3.15 Constants at 128 Hz

| Source constant | Value | At 128 Hz | Used in our design |
|---|---|---|---|
| flush epsilon on insert (IV:711) | 0.0001 s | 0.0128 tick | `FLUSH_EPS` |
| segment epsilon (IV:844, 856, 1383, 1516) | 0.0001 s | | `SEGMENT_EPS` |
| extrapolation segment epsilon (IV:1353) | 0.001 s | | `EXTRAP_EPS` |
| `EXTRA_INTERPOLATION_HISTORY_STORED` (IV:41) | 0.05 s | 6.4 ticks | `history_extra` |
| `cl_extrapolate_amount` (IVC:29) | 0.25 s | 32 ticks | `max_extrapolate` |
| interpolation off without packets (CBE:3201) | 0.5 s | 64 ticks | `stale_after` |
| `cl_interp` default (BC:102) | 0.1 s | `TIME_TO_TICKS` = 13; entity amount 14 ticks = 0.109375 s | |
| `cl_interp_ratio` default (BC:68) | 2.0 | | `oracle_interp_ratio` |
| `sv_maxunlag` (LC:34) | 1.0 s (bounds 0..1) | 128 ticks | `max_unlag` |
| `sv_lagcompensation_teleport_dist` (LC:28) | 64 units, 2D, per record | 64 per tick = 8192 u/s | `teleport_dist` |
| claim tolerance (LC:390) | 0.2 s | `TIME_TO_TICKS` = 26 | `claim_tolerance` |
| `LAG_COMPENSATION_EPS_SQR` (LC:29) | 0.01 | | not needed (no mutate/restore) |
| restore fraction scale (LC:125) | 0.95 | | not needed |
| `NOINTERP_PARITY_MAX` (SD:920) | 4 (2 bits) | | the `serial` field (4.7) plays this role |

### 3.16 Source details we do not copy

1. `int flDeadtime` (LC:246) truncates the dead time to whole seconds, so up to about 1 s of extra history is kept. We use f64.
2. Zero time as the end of history (IV:820-822). For us, t = 0 is a valid server time.
3. f32 `curtime` loses precision with uptime: after one day its ulp is about 7.8 ms, one 128 Hz tick. We use f64.
4. Apply and restore (3.14). Our hit tests are analytic, so a rewound pose is just a value. Nothing needs restoring and there are no stuck checks.
5. The growing ring buffer (IV:347-368). Ours has a fixed capacity.
6. The single-player disable (LC:235-239, 348). The Range has one client, but that client still sees the world late, so we keep lag compensation on.
7. Per-layer animation records. Our hit shapes have no animation.

## 4. Aries design (128 Hz)

### 4.1 Clock

- **Server time** is f64 seconds since an epoch `Instant` taken once at startup. `now` is sampled once at the start of each tick, and once per datagram while systems stay datagram-driven (08 §7.1). The oracle thread stamps snapshots against the same epoch.
- **Tick** is a `u64` counter. Records store both `tick` and that tick's `now`. Do not compute record time as `tick * TICK_DT`: actual tick start times jitter, and after a long stall the scheduler may rebase.
- **Oracle sample time** is the midpoint of the read, `(t_before + t_after) / 2`. The Python stamps nothing; the midpoint halves the bias from read latency.

```rust
pub const TICK_HZ: u32 = 128;
pub const TICK_DT: f64 = 1.0 / TICK_HZ as f64; // 0.0078125, exact

/// Source TIME_TO_TICKS. `as` truncates toward zero like the C cast; callers pass dt >= 0.
/// dt / TICK_DT == dt * 128.0 exactly (power-of-two scale), so either form gives the same result.
pub fn time_to_ticks(dt: f64) -> i64 { (0.5 + dt * TICK_HZ as f64) as i64 }
pub fn ticks_to_time(ticks: i64) -> f64 { ticks as f64 * TICK_DT }
```

### 4.2 Crate layout (`server/crates/interp`)

| Module | Contents |
|---|---|
| `time` | `TICK_HZ`, `TICK_DT`, `time_to_ticks`, `ticks_to_time` |
| `value` | `Interp` trait; `f64`, `Vec3`, `Rot`, `Step<T>` |
| `var` | `InterpolatedVar<T, N>` (4.4) |
| `stream` | `OracleStream<T, N>`: a var plus a delay and a staleness policy (4.6) |
| `lagcomp` | `LagCompensation`, `LagRecord`, `Pose`, `Rewind` (4.7) |

The hit geometry (port of `rb:289-355`) stays in `world` (bots). `interp` only hands it poses.

### 4.3 Value types

```rust
pub trait Interp: Copy {
    /// a + (b - a) * f, per component, in that operation order.
    fn lerp(a: Self, b: Self, f: f64) -> Self;
    const HERMITE: bool = false;
    /// Three-point Hermite over [p1, p2]; p0 is already time-fixed. Default: lerp(p1, p2, t).
    fn hermite(_p0: Self, p1: Self, p2: Self, t: f64) -> Self { Self::lerp(p1, p2, t) }
    /// Continue old -> new to factor f (> 1). Step types hold `new`.
    fn extrapolate(old: Self, new: Self, f: f64) -> Self { Self::lerp(old, new, f) }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec3 { pub x: f64, pub y: f64, pub z: f64 }   // UE5 locations are doubles (res:66-74, rmn:145)
impl Interp for Vec3 { const HERMITE: bool = true; /* lerp, hermite per component */ }
impl Interp for f64  { const HERMITE: bool = true; }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rot { pub pitch: f64, pub yaw: f64, pub roll: f64 }   // degrees
impl Interp for Rot { /* per axis: wrap180(a + wrap180(b - a) * f); no Hermite */ }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Step<T: Copy>(pub T);   // alive flags, enums: older value until f >= 1
impl<T: Copy> Interp for Step<T> { /* lerp: if f < 1.0 { a } else { b }; extrapolate: new */ }
```

Exact operation order, so Rust matches the reference bit for bit (no `mul_add`, no reassociation):

```
lerp:     a + (b - a) * f
hermite:  t2 = t*t;  t3 = t*t2
          h00 = 2.0*t3 - 3.0*t2 + 1.0      // ((2*t3) - (3*t2)) + 1
          h01 = -2.0*t3 + 3.0*t2
          h10 = t3 - 2.0*t2 + t
          h11 = t3 - t2
          d1 = p1 - p0;  d2 = p2 - p1
          out = p1*h00 + p2*h01 + d1*h10 + d2*h11   // summed left to right
wrap180:  x - 360.0 * floor((x + 180.0) / 360.0)   // result in [-180, 180)
fixup:    s = (t2 - t1) / (t1 - t0);  p0' = p0 + (p1 - p0) * (1.0 - s);  t0' = t1 - (t2 - t1)
extrap:   f = 1.0 + amount * (1.0 / (t_new - t_old));  out = old + (new - old) * f
velocity: (new - old) * (1.0 / (t_new - t_old))
```

Rotations are lerped per axis along the shortest arc, not by quaternion slerp (Source). Pitch/yaw cameras and yaw-only bots are the only rotations we interpolate, and the difference from slerp vanishes as the sample step shrinks. As in Source, rotations never use Hermite.

`Rot` output is always normalized by `wrap180`, even at `f = 0`. For example, pitch 350 comes back as -10. A camera taken from a `Rot` stream and fed to `view_ray` is therefore not bit-identical to the raw `read_camera` value: `cos`/`sin` of -10 and of 350 degrees can differ in the last bit. Parity mode uses the raw poll-time camera (4.8), so this only affects precise mode.

### 4.4 `InterpolatedVar<T, N>`

```rust
pub struct Sample<T> { pub t: f64, pub v: T }

pub struct InterpolatedVar<T: Interp, const N: usize> {
    ring: [Sample<T>; N],
    newest: usize,          // ring index of sample 0
    len: usize,
    pub linear_only: bool,  // Source INTERPOLATE_LINEAR_ONLY
}

pub enum Locate {
    Empty,
    AtOrAfterNewest,                                                       // target >= t[0]
    BeforeOldest,                                                          // target < t[len-1]
    Between { newer: usize, older: usize, oldest: Option<usize>, frac: f64 },
}
pub enum Kind { Hermite, Linear, HeldNewest, HeldOldest, Extrapolated { amount: f64 } }
pub struct Value<T> { pub v: T, pub kind: Kind }

pub const FLUSH_EPS: f64 = 1e-4;
pub const SEGMENT_EPS: f64 = 1e-4;
pub const EXTRAP_EPS: f64 = 1e-3;

impl<T: Interp, const N: usize> InterpolatedVar<T, N> {
    pub fn push(&mut self, t: f64, v: T);
    pub fn reset(&mut self, t: f64, v: T);
    pub fn prune_before(&mut self, cutoff: f64);
    pub fn sample(&self, i: usize) -> &Sample<T>;   // 0 = newest
    pub fn len(&self) -> usize;
    pub fn locate(&self, target: f64) -> Locate;
    /// `extrapolate`: Some(max seconds) only when the caller decided the source is stale (4.5).
    pub fn value_at(&self, target: f64, extrapolate: Option<f64>) -> Option<Value<T>>;
    pub fn velocity_at(&self, target: f64) -> Option<T>;   // linear or Hermite derivative (3.7)
}
```

Semantics. Sample 0 is the newest; times are f64 server seconds.
- **`push(t, v)`.** While `len > 0` and `t[0] + FLUSH_EPS > t`, drop sample 0. Then insert at 0. When `len == N`, the oldest sample is overwritten (fixed capacity; Source grows).
- **`reset(t, v)`.** Clear, then insert 3 copies (for Hermite). Use it on every discontinuity: initial placement correction, respawn, `serial` bump.
- **`prune_before(cutoff)`.** Find the newest sample with `t < cutoff`. If its index is `i`, set `len = min(len, i + 3)`.
- **`locate(target)`.** The Source walk from 3.3, with these changes: no zero-time sentinel, and `frac` is not capped at 2.0. If `t[newer] - t[older] <= SEGMENT_EPS`, the result is `Between` with `frac = 0` and `oldest = None`; Source only considers Hermite inside the `dt > 0.0001` branch (IV:844-860). Because `push` flushes any head sample closer than `FLUSH_EPS`, this case is reachable only at float-rounding edges. Otherwise, `oldest` is `Some(i+1)` only when `!linear_only && T::HERMITE`, sample `i+1` exists, and `t[i] - t[i+1] > SEGMENT_EPS`.
- **`value_at`:**
  - `Between` with `oldest`: Hermite, after the 3.5 fixup when `|Dn - Do| > SEGMENT_EPS` and `Do > SEGMENT_EPS`.
  - `Between` without `oldest`: lerp.
  - `BeforeOldest`: hold the oldest.
  - `AtOrAfterNewest`: if `extrapolate == Some(max)` and `len >= 2`, apply the 3.6 formula with `max` in place of `E`, including the hold when `|t0 - t1| < EXTRAP_EPS` or `target <= t0`. Otherwise hold the newest.
- **Capacity.** `N >= ceil((window + history_extra) / sample_period) + 3`. Concretely: 32 for oracle streams at 50 ms, and 256 for anything sampled per tick over `max_unlag`.

### 4.5 Extrapolation policy (server side)

- **Oracle streams.** Extrapolate only when the stream is stale: no snapshot newer than the stream's newest sample has arrived. Every snapshot is pushed even if the value is unchanged, so "stale" means the oracle stalled, not that the value stopped changing. This is our form of Source's choke test (3.6). Because every snapshot is pushed, the test always holds once `now - delay` is past the newest sample. The real trigger is therefore that the oracle delivered nothing for longer than `delay`. Cap it at `max_extrapolate = 0.25 s` (32 ticks), then hold. After `stale_after = 0.5 s` (64 ticks), log a warning and report the value as held.
- **Lag-compensation records.** Never extrapolate (Source 3.13). A target past the newest record uses the current pose (4.7).

### 4.6 `OracleStream<T, N>` (smoothing reads taken at irregular times)

```rust
pub struct OracleStream<T: Interp, const N: usize> {
    pub var: InterpolatedVar<T, N>,
    pub nominal_period: f64,   // the poll period we chose for this read
    pub delay: f64,            // ticks_to_time(time_to_ticks(ratio * nominal_period) + 1)
    pub lookback: f64,         // oldest age value_at(t) must answer: 0 for display-only streams, max_unlag for the camera
    pub newest_snapshot_t: f64,
}
impl<T: Interp, const N: usize> OracleStream<T, N> {
    pub fn ingest(&mut self, now: f64, sample_t: f64, v: T); // push + prune_before(now - max(delay, lookback) - history_extra)
    pub fn value_now(&self, now: f64) -> Option<Value<T>>;  // value_at(now - delay, stale ? Some(max_extrapolate) : None)
    pub fn value_at(&self, t: f64) -> Option<Value<T>>;     // exact-time lookups (shots); no delay
}
```

- **Delay rule.** This is Source 3.8 with `ratio = oracle_interp_ratio = 2.0`. For a 50 ms poll: `time_to_ticks(0.1) + 1 = 14` ticks = 0.109375 s. For a 0.1 s feed (the Python session-ack cadence of the saved-move read): `time_to_ticks(0.2) + 1 = 27` ticks = 0.2109375 s. For a per-tick read: 3 ticks = 0.0234375 s. `nominal_period` must be the period the stream is actually fed at, not the Python gate it came from.
- **Two uses.** `value_now` gives smooth per-tick values. `value_at(t)` answers "where was it at time t". Shots only need `value_at`.
- **Pruning vs `value_at`.** Pruning at `now - delay - history_extra` alone would drop the history that `value_at(fire_t)` needs. Precise mode looks up a fire time that can be one oracle stall old, so `lookback` sets the cutoff: `now - max(delay, lookback) - history_extra`. Size N to match: a per-tick camera with `lookback = max_unlag` needs `ceil(1.05 * 128) + 3 = 138 <= 256`.

### 4.7 `LagCompensation`

```rust
pub type EntityKey = u32;   // actor NetGUID. One LagCompensation per connection, because bots are per
                            // connection (`bot_fields[source]`, ruh:2039); shared once the world is shared.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HitShape { TrainingBot { lift: f64 } }   // regions rb:66-68; lift 0.0 bots, 4.0 dummy (rb:93, 234)

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose { pub origin: Vec3, pub yaw: f64, pub shape: HitShape, pub alive: bool, pub serial: u16 }

#[derive(Clone, Copy)]
pub struct LagRecord { pub tick: u64, pub t: f64, pub pose: Pose }

pub const LAG_RING: usize = 256;   // 1 s at 128 Hz = 128 records, with 2x headroom

pub struct LagCompConfig { pub enabled: bool, pub max_unlag: f64, pub teleport_dist: f64, pub claim_tolerance: f64 }
// Default: true, 1.0, 64.0, 0.2 (Source values, 3.15)

pub enum TargetSource { Claimed, Latency }
pub struct Target { pub t: f64, pub source: TargetSource, pub lerp: f64, pub correct: f64 }

pub enum Lost { DeadOnPath, TeleportDistance, SerialChanged }
pub enum Rewind {
    Exact { pose: Pose, t: f64 },
    Interpolated { pose: Pose, older_t: f64, newer_t: f64, frac: f64 },
    Clamped { pose: Pose, t: f64 },   // target older than all history: oldest pose
    Unavailable(Lost),
}

impl LagCompensation {
    pub fn record(&mut self, tick: u64, now: f64, poses: impl IntoIterator<Item = (EntityKey, Pose)>);
    pub fn forget(&mut self, key: EntityKey);   // bot retired / channel closed
    pub fn prune(&mut self, now: f64);          // drop records with t < now - max_unlag (f64, no truncation)
    pub fn target_time(&self, now: f64, rtt: f64, view_interp: f64, claimed_fire: Option<f64>) -> Target;
    pub fn rewind(&self, key: EntityKey, current: &Pose, now: f64, target: f64) -> Rewind;
}
impl Rewind {
    /// The pose to hit-test: Exact/Interpolated/Clamped -> pose; Unavailable -> current if alive, else None.
    pub fn effective(&self, current: &Pose) -> Option<Pose>;
}
```

**`record`** runs once per tick, after all systems (Source's "post entity think"). It records every compensated entity: opened bots and the opened dummy. Every tick produces a record, because our entities are simulated every tick; Source instead skips a record when simulation time did not advance. Cost: 6 entities x 256 records x about 64 bytes is negligible.

**`target_time`** has the same structure as Source 3.11, in f64 with sub-tick precision:

```
lerp    = ticks_to_time(time_to_ticks(view_interp))            // Source lerp ticks, no +1
correct = clamp(rtt + lerp, 0.0, max_unlag)
if let Some(tf) = claimed_fire {
    target = tf - lerp;  source = Claimed
    if |correct - (now - target)| > claim_tolerance { target = now - correct; source = Latency }
} else { target = now - correct; source = Latency }
target = clamp(target, now - max_unlag, now)                    // explicit; Source relies on pruning
```

Why `rtt + view_interp`:
- The client receives server state from time S at `S + down`. It then displays it after its view delay, so it shows state from about `client_now - down - view_interp`.
- Its shot reaches the server `up` later. So the target is `now - up - down - view_interp`, which is `now - rtt - view_interp`.
- Source uses the engine's outgoing-latency figure, which is closed source; we use our own RTT.
- **Range caveat (oracle-detected shots, `fire_t = None`).** A Range shot is not a packet. It is seen when the mirror poll reads a changed seed, so the "up" leg is really the detection delay. That delay runs from 0 up to one mirror poll period (>= 50 ms, about 62 ms under Python's clock), not `rtt/2`. The camera is also read at poll time, not fire time (05 quirk 13). For moving targets, the Latency target is therefore biased late by up to one poll period. With static bots this has no effect. Precise mode (4.8) removes the bias. Until then, `view_interp` calibration (4.13) must not try to absorb it.
- RTT comes from the `net` ack tracking: `rtt sample = recv time of the client packet that first acks server sequence S - send time of S`. It is smoothed with an EWMA, `srtt += (sample - srtt) / 8`. That is our choice (TCP style), not Source's. On loopback it is near 0.

**`rewind`.** Build a chain: `c[0]` = the current pose at `now`, then the records from newest to oldest. Then:

```
prev = current.origin
for i in 0..chain.len():
    r = c[i]
    if !r.alive                                         -> Unavailable(DeadOnPath)
    if r.serial != current.serial                       -> Unavailable(SerialChanged)
    dx = r.origin.x - prev.x;  dy = r.origin.y - prev.y
    if dx*dx + dy*dy > teleport_dist*teleport_dist      -> Unavailable(TeleportDistance)
    if r.t <= target                                    -> found i; break
    prev = r.origin
not found                                               -> Clamped(oldest)
found i: if i > 0 && r.t < target && r.t < c[i-1].t {
             frac  = (target - r.t) / (c[i-1].t - r.t)
             pose  = { origin: lerp(r.origin, c[i-1].origin, frac),
                       yaw: wrap180(r.yaw + wrap180(c[i-1].yaw - r.yaw) * frac),
                       shape: r.shape, alive: true, serial: current.serial }   -> Interpolated
         } else                                                                -> Exact(r)
```

How this differs from Source:
- **The current pose is a virtual newest record at `now`.** Shots run mid-tick, before that tick's `record`. Without this, a target between the last record and `now` would copy a pose one tick old. With it, the target is interpolated between the last record and `now`.
- **The `serial` field is an explicit discontinuity counter**, like Source's interpolation frame (3.9). Bump it whenever server code moves an entity in a jump (teleport, respawn at the same key). The 64-unit 2D test stays as a safety net. It allows up to 8192 u/s per tick at 128 Hz; Cloudburst at 2900 u/s (`rp:268`) is 22.7 u per tick.
- **Shapes are discrete.** They are taken from the older record; Source lerps mins/maxs.
- **Nothing is mutated or restored** (3.16 item 4).

**Parity with the Python.**
- Bots and the dummy never move (`rb:136-157`, `ruh:2040-2046`), so every record has the current origin. `lerp(a, a, f) = a + (a - a) * f = a` exactly.
- `hit_region` ignores yaw: a sphere and vertical cylinders (`rb:326-338`).
- A dead bot is skipped by Python's `first_hit` (`rb:350`) and gets `effective() == None` here.
- A bot opened after the target time comes back `Clamped` to its spawn pose and stays hittable. That matches Python, where it is hittable as soon as it is opened (`ruh:2059`).

So the hit, region and distance are identical to the Python for every shot, and lag compensation can be on in parity mode.

### 4.8 Shot pipeline (Range, `world`/`range`)

```rust
pub struct Shot {
    pub class: ClassName,
    pub origin: Vec3, pub dir: Vec3,   // camera ray (view_ray(pitch, yaw), rb:289-291)
    pub poll_t: f64,                   // when the seed change was seen
    pub fire_t: Option<f64>,           // exact client fire time mapped to server time (internal oracle, later)
}
```

1. **Detection is unchanged** (05 §6.4, 08 §4.4). A firing-seed change on the >= 0.05 s mirror poll makes one `Shot`. The camera is read at poll time (`ruh:979`), and `fire_t = None`.
2. `target = lagcomp.target_time(now, srtt, cfg.view_interp, shot.fire_t)`.
3. For each bot in `bots` list order with `opened`: `pose = lagcomp.rewind(bot.actor_guid, &current_pose(bot), now, target.t).effective(..)`. Skip `None`. `hit_region(origin, dir, pose.origin + (0, 0, lift))`. Keep the nearest, **replacing only on a strictly smaller distance** (`rb:353`).
4. Do the same for the opened dummy with lift 4.0. The dummy wins if it is hit and there is no bot hit or `d_dummy < d_bot` (`ruh:991`).
5. Damage, health/alive bodies and events are exactly as in 05 §6.4 / 08 §4.4.

**Precise mode** is a behaviour change and needs the user's approval and the `internal` oracle (P5):
- `internal` reports discrete fire events with the client's time.
- `ClockMap` maps client time to server time: `offset = min over the last 2 s of (server receive time - client time)`. The minimum filter removes pipe-latency spikes.
- The camera comes from the camera history at `fire_t`: `loc` and `rot` vars, `value_at(fire_t)`.
- `target_time` gets `claimed_fire = Some(fire_t)`.
- This fixes 05 quirk 13 (camera at poll time) and the shot merging inside a 50 ms poll.

### 4.9 Other consumers

| Consumer | Stream / track | Feed | Use | When |
|---|---|---|---|---|
| Moving bots (future) | `LagCompensation` per bot | `record` each tick from the server's own bot position | Shots rewind correctly once bots move. Needs a RepMovement builder for TrainingBot_PC; its handle is unverified (do **not** reuse the projectile's handle 11, 05 §6.13). | When bots move |
| Player location | `OracleStream<Vec3, 32>` | `newest_saved_location` (`rmn:145-146`, saved move +592). Python reads it only on the move-ack path: on session-ack passes, <= 10 Hz, median about 125 ms, none during client silences (section 2). Feed it with `nominal_period = 0.1` (27-tick delay). If it is fed from the 128 Hz oracle snapshot instead, use `TICK_DT` (3 ticks) and size N accordingly. **Gap:** the 09 §7.2 snapshot `saved_moves` and the 09 §4.17 minimal read set do not read +592; add `newest_saved_location: Option<[f64; 3]>` (None when slot `min(count,256)-1` is null) for this stream. Skip None samples. | Dev menu and diagnostics now; multiplayer relay later. `reset` at the pawn open (`ruh:2357-2360`) and at the initial placement correction (`ruh:4454-4486`). | P3, cheap |
| Camera | `OracleStream<Vec3, 256>` + `OracleStream<Rot, 256>`, `lookback = max_unlag` | `read_camera` (`res:66-74`). Python reads it only inside `bot_shots` and on the smoke-throw transition (`ruh:979, 1027`). The 09 §7.2 snapshot carries `camera` on every oracle snapshot (up to 128 Hz), so ingest each snapshot. | Precise mode (4.8); Cloudburst throw origin at the throw transition | P5 (internal oracle) |
| Cloudburst flight | `InterpolatedVar<Vec3, 64>` per flight | `read_projectile` location (`rp:336`) every >= 0.05 s, stamped with read time. A flight lasts at most 2.0 s (`rp:64`), so 64 samples is enough at the 0.05 s cadence. If it is fed from the per-tick snapshot, use N = 256 or accept the ring overwriting the oldest samples. | Overlay trail; multiplayer relay (resample at `now - delay`). Parity keeps "lost" = last read (`rp:356-357`); an extrapolated lost point would be opt-in. | P3 diagnostics |
| Remote players (multiplayer) | `OracleStream` per player (from their moves) + `LagCompensation` | Server-resampled position each tick | Replicate to others; rewind shots by `rtt_shooter + view_interp_shooter` | Later |

These do **not** need `interp`: bring-up stages, the equippable mirror, resources (ammo is an integer), purchases, settings, HUD and streaming. They are state machines, not continuous values.

### 4.10 Tick integration (128 Hz)

For tick `k` with `now`:
1. Drain the socket and decode, queueing facts (05 §9).
2. Ingest every oracle snapshot published since the last tick into its stream, each with its own sample time.
3. Run the Range systems in Python order (05 §2 table). Shots are applied in the mirror step (#2), before the bot tick (#13), as in Python. Rewinds see records up to tick `k-1` plus the current pose at `now`.
4. The bot tick may retire bots: call `lagcomp.forget(actor_guid)` when their channel closes (`ruh:2047-2051`).
5. `lagcomp.record(k, now, opened bots + dummy)`, then `lagcomp.prune(now)`. Each stream prunes itself on ingest.
6. Flush, then wait for the next tick.

If systems stay datagram-driven in parity mode (08 §7.1), steps 3-4 run per datagram with that datagram's `now`. Steps 2 and 5 still run every tick. The clock is shared, so the results are the same.

Budget: one rewind walks at most 129 entries per entity, with at most 6 entities per shot. That is negligible against 7.8125 ms.

### 4.11 Configuration (`aries-server.toml`)

```toml
[interp]
oracle_interp_ratio = 2.0    # Source cl_interp_ratio default (BC:68)
max_extrapolate     = 0.25   # Source cl_extrapolate_amount (IVC:29)
stale_after         = 0.5    # Source: interpolation off after 0.5 s without packets (CBE:3201)
history_extra       = 0.05   # Source EXTRA_INTERPOLATION_HISTORY_STORED (IV:41)

[lagcomp]
enabled         = true
max_unlag       = 1.0        # Source sv_maxunlag (LC:34)
teleport_dist   = 64.0       # Source sv_lagcompensation_teleport_dist (LC:28), 2D, per record
claim_tolerance = 0.2        # Source StartLagCompensation (LC:390)
view_interp     = 0.0        # the client's effective view delay for remote entities; measure it (4.13)
shot_camera     = "poll"     # "poll" = Python parity; "fire_time" needs the internal oracle and user approval
```

### 4.12 Logging (console + file)

Console: `AllocConsole` when started hidden; colour output plus `logs/aries_server_<pid>.log` (`DESIGN.md:19-20`). The `tracing` targets are `interp` and `lagcomp`.

| Level | Line |
|---|---|
| info, once | `lagcomp: on, tick 128 Hz (7.8125 ms), max_unlag 1.000 s (128 ticks), teleport 64 u/record, view_interp 0.000 s, camera poll` |
| debug, per shot | `lagcomp: shot now=… target=… (-59.2 ms, latency) 300108 interpolated 7.8515625->7.859375 frac 0.426 -> head 853 u` |
| warn | `lagcomp: target clamped to max_unlag` / `lagcomp: 300108 unavailable (teleport_distance)` / `interp: stream camera stale 312 ms (holding)` |
| trace | every `ingest` (stream, sample t, value) |

JSON-lines sink: new keys `lagcomp_shot` `{now, target, source, results: [[guid, kind, frac]]}`, `lagcomp_unavailable`, `interp_stale`. These keys are not in the Python, so the shadow-diff tool must ignore them (05 §8).

### 4.13 Calibrating `view_interp` (measure, do not guess)

The client's real view delay for simulated proxies (TrainingBot_PC, later remote players) depends on UE's smoothing settings in 13.06, which are unverified. Measure it before bots move:
1. Move a test bot along a known server path. This needs the RepMovement handle; see open question 3.
2. Have the oracle read the client's copy of that actor every tick. The chain is the same one `read_projectile` uses (`rp:318-336`): `world+0x40` (net driver) `-> +0xC0` (server connection) `-> Channels = [+0x13F8]`, count `u32 [+0x1400]`, `-> [Channels + 8*ch] -> +0x78` (actor) `-> +0x290` (root component) `-> +0x320` (3 doubles).
3. Find the delay `d` that minimizes `sum |client(t_i) - server_track(t_i - d)|^2`, stepping `d` by one tick over [0, 0.3 s].
4. Set `view_interp = d - rtt/2`, which is about `d` on loopback.

While bots are static, `view_interp` has no effect.

### 4.14 Rust tests

- `time`: the vectors in 5.1.
- `var`: push/flush, prune, every `Locate` case, Hermite with and without fixup, `linear_only`, extrapolation (cap, tiny-segment hold, `target <= t0` hold), rotation wrap, velocity (5.3-5.6). Assert with `to_bits()` equality.
- `stream`: `ingest` pruning with and without `lookback` (5.4), and the delay for each feed period (5.2).
- `lagcomp`: `target_time` (5.7) and `rewind` (5.8). Include dead on the path, the teleport-distance abort when the target lies past the jump, a serial change, clamped, no records, and exact at now.
- `range` integration: the moving-bot vector (5.9) end to end, through the health body hex. Also check that static bots give the same hit/region/distance with lag compensation on and off.

## 5. Test vectors

These come from an offline Python reference of section 4, using the exact operation order from 4.3. Hit tests call `range_bots.hit_region` and the body comes from `range_bots.build_bot_health_body`; both are imported offline, with no sockets and no live process. Floats are shown as Python `repr`, and as `float.hex()` where bit equality matters.

### 5.1 Ticks (128 Hz)

```
time_to_ticks: 0.0078125->1  0.015625->2  0.05->6  0.1->13  0.109375->14  0.2->26  0.25->32  0.3->38
               1.0->128  0.0039->0  0.00390625->1 (exact half rounds up)
ticks_to_time: 1->0.0078125  7->0.0546875  13->0.1015625  14->0.109375  26->0.203125  32->0.25  128->1.0
```

### 5.2 Lerp time (Source 3.8 formulas at 128 Hz)

```
cl_interp ratio rate | client_interp | entity_interp (ticks)  | lag-comp lerp ticks -> s
0.1   2.0  20        | 0.1           | 0.109375 (14)          | 13 -> 0.1015625
0.1   2.0  128       | 0.1           | 0.109375 (14)          | 13 -> 0.1015625
0.0   2.0  128       | 0.015625      | 0.0234375 (3)          | 2  -> 0.015625
0.0   1.0  64        | 0.015625      | 0.0234375 (3)          | 2  -> 0.015625
0.031 2.0  128       | 0.031         | 0.0390625 (5)          | 4  -> 0.03125
0.0   2.0  10        | 0.2           | 0.2109375 (27)         | 26 -> 0.203125
```

The `OracleStream` delay uses the entity column: a 50 ms poll gives 0.109375 s, a 0.1 s feed (rate 10) gives 0.2109375 s, and a per-tick read gives 0.0234375 s.

### 5.3 `InterpolatedVar<Vec3, 32>`: linear and Hermite

Pushed in order (t, v):
```
(10.0,       (0, 0, 0))
(10.0078125, (5, 0, 1))
(10.015625,  (10, 0, 4))
(10.0234375, (15, 0, 9))
(10.0390625, (25, 2, 16))     <- a 2-tick gap before the newest sample
```
Queries:
```
target 10.03125   -> Hermite newer 0 older 1 oldest 2 frac 0.5, fixup yes: p0' = (5, 0, -1) at t0' 10.0078125
                     value (20.0, 0.75, 12.875)  hex (0x1.4000000000000p+4, 0x1.8000000000000p-1, 0x1.9c00000000000p+3)
target 10.02734375-> Hermite newer 0 older 1 oldest 2 frac 0.25, fixup yes
                     value (17.5, 0.21875, 11.171875)  hex (0x1.1800000000000p+4, 0x1.c000000000000p-3, 0x1.6580000000000p+3)
target 10.01953125-> Hermite newer 1 older 2 oldest 3 frac 0.5, fixup no
                     value (12.5, 0.0, 6.25)
target 10.0234375 -> Hermite newer 0 older 1 oldest 2 frac 0.0 -> (15.0, 0.0, 9.0) (exactly p1)
target 10.0       -> Linear newer 3 older 4 frac 0.0 (no sample 5) -> (0.0, 0.0, 0.0)
target 9.99       -> HeldOldest (0, 0, 0)
target 10.05      -> HeldNewest (25, 2, 16) (no extrapolation requested)
linear_only, target 10.03125 -> Linear newer 0 older 1 frac 0.5 -> (20.0, 1.0, 12.5)
```

### 5.4 Push flush and prune

```
push (1.0,1) (2.0,2) (3.0,3) then (2.5,25)  -> [(2.5,25), (2.0,2), (1.0,1)]      3.0 dropped (3.0 + 1e-4 > 2.5)
then push (2.50005,26)                       -> [(2.50005,26), (2.0,2), (1.0,1)]  2.5 dropped (2.5 + 1e-4 > 2.50005)
push t=0..9 (value = t), prune_before(6.5)   -> times [9, 8, 7, 6, 5, 4]           first t < 6.5 is 6 (index 3); keep 3 + 3
push (1.0,a) then (1.00005,b)                -> [(1.00005,b)]                      1.0 dropped (1.0 + 1e-4 > 1.00005)
```

`OracleStream` prune with `lookback` (4.6). Feed `t = k/128` for k = 0..300 into N = 256, with `now = 300/128 = 2.34375`, delay 3 ticks (0.0234375) and `history_extra` 0.05:
```
lookback 1.0: cutoff now - max(0.0234375, 1.0) - 0.05 = 1.29375  -> len 138, oldest tick 163
lookback 0.0: cutoff now - 0.0234375 - 0.05 = 2.2703125          -> len 13,  oldest tick 288
```

### 5.5 Extrapolation (`value_at(target, Some(0.25))`)

History: `(1.0, (0, 0, 0))`, `(1.05, (10, 0, -5))`.
```
target 1.04 -> Linear frac 0.8 -> (8.0, 0.0, -4.0)
target 1.05 -> HeldNewest (target <= t0)
target 1.1  -> Extrapolated amount 0.050000000000000044, f 2.0 -> (20.0, 0.0, -10.0)
target 1.3  -> Extrapolated amount 0.25 (capped), f 5.999999999999996
               -> (59.99999999999996, 0.0, -29.99999999999998)  hex (0x1.dfffffffffffap+5, 0x0.0p+0, -0x1.dfffffffffffap+4)
target 1.5  -> same as 1.3 (cap)
history (1.0, 0), (1.0005, 10), target 1.2 -> HeldNewest 10 (segment 0.0005 < EXTRAP_EPS)
```

### 5.6 Velocity and rotation

```
linear velocity (1.0,(0,0,0)) -> (1.05,(10,0,-5)) = (199.99999999999983, 0.0, -99.99999999999991)
   hex (0x1.8fffffffffffap+7, 0x0.0p+0, -0x1.8fffffffffffap+6)
velocity ramp (E = 0.25, t_new = 1.05): dest 1.2 -> scale 1.0 | dest 1.35 -> diff 0.30000000000000004, scale 0.7999999999999998
   | dest 1.45 -> scale 0.40000000000000036 -> (80.0, 0.0, -40.0) | dest 1.6 -> diff 0.5 (clamped at 2E), scale 0.0
hermite derivative p0 (0,0,0) p1 (5,0,1) p2 (10,0,4), t 0.5, seg 0.0078125 -> (640.0, 0.0, 448.0)
lerp_rot((0,170,0), (0,-170,0), 0.5)  = (0.0, -180.0, 0.0)
lerp_rot((0,170,0), (0,-170,0), 0.25) = (0.0, 175.0, 0.0)
lerp_rot((350,10,0), (10,30,0), 0.5)  = (0.0, 20.0, 0.0)
lerp_rot((-10,0,0), (20,90,0), 0.75)  = (12.5, 67.5, 0.0)
InterpolatedVar<Rot>: (2.0, yaw 179), (2.0078125, yaw -179), (2.015625, yaw -177); target 2.01171875
   -> Linear (no Hermite for Rot) newer 0 older 1 frac 0.5 -> (0.0, -178.0, 0.0)
```

### 5.7 `target_time` (`now = ticks_to_time(1000) = 7.8125`, max_unlag 1.0, tolerance 0.2)

```
rtt     view_interp claimed  | lerp       correct    target                         source
0.0     0.0         None     | 0.0        0.0        7.8125                         Latency
0.0123  0.05        None     | 0.046875   0.059175   7.753325  (0x1.f0367a0f9096cp+2) Latency
0.0123  0.05        7.8025   | 0.046875   0.059175   7.755625  (0x1.f05c28f5c28f6p+2) Claimed
0.0123  0.05        7.4125   | 0.046875   0.059175   7.753325                       Latency (|0.059175 - 0.446875| = 0.3877 > 0.2)
1.5     0.1         None     | 0.1015625  1.0        6.8125                         Latency (correct clamped)
0.03    0.1         7.8625   | 0.1015625  0.1315625  7.7609375 (0x1.f0b3333333333p+2) Claimed (claim 50 ms in the future, still within 0.2)
```

### 5.8 `rewind` edge cases (x-only track, `current.x = 100` at `now = ticks_to_time(10)`)

Records are `x = 90, 80, 70, 60, 50` at ticks 9, 8, 7, 6, 5 (`t = tick/128`). Results give the pose x and the chain indices (0 = current):
```
target tick 10   -> Exact x 100 (current, i 0)
target tick 9.5  -> Interpolated x 95.0, older i 1 (tick 9), newer i 0 (current), frac 0.5
target tick 8    -> Exact x 80 (i 2)
target tick 7.25 -> Interpolated x 72.5, older i 3, newer i 2, frac 0.25
target tick 4    -> Clamped x 50 (oldest, i 5)
track 90@9, 10@8, 9@7 (jump of 80 > 64 between ticks 9 and 8):
   target 7.5 -> Unavailable(TeleportDistance) at i 2
   target 8.5 -> Unavailable(TeleportDistance) at i 2 (the record at/before the target is across the jump)
track 90@9 dead, 80@8, 70@7; target 7.5 -> Unavailable(DeadOnPath) at i 1
track 90@9 serial 1, 80@8 serial 0, current serial 1; target 8.5 -> Unavailable(SerialChanged) at i 2
no records; target tick 9 -> Clamped x 100 (current is the only and oldest entry)
```

### 5.9 End to end: moving bot, rewound head shot, health body

The motion is hypothetical (production bots are static). It uses bot0 from `BotField(count=5)` after `allocate_dummy()`: channel 401, actor 300108, health 300110, base `(4463.170774427629, -14407.674895086804, 1351.8)`, yaw 65.86815966797721 (matches 05 §11).

Records at ticks 1000..1012, with `x = base.x + 600 * (t - 1000/128)` and `t = tick/128`. Now is tick 1013, current `x = 4524.108274427629`.
```
target_time(now 7.9140625, rtt 0.0123, view_interp 0.05) -> 7.8548875 (0x1.f6b67a0f9096cp+2), Latency, correct 0.059175
rewind -> Interpolated older i 8 (tick 1005), newer i 7 (tick 1006), frac 0.4256000000000313   (target = tick 1005.4256)
          origin (4488.603274427629, -14407.674895086804, 1351.8)
          hex (0x1.1889a7031612ep+12, -0x1.c23d662f65307p+13, 0x1.51f3333333333p+10)
camera (4825.0, -13600.0, 1415.8); ray aimed at rewound centre + (0, 0, 75):
   dir (-0.3844540300597448, -0.9230585341336824, 0.012571449152668357)
   hex (-0x1.89ae5137b0221p-2, -0x1.d89b20d0cbb51p-1, 0x1.9bf0f57cad42dp-7)
hit_region(camera, dir, rewound)  -> ('head', 852.9985675012831)
hit_region(camera, dir, current)  -> None   (without lag compensation the shot misses)
shot_damage("BasePistol_C", "head") = 78 -> life 22.0
build_bot_health_body(bot0, export=False)  bits=133
  75469124be9a09636d28d321bb071be803
```

## 6. Open questions

1. **Saved-move float at +0x3C** (`rmn:25-28`). Is it the client move timestamp in seconds? It would make a better sample time for the player-position stream than our read time. Verify in IDA before using it.
2. **Client view delay for simulated proxies** in 13.06 (UE network smoothing on TrainingBot_PC and remote characters). Measure it (4.13); never assume UE defaults.
3. **RepMovement handle for TrainingBot_PC.** Needed before bots can move; unverified.
4. **Exact fire-time source for the `internal` oracle.** Which client function marks a shot (firing-state entry or the RNG seed advance), and which client clock it uses. This is an IDA task.
5. **Precise mode (4.8)** changes which shots hit, compared with Python (05 quirk 13, 08 open question 6). It needs the user's approval.
6. **`max_unlag` for multiplayer.** Source's 1.0 s lets a high-ping or cheating client rewind far. Decide it when multiplayer starts.
7. **Teleport-like abilities.** No speed figures are verified (dash, Updraft). Server code that moves an entity in a jump must bump `serial`; do not tune `teleport_dist` from guesses.
8. **Spawn-before-target shots.** Lag compensation should eventually refuse a hit on a bot spawned after the target time instead of using `Clamped`. That would be a behaviour change from Python (`ruh:2059`), so it is off in parity mode.
9. **What saved move +592 holds.** It could be the move's start or end location. No IDA or doc evidence was found; only the dead follow/track modes ever used it. Verify it before the player-location stream feeds anything beyond diagnostics.

## Verifier corrections

An independent check re-read the Python (`ruh`, `rb`, `rp`, `res`, `rmn`) and the Source SDK files at `9ad9aeb8` (and master for the pose-parameter and `intp` claims). It also regenerated every section 5 vector with a separate offline reference that imports `range_bots` read-only. Every original vector matched bit for bit, including the 133-bit health body `75469124be9a09636d28d321bb071be803`. Changes:

1. **Section 2, player location cadence (wrong).** The saved-move read is not "every >= 0.05 s". The move-ack block is nested inside the session-ack branch (`ruh:2172/2174`, block at indent 28), and `last_move_ack_times` is set only on send (`ruh:4535`). So it runs on session-ack passes, <= 10 Hz (observed median 125 ms, with 0.7-2.1 s client silences, `04_bringup_b.md:63`). Added the `newest_saved_location` None and slot-255 caveats (09 §4.17).
2. **Section 2, client move rate (wrong citation and number).** `04_bringup_b.md:509` does not say "13-17 saved moves"; 04's own verifier corrected that figure to 5-69, about 13-21 per normal tick (`04:513`, `04:1542`). `PLAN.md:30` describes the old standalone experiment. The rate now cites `MOVE_ACK_FINDINGS.md:28-36` (integer ticks * 0.0078125; `FixedMoveTimeStepsPerSecond = 128` at +0xD9), and `rmn:97-103` is marked `__main__`-only.
3. **Section 2, Python clock (missing).** Added that `time.monotonic()` in the production CPython 3.12.3 is `GetTickCount64()` (resolution 0.015625 s, verified with `get_clock_info`), so the 0.05 s and 0.1 s gates actually fire at about 62 ms and 109 ms.
4. **Section 2 rows (missing detail).** Shots: the `is_current`/resources gate and first-read seed priming. Bots: spawn rule (`ruh:2052-2060`) and the production `command_queue` `spawn_bot` path (`ruh:1958-1971`). Initial placement: the pawn opens at the same point (`ruh:2357-2360`) as a second reset point. Dead list: added `move_ack_fixed` (`ruh:4437-4443`).
5. **3.12 (wrong).** HL2MP **overrides** `WantsLagCompensationOnEntity`: it replaces the teammate/friendly-fire test with the attack test, so it does not add to the base filter.
6. **3.8 (imprecise).** Server-side lerp: `cl_interp` is taken raw, and `cl_updaterate` is clamped to `sv_min/maxupdaterate` (GI:2838-2841).
7. **3.7.** Smooth velocity runs after the time fixup (IV:1500); no numeric change.
8. **4.3 (missing).** `Rot` output is `wrap180`-normalized even at f = 0, so a stream camera is not bit-identical to the raw read; this only matters in precise mode.
9. **4.4 (ambiguous).** `locate` returns `oldest = None` when the bracketing segment is <= `SEGMENT_EPS`, as Source does. Noted that `push`'s flush makes this reachable only at rounding edges.
10. **4.5.** Clarified that with every snapshot pushed, the real extrapolation trigger is that no snapshot arrived for longer than `delay`.
11. **4.6 (design inconsistency).** `ingest` pruned at `now - delay - history_extra`. That contradicts `value_at(fire_t)` lookups and the 256-sample camera stream. Added `lookback` and a `now` argument: the cutoff is `now - max(delay, lookback) - history_extra`. Added the 0.1 s feed delay (27 ticks) and a prune vector (5.4) plus a lerp-time row (5.2), both regenerated offline.
12. **4.7 (missing caveat).** For oracle-detected Range shots, the "up" leg is the mirror-poll detection delay (0 to about 62 ms), not `rtt/2`, so the Latency target is biased late by up to one poll for moving targets. No effect with static bots.
13. **4.9 (gaps).** The player-location stream needs `newest_saved_location` (+592), which the 09 §7.2 snapshot and the 09 §4.17 minimal read set do not read. Its `nominal_period` must match its real feed (0.1 s gives 27 ticks; per-tick gives 3). Camera: Python reads it only at shots and throws (`ruh:979, 1027`), but the 09 snapshot reads it at up to 128 Hz, with `lookback = max_unlag`. Cloudburst: sizing for a per-tick feed.
14. **4.13.** Wrote out the full `read_projectile` pointer chain (`rp:318-336`).
15. **0.** `AllocConsole` fails when a console already exists; reopen stdout/stderr on `CONOUT$`.
16. **6.** Added open question 9: what saved move +592 holds is unverified.

Checked and unchanged: all Source line refs and constants (LC:28/29/34/38/125/246/390, IV:41/711/781-795/798-888/1344-1458, IVC:29, BC:68/102, CBE:76/3197-3226/5888-5933, SD:14-20/884/920-921, BE:129-143/271/4443-4446/7260-7263, PL:596-597/732-765, HP:586-606), the commit history (9ad9aeb8, then 44f8f18 moved `mp/`, then the TF2 SDK), the master-only pose parameters and `intp` claims, and every Python ref and constant in the parity argument (`rb:66-68, 93, 234, 289-355`; `ruh:969-1012, 1603-1632, 2034-2062`; `rp:64, 268, 285-295, 308-360`; `res:66-74`).
