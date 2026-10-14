# 01 Transport and connection lifecycle

Port spec for the Rust `net` layer. Source of truth: the Python Range responder in
`local_backend` as launched in production on 2026-10-14. Every number below is copied from
the cited `file:line`; nothing is inferred unless marked **(UE reference, unverified)** or
listed in section 18. Sibling specs: `03_bringup_a.md` / `04_bringup_b.md` (stage chain, move and
input ACKs), `05_authority_loop.md` (per-packet authority work), `06_client_decode.md` (C->S
bunch and RPC decoding), `DESIGN.md` (crate layout). The prose in sections 6, 8 and 10 was
re-implemented independently and checked against the vectors in section 17.

Files (all under `C:\Users\Cayden\Desktop\valorant proj\local_backend` unless noted):

| Short name | File |
|---|---|
| `H` | `range_udp_handshake.py` (4919 lines) |
| `B` | `range_server_bunch.py` |
| `T` | `range_udp_transform.py` |
| `K` | `check_live_packet_keys.py` |
| `L` | `launch_range.py` |
| `PS` | `..\launch.ps1` |

## 0. Conventions

- **Bit order.** All fields are LSB-first. "Bit n of a byte string" means
  `(int.from_bytes(bytes, "little") >> n) & 1` (`H:275-282` `read_bits`, `B:21-25`
  `BitWriter.append_bits`). A multi-bit field of width w at bit n holds
  `(value >> i) & 1` at bit `n+i`.
- **Byte output.** `BitWriter.to_bytes()` is `value.to_bytes(ceil(bit_count/8), "little")`
  (`B:74-75`); unused high bits of the last byte are 0.
- **Sequence arithmetic.** Packet sequences are 14-bit (`& 0x3FFF`); reliable channel
  sequences are 12-bit (`& 0xFFF`).
- **"source"** is the UDP peer `(ip, port)` tuple. Every per-connection structure is keyed by it.
- **"S->C"** server to client, **"C->S"** client to server.

## 1. Production configuration (transport-relevant)

Launch chain: `PS:166-192` starts `L` with `--seconds 3600 --no-health-check --keepalive`
plus the probe flags; `L:214-219` starts `H` with:

```
--port 58100 --seconds 3600 --output runtime/range_launch_<pid>_phase4_match_state.json
--raw-challenge --raw-envelope-bytes 4 --raw-terminal-only
--live-client-pid <pid> --live-client-image-base 0x<base>
<probe_flags_setup L:29-43> <probe_flags_reconcile L:49-53> --game-state-phase-value 4 ...
```

Transport-relevant `run_probe` values: `raw_challenge=True`, `raw_envelope_bytes=4`,
`raw_terminal_only=True`, `derived_key_gcm=False`, `player_key=""` (CLI default `H:4594`),
`session_ack_probe=True`, `welcome_map_url="/Game/Maps/Poveglia/Range"` (default `H:4756`),
`live_client_pid`/`live_client_image_base` set. Environment: `VALORANT_RANGE_HEARTBEAT_HZ`
unset (=0), `VALORANT_RANGE_QUIET_PACKETS` unset, `VALORANT_RANGE_CAPTURE_LOGIN` unset
(no production launcher sets them; only `runtime\launch_v28_loop.bat` sets QUIET).

`session_ack_probe` alone enables the Challenge, Welcome, Join-ACK and session-ACK branches:
the Challenge/Welcome/Join conditions are an `or` that includes `session_ack_probe` (`H:2111`,
`H:2115`, `H:2143`, `H:2158`); the session-ACK branch requires it directly
(`session_ack_probe and ...`, `H:2172`). `main()` rejects `session_ack_probe` combined with any
`*_probe_once` flag (`H:4776-4777`).

Base-pistol initial cosmetic trial: `PS:189-192` passes `--base-pistol-initial-cosmetic-trial`
(default on), `L:286-287` forwards it, `H:4908-4909` turns it into env
`RANGE_BASE_PISTOL_INITIAL_COSMETIC_TRIAL=1`, read by `run_probe` at `H:737-739`. It affects only
the pistol actor open (`03_bringup_a.md`), not transport.

### Dead paths (do not port)

| Path | Where | Status |
|---|---|---|
| `derived_key_gcm` encrypted Challenge, `prefix_bytes`, `inner_magic_bytes` | `H:4549-4557` | Off; client rejected these. |
| `raw_envelope_bytes` 0/5/8, 0xCC trailer on Challenge (`raw_terminal_only=False`) | `H:428-442`, `H:4559` | Off; production is 4-byte prefix, no trailer. |
| `ack_probe_once` / `build_server_ack_probe` | `H:579-595`, `H:2131-2133` | Off. |
| `challenge/welcome/join_ack_probe_once` | `H:2111-2171` | Off; `session_ack_probe` covers the same branches. |
| `welcome_map_url` `?listen` | `H:708` | Off. |
| Server heartbeat (`VALORANT_RANGE_HEARTBEAT_HZ`>0) | `H:1366-1369`, `H:1416-1437` | Off (0). |
| Quiet logging (`VALORANT_RANGE_QUIET_PACKETS=1`) | `H:1365`, `H:1377-1400` | Off. |
| Login capture (`VALORANT_RANGE_CAPTURE_LOGIN=1`) | `H:2079-2085` | Off. |
| `derive_client_receive_key(player_key)` | `H:386-399`, `H:422` | Unused: `receive_key_override` is always set before any S->C frame. |
| `pre_state` packet-notify scan before Challenge (`inspect_packet_notify`) | `H:2116-2126` | Production code, but diagnostic only (event fields); no wire effect. Do not port. |
| `actor_open/static/exported_static/game_state/player_controller/player_state_probe_once` | `H:2185-2329` | Off as flags; `player_pawn_probe_once` (on) drives these same branches (`03_bringup_a.md`). |
| `csvt_probe`, `goto_state_probe`, `server_performance_rpc`, `client_game_phase_begin_probe` | `H:4364-4407`, `H:4221`, `H:1752-1776`, `H:3818` | Off. |
| `move_ack_uniform_transform` | `H:4912-4914` | Off (affects content transform, not packet transform). |
| `pawn_authority_role`, `follow_client_move(_once)`, `track_client_location`, `acr_layout_trial`, `base_pistol_skin_trial` | various | Off. |
| `RANGE_ACR_ACTOR_INSPECTION_PROBE`, `RANGE_EQUIP_STATE_TRIAL` | `H:740`, `H:865` | Unset. |
| `VALORANT_RANGE_DROP_FLAGS` (launcher A/B) | `L:212-213` | Unset. |

## 2. Process lifecycle (Python)

| Step | Python | Detail |
|---|---|---|
| Preconditions | `L:187-197`, `L:56-120` | Exactly one `VALORANT-Win64-Shipping.exe` (Toolhelp snapshot); image base = first module from `EnumProcessModules` (`OpenProcess(0x0410)`); `check_project_client` (`L:110-120`, called at `L:192`) refuses any client whose path is not the project-local `game req\VALORANT_GAME\VALORANT\live\ShooterGame\Binaries\Win64\VALORANT-Win64-Shipping.exe`; refuse if UDP 58100 already owned (`Get-NetUDPEndpoint`). |
| Bind | `H:1410-1414` | `AF_INET/SOCK_DGRAM`, bind `127.0.0.1:<port>`; prints `listening on 127.0.0.1:<port>`. |
| Recv timeout | `H:1412-1413` | `0.25 s` (heartbeat off). |
| Lifetime | `H:1409`, `H:1440` | Loop until `time.monotonic() >= start + seconds` (3600). |
| Liveness | `H:1443-1454`, `H:148-163` | Only on a recv timeout, at most every 1.0 s: `OpenProcess(SYNCHRONIZE=0x00100000)`, `WaitForSingleObject(h, 0) == WAIT_TIMEOUT(0x102)` means alive. Dead -> print `{"status":"live_client_exited",...}` and leave the loop. |
| ICMP reset | `H:1455-1459` | `ConnectionResetError` on `recvfrom` is ignored. |
| Exit | `H:4581-4584` | Writes `{"bind","total_events","retained_events","events"}` (last 8192 events) to `--output`, opened with mode `"x"` (fails if the file exists). |
| Restart | `L:304-323` | `--keepalive`: on responder exit, restart the same command if the same client pid/base is still running and the port is free. A restarted responder has no state for the old source, so the client's live session is not resumed (it keeps sending, gets nothing; see sample in section 16). |

Python `time.monotonic()` on this machine has 15.625 ms granularity (Python 3.12.3:
`time.get_clock_info("monotonic")` = `GetTickCount64()`, resolution 0.015625; 3.13+ would use
QPC): every logged `elapsed_seconds` is a multiple of 1/64 s up to float rounding (e.g. 63.625,
64.03125 printed as 64.031). Consequence for every Python time gate `now - t >= g`: both values
are tick-quantised, so the gate opens after `ceil(g * 64)` ticks of tick time (0.1 s -> 7 ticks
= 0.109375 s; 0.05 s -> 4 ticks = 0.0625 s; 0.25/0.5/1.0/2.0 s exact), which in real time is
anywhere in `((ticks-1)/64, (ticks+1)/64)`.

## 3. Rust runtime requirements

User requirements (2026-10-14): **the external server runs at 128 ticks per second** and
**allocates a console with logging**.

### 3.1 Tick loop (128 tps)

- Tick period `1/128 s = 7.8125 ms`. (Same value as the Python default
  `build_server_performance_rpc_payload(frame_time_ms=7.8125)`, `B:980`, described at
  `H:1749-1751` as a healthy 128 Hz server frame time; that RPC is off in production.)
- Windows default timer resolution is ~15.6 ms. Use a high-resolution waitable timer
  (`CreateWaitableTimerExW` with `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`, Windows 10 1803+; this
  machine is 10.0.19045) or `timeBeginPeriod(1)` plus a short spin, and schedule ticks on
  absolute deadlines (no drift accumulation). The tick thread runs at highest priority
  (`DESIGN.md`). If a tick overruns by more than one period (e.g. a slow handler), advance the
  deadline to the next future multiple instead of running catch-up ticks back to back.
- Socket: non-blocking UDP bound to `127.0.0.1:58100`. Each tick: receive until `WouldBlock`;
  handle each datagram in arrival order with the exact per-datagram algorithm of section 4;
  frames built while handling a datagram are sent in build order (sending them at the end of
  the tick is allowed; reordering is not, because each frame consumed the next server sequence).
- ICMP port-unreachable: Windows reports it as `WSAECONNRESET` on a later `recvfrom`
  (`H:1455-1459`, Python ignores it). In Rust this is `io::ErrorKind::ConnectionReset` from
  `recv_from`: treat it as "no datagram, keep draining", never as end-of-drain or a fatal error
  (alternatively disable it with `WSAIoctl(SIO_UDP_CONNRESET, FALSE)`). Otherwise one stale client
  travel socket stops the drain for every other source.
- Receive buffer 2048 bytes (`H:1442`). A larger datagram: Python's `recvfrom` would raise
  WSAEMSGSIZE and crash the loop; Rust drops it and logs.
- Time gates (`>= 0.1 s` session ACK, `>= 0.05 s` move ACK, others in `03`-`05`) are evaluated
  per datagram with the monotonic clock read at that point, as in Python. Use a fine clock
  (`Instant`); do not quantise to 15.625 ms. Python's clock is tick-quantised (section 2), so a
  Rust server can open a gate up to ~16 ms earlier or later than Python did for the same traffic;
  make the clock injectable, and in replay/shadow tests drive it from the logged
  `elapsed_seconds` so gate decisions match the Python log exactly.
- **Parity mode (required for shadow diffing, phase P1-P3):** no server-initiated traffic.
  Every S->C frame is a reaction to a received datagram, exactly as in Python (heartbeat is off
  in production). The tick only drains the socket, runs timers that have no wire effect
  (liveness, key-scan completion) and flushes logs.
- Reliable retransmission (`DESIGN.md`, "net: retransmit unacked reliables") is an addition
  beyond Python: a resend reuses the bunch's channel sequence but takes a new packet sequence, so
  every later S->C packet sequence shifts by one relative to Python. Make it switchable; keep it
  off for shadow diffs, or have the diff tool normalise packet sequences.
- Liveness: check the client process once per second (Python checks only after 0.25 s of
  silence; the difference has no wire effect). Exit when the client exits.
- Do not port the 3600 s lifetime (section 18, Q4).
- Key scan (section 7) must not block the tick: run it on a worker thread. To match Python,
  where the scan blocks the loop: when the first datagram arrives without keys, hold it, start
  the scan and stop draining the socket (later datagrams wait in the OS buffer, as they did in
  Python; raise `SO_RCVBUF` so a 30 s scan cannot overflow it). Scan success: handle the held
  datagram normally, then resume draining. Scan failure: record the held datagram with
  `handler_state: "not_ready"` (`H:1476-1481`) and drop it; the next datagram starts a new scan.

### 3.2 Console and logging

- At startup, if `GetConsoleWindow()` is null or `!IsWindowVisible(GetConsoleWindow())`, call
  `FreeConsole()` (ignore its result) and then `AllocConsole()`. (The Python responder ran with
  `CREATE_NO_WINDOW` and stdout redirected to `runtime\responder_<pid>_phase4_match_state.out`,
  `L:288-291`; the launcher itself ran under `Start-Process -WindowStyle Hidden`, `PS:214-217`.)
  **(Windows API behaviour, not from the Python)** `AllocConsole` fails while the process is
  still attached to a console, and a process started with `CREATE_NO_WINDOW` or a hidden window
  is attached to a console nobody can see, so `FreeConsole` must come first. A server started
  from a visible terminal keeps that terminal. A `windows_subsystem = "windows"` build has no
  console at all and goes straight to `AllocConsole`.
- Set a console title (e.g. `Aries Range server :58100`). Write the console sink to `CONOUT$`
  opened after `AllocConsole`, so it works even if stdout was redirected. Colour (`DESIGN.md`):
  enable `ENABLE_VIRTUAL_TERMINAL_PROCESSING` on the `CONOUT$` handle and use ANSI colours; if
  that call fails, write plain text.
- Closing the allocated console window terminates the process (`CTRL_CLOSE_EVENT`). Install
  `SetConsoleCtrlHandler` so close/Ctrl+C flushes the logger and writes the exit summary (below)
  before returning.
- Also log to a file (`DESIGN.md`: `logs/aries_server_<pid>.log`).
- Logging must never block the tick: send records over a bounded channel to a logger thread
  that writes console and file; if the channel is full, drop the record and count the drop
  (report the count periodically). Windows console writes can take milliseconds.
- Record format: one JSON object per handled datagram with the same keys Python puts in
  `event` (section 4), so the shadow diff tool can compare Rust and Python line by line.
  Python prints every event when not quiet (`H:1375-1377`). Also log the status lines Python
  prints: `listening on ...` (`H:1414`), the key line `read-only Ares handler=0x.. receive-key
  fingerprint=<sha256(receive_key)[:12]>` (`H:1482`), `live_client_exited` (`H:1451`).
- Add `handler_elapsed_ms` when handling took >= 25 ms (`H:4575-4577`).
- Keep the last 8192 events and the total count; write the same summary JSON at exit, but with
  create-or-overwrite semantics (Python's `"x"` mode fails on a keepalive restart).
- Never log keys (Python logs only the receive-key fingerprint, `H:1482`). Decrypted bytes
  appear only in bounded fields: `control_prefix_hex` (first 12 decoded bytes, every packet) and
  production `movement_packet_capture` `decoded_packet_hex` (at most 300 ch8 packets at >= 0.25 s
  spacing and 16 distinct ch5 packets per source, `H:2086-2106`). Keep the same bounds.

## 4. Per-datagram algorithm (production)

`H:1440-4579`. Order matters: every S->C frame takes the next server sequence when it is built.

```
on datagram (payload, source):
 1. if source.ip != "127.0.0.1": drop, no event                         H:1460-1461
 2. event = {elapsed_seconds: round(now - start, 3), source_port, length} H:1462-1464
 3. if keys not read yet:                                               H:1465-1482
        try read keys (section 7); on OSError/RuntimeError/ValueError:
            event.handler_state = "not_ready"; record + print event; return
 4. try parse_handshake(payload)  (section 5.1)                         H:1483-1484
    success -> handshake path (section 5.3)                             H:4538-4572
    ValueError e ->
     a. event += inspect_client_ares_packet(payload, client_send_key)   H:1487, H:541-557
     b. ack_history_mask = 0                                            H:1488
     c. if authenticated:                                               H:1489-1495
          ack_history_mask = update_received_packet_history(
               recent_client_sequences[source] (deque maxlen 64), client_packet_sequence)
          latest_authenticated_packets[source] = (seq, stateless_header, mask, now)  [heartbeat only]
          [input-ack trial]  if source in client_restart_sent and channel_index == 8:
                             <= 1 unreliable ch8 frame                  H:1499-1563 (04_bringup_b.md s.9)
          [authority work]   if source in client_restart_sent: 0..N frames,
                             mostly reliable via send_reliable          H:1566-2078 (05_authority_loop.md)
     d. [movement_packet_capture] log-only decoded_packet_hex           H:2086-2106
     e. first matching branch only:                                     H:2107-4535
        - not authenticated            -> event.parse_error = str(e)
        - bits == 232 and source in pending
                                       -> hello_packets[source] = decoded (no send)
        - source not in ack_sent and source in pending and bits == 77
                                       -> if source in hello_packets: send CHALLENGE (11.1)
                                          else: return WITHOUT recording the event (Python `continue`, H:2134-2135)
        - source in ack_sent and not in welcome_sent and source in pending
          and control_message_type == 5 -> send WELCOME (11.2)
        - source in welcome_sent and not in join_ack_sent and source in pending
          and control_message_type == 9 -> send JOIN ACK (11.3)
        - source in join_ack_sent and source in pending
          and now - last_session_ack_times[source] >= 0.1
                                       -> SESSION TICK (section 12):
                                          1 session ACK, then <= 1 bring-up stage
                                          (elif chain H:2185-4407), then the move-ACK
                                          block (H:4412-4535)
 5. if handling took >= 25 ms: event.handler_elapsed_ms                 H:4575-4577
 6. record (deque 8192) + print event; send_due_server_heartbeats() (no-op)  H:4573-4579
```

Notes:
- `control_message_type` is the first 8 body bits of the **first bunch only**
  (`inspect_control_bunch`, section 10.4). A Login or Join that is not the first bunch of its
  packet is not seen.
- The 232-bit branch precedes the session branch, so any authenticated 232-bit packet from a
  pending source, at any time, overwrites `hello_packets[source]` and produces no session
  tick for that datagram. Preserve for parity (section 15).
- Branch conditions test `source in pending`; a source that never sent a stateless Initial to
  this process (e.g. a client connected to a previous responder instance) is decoded and logged
  but never answered.
- The Join branch does not check the channel; `build_server_join_ack` raises ValueError if the
  first bunch is not reliable channel 0 (`H:604-606`) and that exception is uncaught in Python
  (process crash). Rust: same condition, but log and skip instead of crashing.
- `parse_error` strings (logged for unauthenticated datagrams, step 4e) come from
  `parse_handshake`: `"missing expected eight-byte Ares initial envelope"` (len < 57 or a nonzero
  byte in 0-7, `H:287`), `"not an ordinary stateless handshake"` (bit 69 != 1 or bit 70 != 0,
  `H:299`; every C->S Ares datagram >= 57 bytes lands here, its stateless header bit 5 being 0),
  `"unexpected handshake version"`, `"unexpected handshake type"`,
  `"invalid initial handshake payload"` (`H:310-314`). `read_bits` can also raise
  `"truncated handshake"` (`H:277`) but not for len >= 57.
- Other uncaught Python crash paths reachable from client input (none observed live):
  authenticated `P < 32` (`struct.unpack_from("<I", decoded)` at `H:547` raises `struct.error`);
  authenticated `P > 8192` (`inverse_short_packet` raises ValueError at `T:126-127`, inside
  `decode_client_ares_packet`, which has no handler); a Welcome or later control reply after
  `hello_packets[source]` was overwritten by a 232-bit packet whose first bunch is not a channel-0
  control Hello ending at bit 231 (`H:649-653` ValueError, or KeyError when that bunch is a close
  bunch). Rust: log, treat the datagram as unanswered, continue.

## 5. Stateless handshake

### 5.1 parse_handshake (`H:285-318`)

Input: whole datagram. Reject (ValueError) unless `len >= 57` and the first 8 bytes are zero
(`ares_initial_envelope = b"\x00"*8`, `H:22`). Fields from bit 64:

| Bits | Width | Field | Check |
|---|---|---|---|
| 64 | 2 | travel_count (UE SessionID) | |
| 66 | 3 | client_id | |
| 69 | 1 | handshake bit | must be 1 |
| 70 | 1 | restart-handshake bit | must be 0 |
| 71 | 8 | minimum_version | `<= version` |
| 79 | 8 | version | must be 4 (`handshake_version`, `H:23`) |
| 87 | 8 | packet_type | 0 Initial or 2 Response (`H:24-27`) |
| 95 | 8 | packet_count | |
| 103 | 32 | network_version | (observed 3923072530) |
| 135 | 16 | runtime_features | |
| 151 | 1 | secret_id | |
| 152 | 64 | timestamp (IEEE-754 double, LE bits) | Initial: must be 0.0 |
| 216 | 160 | cookie (20 bytes, LE) | Initial: must be all zero |

Anything after bit 376 (client padding) is ignored. Every failure is `ValueError`, which routes
the datagram to the Ares path (section 4 step 4).

### 5.2 build_challenge / build_ack (`H:321-383`)

Both emit 377 bits (`challenge_bit_count = 377`, `H:28`) = 48 bytes:

| Width | Challenge (`H:329-346`) | ACK (`H:363-378`) |
|---|---|---|
| 2 | travel_count (echo) | travel_count (echo) |
| 3 | client_id (echo) | client_id (echo) |
| 1 | 1 | 1 |
| 1 | 0 | 0 |
| 8 | minimum_version (echo) | minimum_version (echo) |
| 8 | version (echo) | version (echo) |
| 8 | 1 (Challenge) | 3 (ACK) |
| 8 | packet_count (echo of Initial) | packet_count (echo of Response) |
| 32 | network_version (echo) | network_version (echo) |
| 16 | runtime_features (echo) | runtime_features (echo) |
| 1 | **0** (secret id) | **1** (secret id) |
| 64 | timestamp (double; server value) | `-1.0` (`0xBFF0000000000000`) |
| 160 | cookie (server value) | cookie echoed from the Response |
| 64 | 8 bytes `os.urandom(8)` | 8 bytes `os.urandom(8)` |
| 1 | 1 (terminal) | 1 (terminal) |

Preconditions: Challenge needs `packet_type == 0`, `len(cookie) == 20`, `timestamp > 0`
(`H:322-323`); ACK needs `packet_type == 2` and a 20-byte cookie (`H:356-357`). IDA evidence for
the layouts (COMPATIBILITY.md, 2026-10-11 sections): native Challenge serializer RVA `0x49D90F0`
reserves 441 bits but writes 312 + 8..16 random bytes + 1 bit (helper RVA `0x49D9E00`); ACK
serializer RVA `0x49D9680`.

### 5.3 Handshake dispatch (`H:4538-4572`)

Event gets `packet_type, packet_count, network_version, client_id`.

- **Initial (0):** `timestamp = time.monotonic()` (float seconds, > 0), `cookie = os.urandom(20)`,
  `pending[source] = (timestamp, cookie)` (overwrites any previous entry), send
  `wrap_raw_challenge(build_challenge(...), 377, 4, include_ares_trailer=False)` =
  `00 00 00 00` + 48 challenge bytes = **52 bytes**. Event `sent_challenge_length`,
  `challenge_mode: "raw"`. Every Initial gets a fresh Challenge.
- **Response (2):** `matches_challenge = pending[source] exists and response.timestamp ==
  stored timestamp (exact float equality) and response.cookie == stored cookie`. Event
  `matches_challenge`, `cookie_sha256_prefix = sha256(cookie)[:12]`. If it matches, send
  `wrap_raw_challenge(build_ack(response), 377, 4, False)` = **52 bytes**; event
  `sent_ack_length`. Every matching Response (the client retransmits) gets an ACK with fresh
  random padding. No age check on the timestamp. A non-matching Response gets nothing.
- `wrap_raw_challenge(payload, bits, 4, False)` (`H:428-435`) is just 4 zero bytes + payload.
  The 4-byte prefix is the reliability component's header as read by the client; the server-side
  reliability component reads 8 bytes, which is why C->S datagrams carry 8 (COMPATIBILITY.md,
  "Confirmed Challenge/ACK").
- Rust: timestamp may be any positive finite f64 that is stored and compared bit-exactly; cookie
  and padding come from a CSPRNG injectable for tests. These fields are random in Python, so
  datagram bytes are only reproducible with injected values (vectors 17.4).
- **(UE reference, unverified)** UE's `StatelessConnectHandlerComponent` derives the client's
  initial packet sequences from cookie bytes 0-3 (server) and 4-7 (client). Python does not
  depend on this: it derives the server sequence from the client's ACK field (section 11.1).

## 6. Ares packet layer

### 6.1 S->C datagram layout

Built identically by `build_server_bunch_frame` (`B:1606-1611`),
`build_server_session_ack_from_sequence` (`H:631-637`) and `build_server_control_probe`
(`H:689-694`). Input: decoded packet `D` of `P` bits (section 9/10).

```
X     = transform_short_packet(D, P, counter=0)                 T:72-121
blob  = wrap_gcm(X, "", P, receive_key_override)                H:402-425
      = u32le(0) || tag[0:4] || ciphertext(ceil(P/8) bytes) || suffix(1 byte)
frame = (int(blob) << 38) | (stateless_header << 32) | (1 << (110 + P))
datagram = frame.to_bytes((110 + P) // 8 + 1, "little")
```

| Bit range | Content |
|---|---|
| `[0, 32)` | 0 (4-byte reliability prefix) |
| `[32, 38)` | stateless header, echoed from the client datagram being answered (section 6.3) |
| `[38, 70)` | GCM counter, **always 0** (`H:419`) |
| `[70, 102)` | GCM tag, first 4 bytes of the 16-byte tag (`H:425`) |
| `[102, 102 + 8*ceil(P/8))` | ciphertext bytes |
| next `P%8` bits (or 8 bits if `P%8==0`) | rest of the 0xCC trailer (`suffix`) |
| `110 + P` | 1 (outer terminal bit) |

Datagram length = `(110 + P) // 8 + 1` bytes: P=77 -> 24, P=176 -> 36, P=448 -> 70 (all match
the live log).

**Trailer (`H:410-418`).** `tb = P & 7`. If `tb != 0`: before encryption the last plaintext byte
keeps its low `tb` bits and its high `8-tb` bits are set to `(0xCC << tb) & 0xFF`; `suffix =
0xCC >> (8 - tb)`. If `tb == 0`: plaintext unchanged, `suffix = 0xCC`. In the plaintext view
the 8 bits after the P packet bits are always 0xCC; on the wire the low `8-tb` of them are
encrypted inside the last ciphertext byte and only the high `tb` bits (the suffix) are clear.

**AES-GCM.** AES-256, key 32 bytes, **8-byte nonce** `u32le(counter) || u32le(direction)`, no
AAD, tag truncated to 4 bytes. S->C direction `0x7908CBDD` (`H:420-421`): nonce bytes
`00000000 ddcb0879`. Because the nonce is not 96 bits, `J0 = GHASH_H(nonce || 0^64 || 0^64 ||
u64be(64))` (NIST SP 800-38D). Python uses OpenSSL via `cryptography` (`H:404`, `H:423-424`);
the client uses OpenSSL EVP-GCM (COMPATIBILITY.md: handler near IDA `0x7FF76D4B7880`, GCM wrapper
RVA `0x2C67480`, outbound transform RVA `0x2C666B0`). Rust options: the `openssl` crate (set IV
length 8, tag length 4), or `aes-gcm` `AesGcm<Aes256, U8>` taking `tag[..4]` on encrypt and, on
decrypt, CTR-decrypt then recompute the full tag and compare 4 bytes. Vectors 17.3/17.5 pin it.

### 6.2 C->S datagram layout and decode (`H:445-473`)

```
reject if len < 19 or len(client_send_key) != 32 or last byte == 0
packet_bits = (len - 1) * 8 + bit_length(last byte) - 1     # index of the outer terminal bit
P = packet_bits - 64 - 6 - 72                               # 64 reliability, 6 stateless, 32 counter + 32 tag + 8 trailer
reject if P <= 0 or ceil(P/8) > 2048
counter = bits[70, 102);  tag = bits[102, 134) as 4 LE bytes
ciphertext = ceil(P/8) bytes starting at bit 134
plaintext = AES-256-GCM-decrypt(client_send_key, nonce = u32le(counter) || u32le(0x31ED27AB), tag(4))
            (InvalidTag -> reject)
decoded = inverse_short_packet(plaintext, P, counter)
return (decoded, P, counter, stateless_header = bits[64, 70))
```

| Bit range | Content |
|---|---|
| `[0, 64)` | 8-byte reliability prefix; **not read or checked** |
| `[64, 70)` | stateless header |
| `[70, 102)` | client GCM counter (increments by 1 per client datagram; observed values ~6e8..4e9) |
| `[102, 134)` | tag (4 bytes) |
| `[134, 134 + P)` | plaintext bits (ciphertext region is `ceil(P/8)` bytes and includes trailer bits) |
| `142 + P` | outer terminal bit |

The decoded bytes keep trailer residue: when `P%8 != 0` the high `8 - P%8` bits of the last
decoded byte are the low bits of 0xCC (e.g. real 77-bit packet `...9b9d`: high 3 bits of 0x9d are
`100b`). Python passes these bytes unchanged to every consumer; Rust must too, and must always
use `P` as the length. Client datagram sizes seen live: 28 bytes (P=77), 47 (P=232), 230
(P=1694 Login), up to 996 (`T:4-5`).

`inspect_client_ares_packet` (`H:541-557`) adds: `ares_inspection` (`"authenticated"` or
`"invalid_or_unverified"`), `ciphertext_length` (= len(decoded)), `counter`,
`client_packet_sequence = (u32le(decoded[0:4]) >> 18) & 0x3FFF`, `stateless_header`,
`payload_bits`, `control_prefix_hex = decoded[:12].hex()`, and, if `P >= 128`, the
`inspect_control_bunch` fields (or `bunch_parse_error`).

### 6.3 Stateless header (6 bits)

Read from each C->S datagram at bit 64; every S->C frame built while handling that datagram
puts the same value at bit 32. Builders require `0 <= value < 32` (`B:1544`, `H:627`).
**(UE reference)** It is SessionID/travel count (2 bits) | ClientID (3 bits) << 2 | handshake bit
(0) << 5, matching the first 6 handshake bits of 5.1. Live: value 12 with handshake `client_id` 3,
travel 0; value 4 and 8 on other connections.

## 7. Keys

Two 32-byte AES keys, both read once from the live client and never refreshed.

| Name in `run_probe` | Use | Handler offset |
|---|---|---|
| `client_send_key` | decrypt C->S (`decode_client_ares_packet`) | `+0x28 .. +0x48` |
| `receive_key_override` | encrypt S->C (`wrap_gcm` key) | `+0x48 .. +0x68` |

**When** (`H:1465-1482`): on the first datagram (any type, including the handshake Initial)
received while `live_client_pid` is set and `client_send_key is None`. The scan runs inside the
loop and blocks it (up to 30 s). On failure that datagram is dropped (`handler_state:
"not_ready"`) and the next datagram retries the full scan. On success, print `read-only Ares
handler=0x<addr> receive-key fingerprint=<sha256(receive_key)[:12]>` and handle the same
datagram normally (it is not dropped). Rust equivalent: section 3.1.

**What** (`K:42-82` `read_current_handler_keys(pid, handler_vtable, max_seconds=30.0)`):
- `handler_vtable = live_client_image_base + 0xA5F3BB0` (`H:1472-1473`; Ares packet-handler
  vtable in `.rdata`). Never fall back to a stale base (`H:1470-1471`).
- `OpenProcess(0x0410 = PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, FALSE, pid)`.
- Walk `VirtualQueryEx` from `0x10000` while `< 0x7FFFFFFF0000`, elapsed `< 30 s`, scanned
  `< 4096 MiB` (all three checked once per region, `K:52`; inside a region only the 30 s bound is
  re-checked per chunk, `K:63`, so the byte bound can be overshot by one region). Next address =
  `(BaseAddress or address) + RegionSize`; stop if `VirtualQueryEx` fails or the address does not
  advance (`K:54-58`). Scan regions with `State == MEM_COMMIT (0x1000)`,
  `Type == MEM_PRIVATE (0x20000)`, `!(Protect & 0x101)` (no PAGE_NOACCESS, no PAGE_GUARD) and
  `Protect & 0xEE` (any readable).
- Read each region in 1 MiB chunks starting at the query address (no overlap between chunks, so a
  needle straddling a chunk boundary is missed); a failed `ReadProcessMemory` skips the chunk and
  does not count toward the byte bound. Search every byte offset of the bytes actually read for
  the 8-byte little-endian `handler_vtable` value.
- For each hit (ascending address): re-read 112 bytes at the hit; if 112 bytes were read and the
  first 8 equal the needle, return `(hit, bytes[0x28:0x48], bytes[0x48:0x68])`. First valid hit
  wins.
- Nothing found -> `RuntimeError`.

Observed: all key lines in the logs under `runtime\` (171 at verification time) print fingerprint `107b74a379f0` (many
client processes, different handler addresses, e.g. `runtime\20261012T-pawn\stdout.txt:2`); no
other value was ever logged.
It matches none of `derive_client_receive_key("")`, `("NOSESSION")`, `("NOSESSIO")`
(`e63483d11a39`, `aedea0dd2949`, `752d3d7e8304`) nor the server-seed candidates in `K:90-112`.
Key bytes were never saved (project rule). The send-key fingerprint was never logged.

Second scan in production: the diagnostic `inspect_packet_notify` call before NMT_Challenge
(`H:2116-2126`) re-runs this whole scan (`inspect_live_packet_notify.py:16`) and then follows
pointers for `incoming_control_sequence` / `in_sequence`; only event fields
(`pre_control_sequence`, `pre_in_sequence` or `pre_state_error`) come out of it, and it is what
stalled event 461 for 3469 ms. Not ported (section 1 dead-path table, parity note 12); a shadow
diff must ignore those three keys.

Rust P1: implement the same read-only scan on a worker thread (or get both keys from the
`internal` oracle when it exists, PORT_PLAN_v2 P5). Keep the `not_ready` drop semantics.

## 8. Packet transform (`T`)

Applied to the decoded packet before encryption (S->C, counter 0) and inverted after decryption
(C->S, client counter). Bound: `0 < bits <= 8192` and `bits <= 8*len` (`T:74-75`, `T:126-127`);
this caps a whole S->C packet at 8192 bits.

### 8.1 PRNG

```
M32 = 2^32-1, M64 = 2^64-1, K = 0x2545F4914F6CDD1D
initial_state(counter):                                  T:52-61
  c  = counter & M32
  a  = (c + 0x024EDCC0) & M32                            # 38722752
  mf = a ^ (((a ^ (a >> 15)) >> 12) & M32)
  mf ^= ((c - 64) << 25) & M32                           # two's-complement: (c.wrapping_sub(64) << 25) as u32
  ms = c ^ ((c << 25) & M32)
  ms ^= ((c ^ (c >> 15)) >> 12) & M32
  return (K * mf) & M64, (K * ms) & M64, selector = c
advance_state(s1, s2):                                   T:64-69
  sum   = (s1 + s2) & M64
  mixed = s1 ^ s2
  s1'   = (mixed ^ (mixed << 14) ^ rotr64(s1, 9)) & M64
  s2'   = rotr64(mixed, 28)
  return s1', s2', selector = (sum >> 32) & M32
```

### 8.2 Forward (`transform_short_packet`, `T:72-121`)

Process the buffer in place from byte 0. After **each** unit, `advance_state`.

1. `bits // 64` wide units (u64 LE at the current offset):
   `v += rotr32(sel,1)` (mod 2^64; the addend is a zero-extended u32), `v = Swide(v)`,
   `v += rotr32(sel,3)`, `v = Swide(v)`, `v = R64(v - rotr32(sel,5))` (sub mod 2^64), `v = Swide(v)`.
2. Remaining `bits % 64`: if `>= 32`, one word unit (u32 LE), using **rotl**:
   `v += rotl32(sel,1)`, `Sword`, `v += rotl32(sel,3)`, `Sword`, `v = rev32(v - rotl32(sel,5))`,
   `Sword` (all mod 2^32).
3. Then `(rem % 32) // 8` byte units: `v = Sb[(v + 11*sel) & 0xFF]`, `v = Sb[(v + 51*sel) & 0xFF]`,
   `v = (v - 27*sel) & 0xFF`, `v = Sb[rev8(v)]`.
4. Then `rb = rem % 8`; if `rb`: `byte[off] ^= ((1 << rb) - 1) & (sel ^ 0xC0)` with the current
   selector (after the last advance; the initial selector `= counter` if no unit ran). Bits above
   `rb` are untouched.

`Swide/Sword` substitute every byte of the LE value through `wide_sbox` / `word_sbox`; `Sb` is
`byte_sbox` (tables in Appendix A). `rev32`/`rev8` are full bit reversals of 32/8 bits
(`T:33-34`). `R64 = reverse_wide_bits` (`T:37-43`): reverse the bits of each byte, swap the two
bytes of each 16-bit lane, then rotate right by 32. It is an involution and is used in both
directions.

### 8.3 Inverse (`inverse_short_packet`, `T:124-171`)

Same unit split and state advance. Wide: `v = Swide_inv(v)`, `v = R64(v) + rotr32(sel,5)`,
`v = Swide_inv(v)`, `v -= rotr32(sel,3)`, `v = Swide_inv(v)`, `v -= rotr32(sel,1)`. Word:
`v = Sword_inv(v)`, `v = rev32(v) + rotl32(sel,5)`, `Sword_inv`, `v -= rotl32(sel,3)`,
`Sword_inv`, `v -= rotl32(sel,1)`. Byte: `v = Sb_inv[x]`, `v = rev8(v)`, `v = (v + 27*sel) & 0xFF`,
`v = Sb_inv[v]`, `v = (v - 51*sel) & 0xFF`, `v = Sb_inv[v]`, `out = (v - 11*sel) & 0xFF`.
Remaining bits: identical XOR.

## 9. Packet header, packet info, ACK history

### 9.1 Decoded packet prefix (both directions)

| Bits | Width | Field | S->C value |
|---|---|---|---|
| 0 | 4 | history word count - 1 **(UE name)** | 0 (one 32-bit word) |
| 4 | 14 | acked sequence (latest packet sequence received from the peer) | `client_packet_sequence` of the datagram being answered |
| 18 | 14 | packet sequence (sender's own) | `next_server_sequences[source]` |
| 32 | 32 | ACK history mask | section 9.3 |
| 64 | 1 | has packet info | 1 |
| 65 | 10 | jitter clock delta **(UE name)** | 0 |
| 75 | 1 | server frame time flag **(UE name)** | 0 |
| 76.. | | bunches, then a final 1 bit (packet terminator) | |

Header word: `(server_sequence << 18) | (acked_client_sequence << 4)` (`B:1566`, `H:631`,
`H:661`). Live client packets set bit 75 = 1 (Hello bytes 8-11 `ff3f01c0`: jitter 1023; short
packet bytes 8-9 `9b9d`: jitter 717, bit 76 = terminator). Python's reader takes 1 + 10 + 1 bits
after the 64-bit header (`H:490-493`; the jitter is only skipped when bit 64 is set, the next bit
is always skipped). Never set bit 75 in S->C frames: the frame-time byte that UE would expect
after it is not implemented.

ACK-only packet (`H:632`): `u32le(header) || u32le(mask) || 01 10` = 77 bits: bit 64 = 1,
bits 65-75 = 0, bit 76 = 1 (terminator).

### 9.2 Sequences

- `client_packet_sequence = (u32le(decoded[0:4]) >> 18) & 0x3FFF` (`H:547`).
- Server sequence: set once at the Challenge (section 11.1), then `+1 & 0x3FFF` after every S->C
  frame on that source (`H:2140`, `H:2156`, `H:2169`, `H:2183`, `H:1585`, ...; every `sendto`
  in `H` except the two handshake sends `H:4560`, `H:4571` is followed by this increment).
- Python never reads the client's ACK field or history except to derive the Challenge sequence.
  No S->C packet loss detection, no retransmission (section 13.4).

### 9.3 update_received_packet_history (`H:560-576`)

```
history_mask = 0
for prev in recent (deque maxlen 64, oldest first):
    d = (current - prev) & 0x3FFF
    if 1 <= d <= 32: history_mask |= 1 << (d - 1)
recent.append(current)
return history_mask
```

Bit 0 means "current-1 was received". Called for every authenticated C->S packet (`H:1490-1492`),
before any send for that datagram. The mask is used by the session ACK and by every frame built
while handling the same datagram. Fixed masks: Challenge 3, Welcome 7 (`H:662`), Join ACK 3 (default
of `build_server_session_ack`, `H:613`, not passed by `H:607`). Unauthenticated datagram: 0 (unused).

## 10. Bunch framing

### 10.1 BunchHeader (`B:1505-1521`)

| Field | Default | Production use |
|---|---|---|
| `channel_index` | required | packed uint |
| `channel_name_index` | required | always 102 for actor channels, 255 for control (Challenge/Welcome copy it from Hello) |
| `channel_sequence` | 0 | 12-bit, reliable only |
| `is_open` / `is_close` | False | mutually exclusive |
| `is_reliable` | True | |
| `replication_paused` | False | always False in production |
| `has_exports` | False | per bunch |
| `has_must_map`, `is_partial`, `header_flag_4` | False | must stay False (`B:1552-1553` raises) |
| `partial_flags` | 0 | never read by the builder (`B:1517`); partial bunches are rejected |
| `close_reason` | 0 | `SerializeInt(reason, 15)`, only 0 (Destroyed) is ever sent (no caller sets it) |

### 10.2 build_server_bunch_frame (`B:1524-1611`)

Checks (`B:1540-1563`): sequences < 0x4000, mask <= 0xFFFFFFFF, stateless < 32, key 32 bytes,
`channel_index` and `channel_name_index` <= 0xFFFFFFFF, not (open and close), no partial/must-map/flag4, close must be reliable, `close_reason < 15`,
reliable needs `channel_sequence < 0x1000`, unreliable cannot open, `body_bit_count < 8192`.

Decoded layout:

```
32  (server_sequence << 18) | (acked_client_sequence << 4)
32  ack_history_mask
1   1        has packet info
10  0
1   0
-- reliable (is_reliable=True) --
1   1        bControl
1   is_open
1   is_close
    if is_close: SerializeInt(close_reason, 15)   (4 bits for 0)
1   replication_paused (0)
1   1        bReliable
8n  packed uint channel_index
1   has_exports
1   0        must-be-mapped GUIDs
1   0        partial
1   0        header flag 4
12  channel_sequence
1   1        hardcoded channel name follows
8n  packed uint channel_name_index
-- unreliable (is_reliable=False) --
1   0        bControl (no open/close bits)
1   replication_paused (0)
1   0        bReliable
8n  packed uint channel_index
1   has_exports
1   0
1   0
1   0        (no sequence, no name)
--
13  body_bit_count
N   body
1   1        packet terminator
```

Then section 6.1 framing. Returns `(datagram, decoded_bit_count)`. One bunch per packet; Python
never packs two bunches in one S->C packet.

### 10.3 Bit helpers (`B:21-72`)

- `append_packed_uint(v)` (`B:27-35`): `v < 2^32`; repeat: `part = v & 0x7F; v >>= 7;` write
  8 bits `(part << 1) | (v != 0)`; stop when `v == 0`. Always whole bytes; 0 -> `00`, 102 -> `cc`,
  255 -> `ff 02`.
- `append_serialize_int(v, max)` (`B:37-60`), UE `SerializeInt`: `0 <= v < max <= 2^32`;
  `nv = 0, mask = 1; while nv + mask < max and mask < 2^32: write bit (v & mask != 0); if set nv += mask;
  mask <<= 1`. Width depends on value and max: (0,15) -> 4 bits, (7,15) -> 3 bits.
- `append_payload(bytes, n)`: the low n bits of the LE integer.
- `append_fstring_ascii(s)`: int32 `len+1`, then ASCII bytes and a NUL (1..240 chars).

### 10.4 inspect_control_bunch (`H:476-538`), the C->S first-bunch reader

Requires `bit_count >= 128`. From bit 64: `has_packet_info`(1), if set 10 more bits, then 1 bit;
bunch at the next bit: `is_control`(1); if control: `is_open`(1), `is_close`(1); if close: return
`{bunch_start_bit, bunch_is_close: True}` (the close reason is not parsed); `paused`(1),
`reliable`(1), packed `channel_index` (max 5 bytes), `has_exports`, `has_must_map`, `is_partial`,
`header_flag_4` (1 each), if reliable: `channel_sequence`(12) (records its start bit), if partial:
2 bits, if `is_control or is_reliable`: hardcoded flag (1) and, if set, packed name index;
`bunch_bit_count`(13); `control_message_type` = next 8 bits if `bunch_bit_count >= 8`.
Any field past `bit_count` raises ValueError. Real client control bunches: Hello is a control,
open, reliable bunch on channel 0 (body starts at bit 135); Login and Join are **non-control**,
reliable bunches on channel 0 (body at bit 133).

## 11. Control channel (NMT) flow

Sequence on one source (production run 2026-10-14, events 458-481 of
`runtime\range_launch_23872_phase4_match_state.json`):

```
C: Initial (79 B)                        -> S: Challenge 52 B
C: Response (73 B, matches)              -> S: ACK 52 B
C: Hello (232 bits, ch0 seq 3558)        -> stored in hello_packets
C: 77-bit ACK-only (seq 15846, ack 11195)-> S: NMT_Challenge (36 B), server seq 11196, ch0 seq 3005
C: Hello resends, 77-bit packets         -> nothing
C: Login (1694 bits, NMT 5, ch0 seq 3559)-> S: NMT_Welcome (70 B), server seq 11197, ch0 seq 3006
C: 329 bits, first bunch = Hello resend -> nothing (Join's ch0 seq 3561 implies another
                                            reliable control bunch, 3560, that was not inspected)
C: Join (142 bits, NMT 9, ch0 seq 3561)  -> S: Join ACK (24 B), server seq 11198
C: any packet >= 0.1 s later             -> S: session ACK (seq 11199) + first bring-up stage (seq 11200)
```

### 11.1 NMT_Challenge (`H:2111-2142`, builder `H:640-700`)

Trigger: authenticated, `bits == 77`, `source in pending`, `source not in ack_sent_sources`,
`source in hello_packets`.

`build_server_control_probe(hello, 232, short, 77, stateless, K, message_type=3, ("",))`:
1. Validate the stored Hello: `hello_bit_count == 232`, first bunch `control_message_type == 0`,
   `channel_index == 0`, `bunch_is_control`, `bunch_reliable`, `bunch_end_bit == 231`.
2. `client_sequence = (u32le(short[0:4]) >> 18) & 0x3FFF`;
   `server_sequence = (((u32le(short[0:4]) >> 4) & 0x3FFF) + 1) & 0x3FFF` (client's acked
   sequence + 1).
3. Packet prefix: header word, mask 3, bit 64 = 1, bits 65-75 = 0.
4. Bunch header: copy Hello bits `[76, body_start - 13)` (46 bits for the real Hello), clear bit 1
   of the copy (`is_open`), replace the 12 bits at `channel_sequence_start_bit - 76` with
   `(server_sequence + 1) & 0xFFF`. For the real Hello this yields exactly control=1, open=0,
   close=0, paused=0, reliable=1, channel 0, flags 0, sequence, hardcoded=1, name 255, i.e. the
   same bits as `build_server_bunch_frame(body, 40, BunchHeader(0, 255, (S+1) & 0xFFF), mask=3)`
   (asserted in `test_range_server_bunch.py:481-500` and vector 17.6).
5. Body: `03` + int32 0 (empty FString) = 40 bits; 13-bit size; terminator.
6. Frame (section 6.1).

After sending (`H:2136-2142`): `next_server_sequences[source] = S + 1`,
`initial_actor_channel_sequences[source] = (S + 1) & 0xFFF`, add to `ack_sent_sources`. Event:
`sent_control_message_type: 3`, `sent_ack_probe_length`, `acked_client_sequence`, `server_sequence`.

Rust: implement the copy exactly (it is a few lines); also log a warning if the copied bits
differ from the canonical header above.

### 11.2 NMT_Welcome (`H:2143-2157`, builder `H:703-712`)

Trigger: `source in ack_sent_sources`, not in `welcome_sent_sources`, `source in pending`,
first-bunch `control_message_type == 5` (Login). Builder: same template copy from the stored
Hello, `server_sequence = next_server_sequences[source]` (override), client sequence from the
Login header, mask **7**, channel sequence `(server_sequence + 1) & 0xFFF`, body = `01` +
FString `"/Game/Maps/Poveglia/Range"` (int32 26, 25 ASCII bytes, NUL) + int32 0 + int32 0
= 312 bits. Packet = 448 bits, datagram 70 bytes. String bounds: <= 200 bytes, no NUL
(`H:674-675`). The builder re-inspects the Login and requires `control_message_type == 5`
(`H:706-707`). After: server sequence +1, add to `welcome_sent_sources`. Event:
`sent_control_message_type: 1`, `sent_welcome_probe_length`, `acked_client_sequence`,
`server_sequence` (`H:2152-2155`).

### 11.3 Join ACK (`H:2158-2171`, builder `H:598-608`)

Trigger: `source in welcome_sent_sources`, not in `join_ack_sent_sources`, `source in pending`,
first-bunch `control_message_type == 9`. Builder checks the first bunch is NMT 9 on channel 0 and
reliable, then sends an ACK-only packet (section 9.1) with `server_sequence =
next_server_sequences[source]`, the Join's client sequence, mask 3. 24 bytes. After: sequence +1,
add to `join_ack_sent_sources`, `last_session_ack_times[source] = now`.

Control-channel reliable sequences: Challenge `(S+1) & 0xFFF`, Welcome `(S+2) & 0xFFF` (since
Welcome's packet sequence is S+1); with the live S = 11196 that is 3005 and 3006. No further S->C
control bunches are sent.

## 12. Session tick, keepalive, heartbeat

**Session ACK** (`H:2172-2184`): trigger `source in join_ack_sent_sources`, `source in pending`,
`now - last_session_ack_times.get(source, 0) >= 0.1`. Send
`build_server_session_ack(decoded, P, stateless, K, next_server_sequences[source],
ack_history_mask)` (ACK-only, real history mask, client sequence of this datagram). Then
`next_server_sequences += 1`, `last_session_ack_times[source] = now` (taken after the send).
Event: `sent_session_ack_length` (24), `acked_client_sequence`, `server_sequence`.

Immediately after, in the same datagram:
1. Bring-up stage chain `H:2185-4407`: one `if/elif` chain of 56 branches; only the first branch
   whose `sent`/`not sent`/elapsed-time conditions hold runs on this tick. Each stage frame uses
   the same client sequence, stateless header and mask, and the next server sequence. Content and
   timing of the stages: `03_bringup_a.md` (`H:2185-3352`) and `04_bringup_b.md` (`H:3353-4407`).
2. Move-ACK block `H:4412-4535`, a separate `if` (runs even when a stage ran): gated by
   `move_ack_probe`, `source in client_restart_sent_sources`, `>= 0.05 s` since the last move ACK.
   Specified in `04_bringup_b.md` section 9.

A stage sends up to 3 frames: two in the PlayerState character-link stage (`H:2492-2527`, loop
at `H:2507`) and the item-slot scale stage (`H:2873-2968`, sends at `H:2935`, `H:2960`), three in
the starting-equippable stage (`H:4003-4101`, loop at `H:4085`), otherwise one (or none when a
stage's own sub-condition fails). A session tick therefore emits 1 to 5 frames, in this order:
session ACK, stage frames, move ACK. Live cadence: the idle client sends a 77-bit packet about
every 234 ms; every packet >= 0.1 s after the previous session ACK gets one (sample: events
481-499 in section 16).

**Keepalive.** Production has no server-initiated packets. Liveness of the UDP session rests on
the client's own traffic; the session ACK replies are what stopped the client's former 60-second
timeout (COMPATIBILITY.md, "Native control-channel progress").

**Heartbeat (dead).** `send_due_server_heartbeats` (`H:1416-1437`) would send ACK-only packets
from `latest_authenticated_packets` to peers with a Join ACK, last packet <= 5 s old, at
`VALORANT_RANGE_HEARTBEAT_HZ` (0..60). Off in production; keep off in parity mode.

## 13. Reliable sequencing

### 13.1 Base

`initial_actor_channel_sequences[source] = (S + 1) & 0xFFF`, S = Challenge packet sequence
(`H:2141`). Every actor channel's first reliable bunch uses `base + 0`. **(UE reference,
unverified)** consistent with UE `InitSequence`: the client's `InitInReliable` for all channels is
`S & (MAX_CHSEQUENCE-1)`, so it expects `S+1` first on every channel.

### 13.2 Fixed offsets

Bring-up stages write `channel_sequence=(initial_actor_channel_sequences[source] + k) & 0xFFF`
with literal k per stage (e.g. ch3/ch4/ch5 open `+0` at `H:2202`, `H:2233`, `H:2295`; ch4 `+1` at
`H:2395`). Full channel-sequence maps: `03_bringup_a.md`, `04_bringup_b.md` section 4.

### 13.3 Dynamic counter (`H:786-802`, `H:911-914`)

```
channel_first_dynamic_offset = {6: 11, 8: 51, 9: 5, 12: 2, 13: 5, 14: 2, 15: 2, 16: 2, 17: 2, 18: 2}   H:786
channel_first_dynamic_offset[4] = range_shop_sequence_offset(team_state_array_probe) + 1                H:911  -> 8
channel_first_dynamic_offset[5] = (7 if goto_state_probe else 6) + (1 if client_game_phase_begin_probe else 0)  H:913-914 -> 6
channel_next_offset = {}                                                                                 H:787

take_channel_sequence(source, ch):                                                                       H:789-793
    off = channel_next_offset.get((source, ch), channel_first_dynamic_offset[ch])
    channel_next_offset[(source, ch)] = off + 1
    return (initial_actor_channel_sequences[source] + off) & 0xFFF
```

- `channel_first_dynamic_offset` is **global** (not per source). Dynamic channels set
  `channel_first_dynamic_offset[ch] = 0` before their first send: purchases (`H:1197`, `H:1225`),
  smoke projectile/zone (`H:1038`, `H:1085`), bots/dummy (`H:2042`, `H:2056`, `H:1965`), command
  queue opens (`H:1974`, `setdefault`). The value is only consulted when the `(source, ch)` key is
  absent, so a reused channel index continues its counter (allocators never reuse indices:
  purchases from 20, bots and the dummy from 400, projectiles from 10000 up to 32766; starting
  equippables and abilities use the fixed channels 12-18 registered at `H:786`).
- `channel_first_dynamic_offset[ch]` = number of fixed-stage reliable bunches already sent on `ch`.
  A missing key raises KeyError in Python (channel not registered).
- `take_ch8/9/13_sequence` (`H:795-802`) are aliases. In production every take goes through
  `send_reliable` (`H:1575`): `take_ch9_sequence` is called only by dead paths
  (`base_pistol_skin_trial` `H:3277`, `RANGE_EQUIP_STATE_TRIAL` `H:4351`) and `take_ch8/13` have no
  callers. Fixed-offset stage bunches (13.2) never touch `channel_next_offset`.
- `range_shop_sequence_offset(True) = 7` and `game_state_completion_sequence_offsets(True) =
  (4, 5, 6)` (`H:243-257`; unit-tested `test_range_udp_handshake.py:113-119`).

### 13.4 send_reliable (`H:1571-1585`)

```
send_reliable(channel, body, bits, has_exports, is_open=False, is_close=False):
    frame = build_server_bunch_frame(body, bits,
        BunchHeader(channel, 102, take_channel_sequence(source, channel), is_reliable=True,
                    is_open, is_close, has_exports),
        server_sequence=next_server_sequences[source], acked_client_sequence=<this datagram>,
        ack_history_mask=<this datagram>, stateless_header=<this datagram>, key)
    sendto; next_server_sequences[source] += 1 (& 0x3FFF)
```

No retransmission, no ACK tracking of S->C packets, no partial bunches (body < 8192 bits, whole
packet <= 8192 bits). A lost S->C reliable bunch stalls that channel on the client (its later
reliable bunches wait in its queue). Loopback makes loss rare; PORT_PLAN_v2 P1 adds
retransmission, which is out of parity scope.

### 13.5 Close bunches

`send_reliable(ch, b"", 0, False, is_close=True)`: reliable, control, `is_close=1`, close reason
`SerializeInt(0, 15)` = 4 zero bits (`B:1576-1577`; client reads it right after bClose, IDA
`sub_7FF7393C14C0`, `B:1519-1520`), consumes the channel's next reliable sequence, empty body.
Sites: smoke flight `H:1092`, smoke zone `H:1096`, the item a purchase replaces in its slot
`H:1275` (a previous purchase from `purchased_slot_items`, or, once per connection, the starting
Classic on ch9 when a bought sidearm takes its slot, `H:1267-1272`; that close takes the next ch9
sequence from `take_channel_sequence(source, 9)`), dead bot `H:2049`. Channel 0 is never closed.

## 14. Per-connection state

| State | Python | Set | Updated / read |
|---|---|---|---|
| `client_send_key`, `receive_key_override` | `H:767-768` (global) | first datagram (section 7) | never refreshed |
| `pending[src] = (timestamp, cookie)` | `H:773` | each Initial | read by Response; membership gates all session branches; never removed |
| `hello_packets[src] = (decoded, bits, counter, stateless)` | `H:774` | each authenticated 232-bit packet from a pending src | template for Challenge and Welcome |
| `ack_sent_sources` | `H:804` | Challenge sent | |
| `welcome_sent_sources` | `H:805` | Welcome sent | |
| `join_ack_sent_sources` | `H:806` | Join ACK sent | |
| `next_server_sequences[src]` | `H:775` | Challenge: S+1 | +1 per S->C frame |
| `initial_actor_channel_sequences[src]` | `H:776` | Challenge: (S+1) & 0xFFF | constant |
| `channel_next_offset[(src, ch)]` | `H:787` | first `take_channel_sequence` | +1 per take |
| `channel_first_dynamic_offset[ch]` | `H:786`, `H:911-914` (global) | startup; dynamic opens set 0 | |
| `recent_client_sequences[src]` deque(64) | `H:1362` | every authenticated packet | history mask |
| `last_session_ack_times[src]` | `H:1408` | Join ACK | each session ACK (heartbeat too) |
| `latest_authenticated_packets[src]` | `H:1363` | every authenticated packet | heartbeat only (dead) |
| `events` deque(8192), `total_event_count` | `H:771-772` (global) | every recorded event | exit summary |

Everything else per source (stage sets, NetGUID allocators, mirrors) belongs to the bring-up,
authority-loop and client-decode docs (`03`-`06`). Nothing in this table is ever removed (no
`pop`/`discard`/`del` on any of it in `H`). A new client connection uses a new source port and
gets fresh state. A same-port re-handshake is not supported: the stateless Challenge and ACK are
sent again (`pending[source]` is overwritten) and the new Hello overwrites `hello_packets`, but
`ack_sent_sources`/`welcome_sent_sources`/`join_ack_sent_sources` stay set, so no NMT_Challenge,
Welcome or Join ACK is sent; once Join ACK was sent before, session ACKs (and stages/authority
work) continue with the old `next_server_sequences` and channel base.

## 15. Parity notes (preserve unless the user decides otherwise)

1. S->C GCM counter is always 0, so every S->C packet reuses one nonce. Preserve.
2. S->C transform counter is always 0. Preserve.
3. History-mask bit 0 = "current-1 received" (section 9.3). Preserve (see Q2).
4. Challenge/Welcome/Join ACK use fixed masks 3/7/3.
5. A 232-bit authenticated packet from a pending source is always stored as Hello and skips that
   datagram's session tick.
6. Only the first bunch of a C->S packet is inspected for control messages.
7. A 77-bit packet before any Hello is dropped and its event is not recorded.
8. Decoded C->S bytes keep 0xCC trailer residue above P.
9. C->S bytes 0-7 are ignored.
10. Every S->C frame carries the stateless header, client sequence and mask of the datagram that
    triggered it.
11. Python crash paths (uncaught ValueError in builders, `P < 32` / `P > 8192` client packets,
    overwritten Hello template, WSAEMSGSIZE, output file exists; section 4 notes): Rust logs and
    continues instead. Not wire behaviour.
12. The pre-Challenge diagnostic memory scan stalled the live handler 3469 ms (event 461,
    `handler_elapsed_ms`); Rust omits it, so the Challenge goes out earlier. Timing only.

## 16. Scenario test (live run, 2026-10-14)

Source: `runtime\range_launch_23872_phase4_match_state.json` (keeps the last 8192 of 9466
events; "Event" = index in that array), source port 59690, stateless 12. A Rust shadow test can
feed the decoded packets (bits, header) in order and assert the same decisions:

| Event | t (s) | C->S | S->C |
|---|---|---|---|
| 0-457 | 42.5-57.7 | port 54029, stateless 8, authenticated, not in `pending` (the client's connection to the previous responder process) | nothing |
| 458 | 59.859 | Initial, 79 B, packet_count 1, client_id 3 | Challenge 52 B |
| 459 | 59.890 | Response, 73 B, matches | ACK 52 B |
| 460 | 59.922 | 232 bits, Hello, counter 617901793, seq 15845, prefix `b0bb96f700000000ff3f01c0` | stored |
| 461 | 60.156 | 77 bits `b0bb9af7000000009b9d` (seq 15846, ack 11195) | NMT_Challenge 36 B, S=11196 |
| 462-476 | 63.625 | 77-bit packets, 3 Hello resends | nothing |
| 477 | 63.625 | 1694 bits, Login, ch0 seq 3559, seq 15862 | Welcome 70 B, seq 11197 |
| 478 | 63.656 | 329 bits, first bunch Hello resend | nothing |
| 479 | 64.031 | 142 bits Join, ch0 seq 3561, seq 15864, prefix `d0bbe2f703000000a34c0048` | Join ACK 24 B, seq 11198 |
| 480 | 64.078 | 77 bits (0.047 s after Join ACK) | nothing |
| 481 | 64.312 | 77 bits seq 15866 | session ACK seq 11199 + ch3 open (142 B, seq 11200, ch seq 3005) |
| 482 | 64.531 | 77 bits seq 15867 | session ACK seq 11201 |
| 490 | 66.406 | 77 bits seq 15875 | session ACK 11209 + ch4 open (ch seq 3005, seq 11210) |
| 492 | 66.859 | 77 bits (0.094 s after previous ACK) | nothing |

## 17. Test vectors

All vectors were produced offline by importing the Python builders (generator: Appendix B). Hex is byte order on the wire / in memory. `bits` is the meaningful bit count. Unless stated otherwise: server key `K_s = 000102...1f` (stand-in for `receive_key_override`), client key `K_c = 202122...3f` (stand-in for `client_send_key`), stateless header 12 (real value from the 2026-10-14 run), `os.urandom` padding forced to `a0 a1 a2 a3 a4 a5 a6 a7`.

### 17.1 Transform PRNG state (range_udp_transform.py:52-69)

| counter | state_first | state_second | selector0 | next 4 selectors |
|---|---|---|---|---|
| 0 | `a0d43df0852cf619` | `0000000000000000` | `00000000` | `a0d43df0 f647d83f dfe96f1d 7b2bed83` |
| 1 | `58ef6fa4fbc018fc` | `47e4ce4b896cdd1d` | `00000001` | `a0d43df0 b4286905 51a4144d 633fb3e3` |
| 617901793 | `8b2511422ba1b26d` | `4aceca0a58c9c077` | `24d46ee1` | `d5f3db4c 388376c6 903fa2d1 a4431d99` |
| 4294967295 | `9c1409c251e8eb4a` | `5d028085065ec683` | `ffffffff` | `f9168a47 416c334a 49c4e254 a24e7b2a` |

`reverse_wide_bits` (range_udp_transform.py:37-43):

| in (u64) | out (u64) |
|---|---|
| `0123456789abcdef` | `d591f7b3c480e6a2` |
| `0000000000000001` | `0000800000000000` |
| `8000000000000000` | `0000000000010000` |
| `00000000000000ff` | `0000ff0000000000` |

### 17.2 transform_short_packet / inverse_short_packet (range_udp_transform.py:72-171)

Input payload is `bytes((i*37+19) & 0xFF for i in range(ceil(bits/8)))`. `inverse_short_packet(out, bits, counter) == in` for every row. Bytes above `bits` in the last byte pass through unchanged.

| bits | counter | in | out |
|---|---|---|---|
| 1 | 0 | `13` | `13` |
| 1 | 617901794 | `13` | `13` |
| 7 | 0 | `13` | `53` |
| 7 | 617901794 | `13` | `31` |
| 8 | 0 | `13` | `a4` |
| 8 | 617901794 | `13` | `39` |
| 13 | 0 | `1338` | `a428` |
| 13 | 617901794 | `1338` | `3938` |
| 31 | 0 | `13385d82` | `a4e9ffdf` |
| 31 | 617901794 | `13385d82` | `39ad36d3` |
| 32 | 0 | `13385d82` | `d201b46b` |
| 32 | 617901794 | `13385d82` | `395ab7ba` |
| 39 | 0 | `13385d82a7` | `d201b46b97` |
| 39 | 617901794 | `13385d82a7` | `395ab7baa7` |
| 64 | 0 | `13385d82a7ccf116` | `3c168e51bad3b622` |
| 64 | 617901794 | `13385d82a7ccf116` | `3c168e51c2757254` |
| 77 | 0 | `13385d82a7ccf1163b60` | `3c168e51bad3b622447f` |
| 77 | 617901794 | `13385d82a7ccf1163b60` | `3c168e51c2757254967b` |
| 96 | 0 | `13385d82a7ccf1163b6085aa` | `3c168e51bad3b622b62b554c` |
| 96 | 617901794 | `13385d82a7ccf1163b6085aa` | `3c168e51c27572546e6cae8f` |
| 135 | 0 | `13385d82a7ccf1163b6085aacff4193e63` | `3c168e51bad3b622243a6bfb4a89dc881c` |
| 135 | 617901794 | `13385d82a7ccf1163b6085aacff4193e63` | `3c168e51c275725424256bfb1526318058` |
| 176 | 0 | `13385d82a7ccf1163b6085aacff4193e6388add2f71c` | `3c168e51bad3b622243a6bfb4a89dc8880a9b94be20c` |
| 176 | 617901794 | `13385d82a7ccf1163b6085aacff4193e6388add2f71c` | `3c168e51c275725424256bfb152631808be5b0aa07fa` |
| 232 | 0 | `13385d82a7ccf1163b6085aacff4193e6388add2f71c41668bb0d5fa1f` | `3c168e51bad3b622243a6bfb4a89dc886d3e66f00bad047d206f69cedd` |
| 232 | 617901794 | `13385d82a7ccf1163b6085aacff4193e6388add2f71c41668bb0d5fa1f` | `3c168e51c275725424256bfb152631806d9366f097f2f5b551ab2a711d` |
| 448 | 0 | `13385d82a7ccf1163b6085aacff4193e6388add2f71c41668bb0d5fa1f44698eb3d8fd22476c91b6db00254a6f94b9de03284d7297bce106` | `3c168e51bad3b622243a6bfb4a89dc886d3e66f00bad047d4a7594b8d6dbd1cd86d762008db6db7887ad534387ebabb52d2f5f0dc22b2804` |
| 448 | 617901794 | `13385d82a7ccf1163b6085aacff4193e6388add2f71c41668bb0d5fa1f44698eb3d8fd22476c91b6db00254a6f94b9de03284d7297bce106` | `3c168e51c275725424256bfb152631806d9366f097f2f5b54ae294b8f065f47c86d762007016841e873a5343a470dcf92d955f0dbaadcdb4` |

### 17.3 wrap_gcm (range_udp_handshake.py:402-425), key K_s, counter 0

Output layout: `u32le counter(0) | tag[0:4] | ciphertext | suffix`. This is the blob before the 38-bit shift.

| bits | in | out | len |
|---|---|---|---|
| 1 | `05` | `00000000edaee85ee101` | 10 |
| 8 | `05` | `00000000a69b1b677dcc` | 10 |
| 13 | `052a` | `00000000c2b370a97dad19` | 11 |
| 77 | `052a4f7499bee3082d52` | `000000008afb88717d0d5ba6f8cd4a00b8c719` | 19 |

`derive_client_receive_key` (dead in production, range_udp_handshake.py:386-399): '' -> `2eee56027ebc20b3d3479fb75f3b0ff09883df45f33d8668bc27acd47d77e1a0`, 'NOSESSION' -> `b256615b764f8bf776e7acb547c774dc406471d2409eecf1eb05d320592e13e4`

### 17.4 Stateless handshake (range_udp_handshake.py:285-442)

Captured Initial datagram (test fixture, test_range_udp_handshake.py:122-126), 81 bytes:
```
0000000000000000b00102000909abea74000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000099a6483897177280b6f2d7f1cd846fed01
```
parse_handshake ->
```
{"travel_count": 0, "client_id": 4, "minimum_version": 3, "version": 4, "packet_type": 0, "packet_count": 18, "network_version": 3923072530, "runtime_features": 0, "secret_id": 0, "timestamp": 0.0, "cookie": "0000000000000000000000000000000000000000"}
```
build_challenge(packet, timestamp=10.0, cookie=00 01 .. 13), 377 bits, 48 bytes:
```
b00182000909abea7400000000000000002440000102030405060708090a0b0c0d0e0f10111213a0a1a2a3a4a5a6a701
```
wrap_raw_challenge(challenge, 377, 4, False) = Challenge datagram, 52 bytes:
```
00000000b00182000909abea7400000000000000002440000102030405060708090a0b0c0d0e0f10111213a0a1a2a3a4a5a6a701
```
Synthetic Response datagram (client layout, type 2, packet_count 2, timestamp 10.0, same cookie, 9 padding bytes b0..b8 + terminal bit):
```
0000000000000000b00102010109abea7400000000000000002440000102030405060708090a0b0c0d0e0f10111213b0b1b2b3b4b5b6b7b801
```
parse_handshake ->
```
{"travel_count": 0, "client_id": 4, "minimum_version": 3, "version": 4, "packet_type": 2, "packet_count": 2, "network_version": 3923072530, "runtime_features": 0, "secret_id": 0, "timestamp": 10.0, "cookie": "000102030405060708090a0b0c0d0e0f10111213"}
```
build_ack(response) (377 bits) then wrap_raw_challenge(ack, 377, 4, False) = ACK datagram, 52 bytes:
```
00000000b00182010109abea740080000000000000f0bf000102030405060708090a0b0c0d0e0f10111213a0a1a2a3a4a5a6a701
```

### 17.5 Client datagram decode (decode_client_ares_packet, range_udp_handshake.py:445-473; inspect_* 476-557)

The decoded payloads are real (first 96 bits copied from the 2026-10-14 production log `runtime/range_launch_23872_phase4_match_state.json`, remaining header bits rebuilt from the logged inspected fields; Hello body after the type byte and the Login body are synthetic). The datagrams are synthetic: they were produced by the inverse of the decoder (transform with the counter, AES-GCM nonce `u32le(counter)||u32le(0x31ED27AB)`, 0xCC trailer, 8 zero prefix bytes, 6-bit stateless header 12) and verified to decode with the Python decoder. Key K_c.

**short77**: counter 617901794, decoded bits 77, datagram 28 bytes
```
datagram: 00000000000000008cb81b3589ac0387be3f12cc2e289d14c2b17e0e
decoded:  b0bb9af7000000009b9d
```
inspect_client_ares_packet (subset):
```
{"client_packet_sequence": 15846, "stateless_header": 12, "payload_bits": 77, "ciphertext_length": 10}
```
**hello232**: counter 617901793, decoded bits 232, datagram 47 bytes
```
datagram: 00000000000000004cb81b3549bac2ebf718c0b46fc8fe589c02c75ec0b51e72840d04b76e1ba69a7196efb8b60273
decoded:  b0bb96f700000000ff3f01c0bcff0b8001000000000000000000000080
```
inspect_client_ares_packet (subset):
```
{"client_packet_sequence": 15845, "stateless_header": 12, "payload_bits": 232, "ciphertext_length": 29, "bunch_is_control": true, "bunch_is_open": true, "bunch_reliable": true, "channel_index": 0, "channel_sequence": 3558, "channel_sequence_start_bit": 93, "channel_name_index": 255, "bunch_bit_count": 96, "bunch_body_start_bit": 135, "bunch_end_bit": 231, "control_message_type": 0}
```
**login142_synthetic**: counter 617901810, decoded bits 142, datagram 36 bytes
```
datagram: 00000000000000008cbc1b35c9d1b95181e67330f39190e0f67215b75f9047e6155ade1c
decoded:  b0bbdaf7000000006d490038efff0208a020
```
inspect_client_ares_packet (subset):
```
{"client_packet_sequence": 15862, "stateless_header": 12, "payload_bits": 142, "ciphertext_length": 18, "bunch_is_control": false, "bunch_is_open": false, "bunch_reliable": true, "channel_index": 0, "channel_sequence": 3559, "channel_sequence_start_bit": 91, "channel_name_index": 255, "bunch_bit_count": 8, "bunch_body_start_bit": 133, "bunch_end_bit": 141, "control_message_type": 5}
```
**join142**: counter 617901812, decoded bits 142, datagram 36 bytes
```
datagram: 00000000000000000cbd1b35492a3aa66d74a0141be89f5927a79f8e6a78fd6630f2d81c
decoded:  d0bbe2f703000000a34c0048efff02082021
```
inspect_client_ares_packet (subset):
```
{"client_packet_sequence": 15864, "stateless_header": 12, "payload_bits": 142, "ciphertext_length": 18, "bunch_is_control": false, "bunch_is_open": false, "bunch_reliable": true, "channel_index": 0, "channel_sequence": 3561, "channel_sequence_start_bit": 91, "channel_name_index": 255, "bunch_bit_count": 8, "bunch_body_start_bit": 133, "bunch_end_bit": 141, "control_message_type": 9}
```

### 17.6 Control-channel replies (range_udp_handshake.py:598-712)

Inputs: Hello = `hello232` above; short packet = `short77`; Login = `login142_synthetic`; Join = `join142`; key K_s; stateless 12.
`opened` shows the layers recovered from the frame: GCM tag, transformed plaintext, decoded plaintext (transform inverted with counter 0).

**challenge**: server_sequence 11196, acked_client_sequence 15846, plaintext bits 176, datagram 36 bytes
```
datagram:    000000000c00000080f4b20c37aa06b1808cc698bbd8cf26c1f4a582b034d13bd9351173
tag:         d2cb32dc
transformed: d03dd0d05369cae6f76aa1db1da7af36c007587c90bc
decoded:     60def3ae03000000011001a077ff0ba0800100000080
```
**welcome**: server_sequence 11197, acked_client_sequence 15862, plaintext bits 448, datagram 70 bytes
```
datagram:    000000000c000000805efd000caa0db140b9c618ba18bc02c1f4e5aef0a492cc4d3ae98996cdf811ae4cbc2929e4fa9a90b4605ccda7a9aae9270f14920bac182a09c2dd0273
tag:         7af50330
transformed: d011d0d08469cae0f7a530db1da71e368109852fae5c98c23f3b43c6783dda252e9d7c627f732ca99d56d0120c6b1521d0b281d0ba059ec1
decoded:     60dff7ae07000000011001c077ff0be084000d00008097a3b0b6b297a630b8b917a837bbb233b6b4b017a930b7b332000000000000000080
```
**join_ack**: server_sequence 11198, acked_client_sequence 15864, plaintext bits 77, datagram 24 bytes
```
datagram:    000000000c00000000da656e1aaa06b140b9fc98748e760e
tag:         6897b969
transformed: d03dd0d08481cadaac8f
decoded:     80dffbae030000000110
```
Challenge equals `build_server_bunch_frame(03 00000000, 40, BunchHeader(0, 255, 3005), server_sequence=11196, acked_client_sequence=15846, ack_history_mask=3, stateless_header=12)`: **True**.

### 17.7 Session ACK (build_server_session_ack_from_sequence, range_udp_handshake.py:622-637)

| case | client seq | server seq | mask | datagram (24 bytes) | decoded (77 bits) |
|---|---|---|---|---|---|
| first_after_join | 15866 | 11199 | `00000001` | `000000000c000000c05a425a15ea1ab140b9f4d87f8e760e` | `a0dfffae010000000110` |
| full_history | 15895 | 11225 | `ffffffff` | `000000000c000000c00505c2c023f47809db4bc37d8e760e` | `70e167afffffffff0110` |
| wrap | 16383 | 16383 | `00000003` | `000000000c00000040e26e150faa06b180658bd77f8e760e` | `f0ffffff030000000110` |

build_server_session_ack(short77, 77, 12, K_s, 11199, ack_history_mask=1) = `000000000c00000000ba211313ea1ab1808cc6d87f8e760e` (client seq 15846 taken from the decoded header).

### 17.8 build_server_bunch_frame (range_server_bunch.py:1524-1611)

Common args: server_sequence 11230, acked_client_sequence 15903, ack_history_mask 0x7FFFFFFF, stateless 12, key K_s.

**reliable_open_exports_ch3**: channel_index=3, channel_name_index=102, channel_sequence=3005, is_open=1, is_close=0, is_reliable=1, has_exports=1, close_reason=0; body `00` / 8 bits; packet bits 136; datagram 31 bytes
```
decoded:     f0e17bafffffff7f01300da27733230080
transformed: f7f7cbf70d5ea4ec2e6ad00b9fa74af291
tag:         1f18651b
datagram:    000000000c000000c00746d9c623f477095b4b03f9ee8f3a75d4e5bbc12073
```
**reliable_ch4_seq3006**: channel_index=4, channel_name_index=102, channel_sequence=3006, is_open=0, is_close=0, is_reliable=1, has_exports=0, close_reason=0; body `2d` / 6 bits; packet bits 134; datagram 31 bytes
```
decoded:     f0e17bafffffff7f011011c077331b8036
transformed: f7f7cbf70d5ea4ec2ea581ea1da71e2709
tag:         9eb129e0
datagram:    000000000c00000080676c0af823f477095b4b03f92efc6ecdf4e5eef4c61c
```
**unreliable_ch8_20bits**: channel_index=8, channel_name_index=102, channel_sequence=0, is_open=0, is_close=0, is_reliable=0, has_exports=0, close_reason=0; body `a5c30f` / 20 bits; packet bits 125; datagram 30 bytes
```
decoded:     f0e17bafffffff7f010008a000a5c31f
transformed: f7f7cbf70d5ea4ec28d76c728c486d8b
tag:         8f8fc43f
datagram:    000000000c000000c0e323f1cf23f477095b4b0379afa055ab101ef25f0e
```
**close_ch400**: channel_index=400, channel_name_index=102, channel_sequence=3006, is_open=0, is_close=1, is_reliable=1, has_exports=0, close_reason=0; body `(empty)` / 0 bits; packet bits 140; datagram 32 bytes
```
decoded:     f0e17bafffffff7f015030c4007c37330008
transformed: f7f7cbf70d5ea4ec094c2e9f8da775cb8cc5
tag:         857968dd
datagram:    000000000c00000040611e5af723f477095b4b0339670605d0d025f48fa72107
```
**reliable_open_ch10000**: channel_index=10000, channel_name_index=102, channel_sequence=4095, is_open=1, is_close=0, is_reliable=1, has_exports=1, close_reason=0; body `0102030405` / 37 bits; packet bits 173; datagram 36 bytes
```
decoded:     f0e17bafffffff7f01304338e3ff3397800081018212
transformed: f7f7cbf70d5ea4ecf7422f2e9fa77bc9ab087c7c5991
tag:         b508d8dc
datagram:    000000000c000000402d0236f723f477095b4b03b9d845457cd4a5774feed23299475a0e
```

### 17.9 BitWriter helpers (range_server_bunch.py:21-60)

| append_packed_uint(v) | bytes | bits |
|---|---|---|
| 0 | `00` | 8 |
| 1 | `02` | 8 |
| 102 | `cc` | 8 |
| 127 | `fe` | 8 |
| 128 | `0102` | 16 |
| 255 | `ff02` | 16 |
| 400 | `2106` | 16 |
| 3714 | `053a` | 16 |
| 10000 | `219c` | 16 |
| 4294967295 | `ffffffff1e` | 40 |

| append_serialize_int(value, max) | bits written |
|---|---|
| (0,15) | 4 |
| (7,15) | 3 |
| (14,15) | 4 |
| (44,179) | 8 |
| (25,54) | 5 |
| (0,2) | 1 |
| (1,2) | 1 |

### 17.10 update_received_packet_history (range_udp_handshake.py:560-576)

One deque (maxlen 64), sequences fed in this order -> returned mask:

| seq | mask |
|---|---|
| 100 | `00000000` |
| 101 | `00000001` |
| 104 | `0000000c` |
| 136 | `80000000` |
| 137 | `00000001` |
| 16383 | `00000000` |
| 0 | `00000001` |
| 2 | `00000006` |

### 17.11 Production channel_first_dynamic_offset after startup (range_udp_handshake.py:786, 911-914)

```
{"6": 11, "8": 51, "9": 5, "12": 2, "13": 5, "14": 2, "15": 2, "16": 2, "17": 2, "18": 2, "4": 8, "5": 6}
```

## 18. Open questions

- **Q1 Key provenance.** Both keys are read from client memory once per process. The receive-key
  fingerprint has been `107b74a379f0` in every logged run, so the key looks constant for this
  build/config, but its derivation is unknown and the bytes were never stored. Options: keep the
  read-only scan in Rust P1, or have the `internal` oracle hand the keys over. Storing the key in
  config needs the user's decision.
- **Q2 ACK-history bit meaning.** **(UE reference, unverified)** UE `FNetPacketNotify` treats
  history bit 0 as the acked sequence itself; Python sets bit 0 for `current-1`. For a continuous
  stream all low bits are set, so the client sees no loss; on gaps the client may mark the wrong
  packet as lost and resend reliable data (the responder dedups purchases and settings by channel
  sequence, `H:1864-1867`, `H:1934-1937`). Verify in IDA before changing; parity keeps Python's
  form.
- **Q3 Fixed S->C GCM counter.** The client accepts counter 0 on every packet. Unknown whether a
  monotonically increasing counter would also be accepted; untested, keep 0.
- **Q4 Lifetime.** Python exits after 3600 s and the keepalive restart cannot resume the client's
  session, so a Range session ends after one hour. Recommendation: no lifetime limit in Rust.
- **Q5 Cookie and sequences.** Whether the client derives its initial packet sequences from the
  cookie (UE `GetChallengeSequence`) is unverified for this build. Python's derivation from the
  client's ACK field works regardless; keep it.
- **Q6 Hello swallowing (parity note 5).** Harmless so far; a later fix would change which
  datagram gets the session ACK.
- **Q7 Tick-native sending.** With the 128 tps loop the server could ACK or replicate per tick
  instead of per client datagram. That is a behaviour change, so it waits until parity is proven
  (PORT_PLAN_v2 step 4, shadow mode).

## Appendix A. Transform tables (range_udp_transform.py:13-18)

256 bytes each, hex. SHA-256 of the raw bytes given for verification.

`byte_sbox` sha256 `0ff37434be2510f37185b10b4c065b41eea255d16494586abc957b74a83b0974`
```
d6f0de79a73956bf07df008caa3e109a891e4625a84595c0234136815db07b49dacc2a1ce7a466967348cfb86f1fca8f3cf8aba322e61bb4ea0a43f5c1b1788db3acd74d98a1804476bc55fe125131ffa5db4e4ab9900fc3e92706b219d5ad9175c687f3142d24d164027d5f018b1126c9746d9f29ef86345aae5efdd4d0676e9c6016099913831a18a088c49d7ef67a3dbd94cd323003cee8151d3f82bb358e0bfa3a59afd82c5bed47ebd24b0d4f4c33f9ee0885d3fb9e20ec69e468e35352f2b5e09221a993f73b4204f16bc5c8b658e1409717575c2f61d9c2a205dd62f4289b2e546c63fc386ae5a6ba72dc3777717c7fbe0e50c784e20c2b65708ab7cb
```
`byte_sbox_inverse` sha256 `9f8191299695d99e910479397a403827f91e9d1432b9f6fbf3ba02000346387a`
```
0a6c6996cadc5a08b38339a0f9adf4560e6e4c85649982d4885c8736239a112db8c4341866136f59e07422faa665e2d7954e94b0779e1aeee705a2c830900d9bd219c93a471512a9291f53acaf4352aef54dbfbee34a06d5d0a378a7d61c7a6b81d8dee568fb267ebcbae8cce4727f2cfcf0ec28716048ef3e038f1ef16a8df2461b9c86f7b476628a10fd6d0b3f9f2f555fc3c6921627d344840fe1808cb7738945db332550ea0414c50c32415e79a41d3d5b4037c1cffe2b54eb9d4991f307173cda578bcd61f6ce702eff2193972a7d67abb57c5d0042a5d92051eddd0209c2d1f8bdbbe93524985838aab9a8b27501cbc063df3b8ec731b1a1b6e67b4b4f
```
`word_sbox` sha256 `ba0394eb8a2b3e04d869736f589253df124108125a46731d022ec2fc0f1bf9a8`
```
d66b8be962da0957af47f899d97f5a8e61453d792f160bfa6cdde729c8556fba4d00dfb8741536f32e69e40aeb8c24a1d204c99d7c1a176e222babcea86dfb05d0124e87ef9f1b9246827271d543f7a9e50ebdca51d47a9e41bce1a59abfea1dff9b2ab3c7705c01493c5650685373a3193eb1b03a52cf97c180b7114bde9cccaebbf128149321bed83f917e8d3b856477a4837b271303d3675f31e210fc30a65b3725c618b91e0f54267dedfe6094f41c658f02c5c30d44b6480620d776c4e690c2e01f9859894ae83386883542f2a739ec2d079608f65838758a5dcdee34dc95db6aaaade3a081fdf9b2f5d1acf066cb0c322c4c4f23405e84c0b4b5a27863
```
`word_sbox_inverse` sha256 `361a89e4017c9609939a6b7230beba40254b5af5e143d435506fadf7bab68294`
```
2167b396313fbad3d5062b16f1b651a79c7b419584251536a4703546b05fa6c3bb8638f62ea2a994831b6239f3d228149e9af2c9decc26a1d8d0748d69127189f758cd4db7114809b968c77cf42042f56b54756da81d6a07d7c50ea066dbf899ad1004ff8fb1ef986c29e201183d371e654b4a6e24d9bd90fe13569334aa8b0d79e74992f98eca43cbc6da022d8c0fb2c08a4785aee0d477c40b5c617e335745e62ffd6f915b9fcf3c4fe33aede480087372ea63fbfcb87a23a51f815952875dfa78c1b5beb4a3641c3253f07fdc3b7640ec3097554c00bc880c05e1df197d22c25a9be52a50bf1ac8035e2cd1abdd44ee82ce27afebd64e0ae9173e9de8ac60
```
`wide_sbox` sha256 `62c5d207912a333fd19d0bccbfa9dc8f0cb41a6db75cc18d0efd248fdb72dd83`
```
ecddd123022f1ec9868aa7e18931578ed4cba179d9dcd51df554c4482a96973963bc99878f37da06269c85e0a2138103126136ba84780cb5b1943598fdaf800bb39ef224085c7b423a5edf8b92e8e4d773c3752e9193a8f4eec8386eea161c6f414aadff4b0e6674de18289df7e63ec02cbf9a09c67db9001f69aa77d605f1e26759c25ab43ff6fcd8fb454ffe7e2990c158aea31aedd3a615400afaa55babb01b6d341920e3a033a9149f22cd52d2b8e9ccb67cac55ef3c5301ca43db626456473b4d8debc565602507a4469550f82d72685d30b7f0ce6a4ef99bf3830f1770113d887f44276bd0e55f4904e7cf8cbd4c712bb2820d21107ac7bbbe6c517632
```
`wide_sbox_inverse` sha256 `3747e5b8a551c4a3918afe9e824eef433719ea170072e0bc203a6bc59f731633`
```
77b9042feb7d27c944739a3f36f565ddf7e0302da9985dde69a394a05e170678a4f6ab0343c828e56a8e1cf270cf5305d30dffa7a23a32255a1f48c1b7e16e85996047bbe48acbc01bea6164f0c2d88bcdfdadb819b5bf0e9181839d45d249e9c731bd20bec66680d179d7e6fca15b5fdff1d0506752fe7b3513f846b3758de33e2ef4dc342a0823e20c094beec30f248f544c5539cc1d1e3b2272da296b41aaa6122c93ca9c970a56a87a9eb462923d9f38f3408437b2d4af7633fa21effb716f9082511ac574f95907ba11b1acd6ede702ae9610167c4f881426bc1501684a2b0b7fa54ee86dec4db05cc4009558b6d57e42db5718866cced99b89873c8c63
```

## Appendix B. Vector generator (offline, scratch only)

Run from any directory with Python 3.12 and `cryptography`; it imports the builders from `local_backend`, monkeypatches `range_udp_handshake.os.urandom`, and prints JSON. No sockets, no process access.

```python
"""Offline test-vector generator for the transport spec (no sockets, no live process)."""
import hashlib
import json
import struct
import sys
from collections import deque

sys.path.insert(0, r"C:\Users\Cayden\Desktop\valorant proj\local_backend")

import range_udp_handshake as h
import range_udp_transform as t
import range_server_bunch as b
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes

out = {}
KEY = bytes(range(32))           # server->client (receive_key_override stand-in)
CKEY = bytes(range(32, 64))      # client->server (client_send_key stand-in)


def det_urandom(n):
    return bytes((0xA0 + i) & 0xFF for i in range(n))


h.os.urandom = det_urandom


def payload_for(bits, salt=19):
    return bytes((i * 37 + salt) & 0xFF for i in range((bits + 7) // 8))


def open_server_frame(frame, key):
    total = int.from_bytes(frame, "little")
    terminal = total.bit_length() - 1
    P = terminal - 32 - 6 - 64 - 8
    assert h.read_bits(frame, 0, 32) == 0
    stateless = h.read_bits(frame, 32, 6)
    counter = h.read_bits(frame, 38, 32)
    tag = h.read_bits(frame, 70, 32).to_bytes(4, "little")
    n = (P + 7) // 8
    ct = bytes(h.read_bits(frame, 102 + 8 * i, 8) for i in range(n))
    dec = Cipher(algorithms.AES(key), modes.GCM(struct.pack("<II", counter, 0x7908CBDD), tag, min_tag_length=4)).decryptor()
    pt = dec.update(ct) + dec.finalize()
    trailer_low = None
    if P & 7:
        trailer_low = pt[-1] >> (P & 7)
    decoded = bytearray(t.inverse_short_packet(pt, P, counter))
    if P & 7:
        decoded[-1] &= (1 << (P & 7)) - 1
    return dict(stateless_header=stateless, counter=counter, tag=tag.hex(), plaintext_bits=P,
                transformed_plain_hex=pt.hex(), decoded_hex=bytes(decoded).hex(),
                trailer_low_bits_in_plaintext=trailer_low)


# ---------------- transform ----------------
tr = {}
tr["initial_state"] = {}
for c in (0, 1, 617901793, 0xFFFFFFFF):
    s1, s2, sel = t.initial_state(c)
    chain = []
    a, bb = s1, s2
    for _ in range(4):
        a, bb, s = t.advance_state(a, bb)
        chain.append(f"{s:08x}")
    tr["initial_state"][str(c)] = {"state_first": f"{s1:016x}", "state_second": f"{s2:016x}",
                                   "selector0": f"{sel:08x}", "next_selectors": chain}
tr["reverse_wide_bits"] = {f"{v:016x}": f"{t.reverse_wide_bits(v):016x}" for v in
                           (0x0123456789ABCDEF, 1, 0x8000000000000000, 0xFF)}
tr["vectors"] = []
for bits in (1, 7, 8, 13, 31, 32, 39, 64, 77, 96, 135, 176, 232, 448):
    for counter in (0, 617901794):
        p = payload_for(bits)
        enc = t.transform_short_packet(p, bits, counter)
        assert t.inverse_short_packet(enc, bits, counter) == p
        tr["vectors"].append({"bits": bits, "counter": counter, "in": p.hex(), "out": enc.hex()})
tr["sbox_sha256"] = {name: hashlib.sha256(getattr(t, name)).hexdigest() for name in
                     ("byte_sbox", "byte_sbox_inverse", "word_sbox", "word_sbox_inverse", "wide_sbox", "wide_sbox_inverse")}
out["transform"] = tr

# ---------------- gcm ----------------
g = {}
g["derive_client_receive_key"] = {repr(k): h.derive_client_receive_key(k).hex() for k in ("", "NOSESSION")}
g["wrap_gcm"] = []
for bits in (1, 8, 13, 77):
    p = payload_for(bits, 5)
    frame = h.wrap_gcm(p, "", bits, KEY)
    g["wrap_gcm"].append({"bits": bits, "in": p.hex(), "key": KEY.hex(), "out": frame.hex(), "out_len": len(frame)})
out["gcm"] = g

# ---------------- handshake ----------------
hs = {}
initial = bytes.fromhex(
    "0000000000000000b00102000909abea74000000000000000000000000000000"
    "0000000000000000000000000000000000000000000000000000000000000000"
    "99a6483897177280b6f2d7f1cd846fed01")
pk = h.parse_handshake(initial)
hs["initial_packet_hex"] = initial.hex()
hs["parsed"] = {k: (v.hex() if isinstance(v, bytes) else v) for k, v in pk.__dict__.items()}
ch = h.build_challenge(pk, 10.0, bytes(range(20)))
hs["challenge_bits"] = 377
hs["challenge_payload_hex"] = ch.hex()
hs["challenge_payload_len"] = len(ch)
raw = h.wrap_raw_challenge(ch, 377, 4, False)
hs["challenge_datagram_hex"] = raw.hex()
hs["challenge_datagram_len"] = len(raw)
resp = h.HandshakePacket(pk.travel_count, pk.client_id, pk.minimum_version, pk.version, 2, 2,
                         pk.network_version, pk.runtime_features, 0, 10.0, bytes(range(20)))
ack = h.build_ack(resp)
hs["ack_payload_hex"] = ack.hex()
ackraw = h.wrap_raw_challenge(ack, 377, 4, False)
hs["ack_datagram_hex"] = ackraw.hex()
hs["ack_datagram_len"] = len(ackraw)
hs["note"] = "os.urandom(8) padding replaced with a0 a1 a2 a3 a4 a5 a6 a7"
# synthetic response packet bytes: 8 zero bytes + same layout as challenge with type 2
w = b.BitWriter()
for v, wd in ((pk.travel_count, 2), (pk.client_id, 3), (1, 1), (0, 1), (pk.minimum_version, 8), (pk.version, 8),
              (2, 8), (2, 8), (pk.network_version, 32), (pk.runtime_features, 16), (0, 1),
              (int.from_bytes(struct.pack("<d", 10.0), "little"), 64), (int.from_bytes(bytes(range(20)), "little"), 160)):
    w.append_bits(v, wd)
for byte in range(0xB0, 0xB0 + 9):
    w.append_bits(byte, 8)
w.append_bits(1, 1)
resp_bytes = bytes(8) + w.to_bytes()
rp = h.parse_handshake(resp_bytes)
hs["synthetic_response_hex"] = resp_bytes.hex()
hs["synthetic_response_parsed"] = {k: (v.hex() if isinstance(v, bytes) else v) for k, v in rp.__dict__.items()}
hs["synthetic_response_ack_datagram_hex"] = h.wrap_raw_challenge(h.build_ack(rp), 377, 4, False).hex()
out["handshake"] = hs

# ---------------- client packets (synthetic encryption of real decoded prefixes) -------------


def client_encode(decoded, bits, counter, stateless, key):
    transformed = t.transform_short_packet(decoded, bits, counter)
    trailer = 0xCC
    tb = bits & 7
    pt = bytearray(transformed)
    if tb:
        pt[-1] &= (1 << tb) - 1
        pt[-1] |= (trailer << tb) & 0xFF
        suffix = bytes([trailer >> (8 - tb)])
    else:
        suffix = bytes([trailer])
    enc = Cipher(algorithms.AES(key), modes.GCM(struct.pack("<II", counter, 0x31ED27AB))).encryptor()
    ct = enc.update(bytes(pt)) + enc.finalize()
    blob = struct.pack("<I", counter) + enc.tag[:4] + ct + suffix
    terminal = 64 + 6 + 64 + bits + 8
    v = (int.from_bytes(blob, "little") << 70) | (stateless << 64) | (1 << terminal)
    return v.to_bytes((terminal + 8) // 8, "little")


def bits_from_fields(fields):
    v, pos = 0, 0
    for val, wd in fields:
        assert 0 <= val < (1 << wd), (val, wd)
        v |= val << pos
        pos += wd
    return v, pos


# Real 77-bit short packet (event 461 of runtime/range_launch_23872_phase4_match_state.json)
short77 = bytes.fromhex("b0bb9af7000000009b9d")
# Hello: real first 96 bits, rest rebuilt from inspected fields; body after type byte is zero (synthetic)
hello_prefix = int.from_bytes(bytes.fromhex("b0bb96f700000000ff3f01c0"), "little")
hello_fields = [(hello_prefix & ((1 << 76) - 1), 76), (1, 1), (1, 1), (0, 1), (0, 1), (1, 1), (0, 8), (0, 4),
                (3558, 12), (1, 1), (0xFF, 8), (2, 8), (96, 13), (0, 96), (1, 1)]
hv, hbits = bits_from_fields(hello_fields)
hello = hv.to_bytes((hbits + 7) // 8, "little")
assert hbits == 232 and hello[:12].hex() == "b0bb96f700000000ff3f01c0", hello[:12].hex()
login_prefix = int.from_bytes(bytes.fromhex("b0bbdaf7000000006d490038"), "little")
login_fields = [(login_prefix & ((1 << 76) - 1), 76), (0, 1), (0, 1), (1, 1), (0, 8), (0, 4), (3559, 12), (1, 1),
                (0xFF, 8), (2, 8), (8, 13), (5, 8), (1, 1)]
lv, lbits = bits_from_fields(login_fields)
login = lv.to_bytes((lbits + 7) // 8, "little")
assert login[:12].hex() == "b0bbdaf7000000006d490038", login[:12].hex()
join_prefix = int.from_bytes(bytes.fromhex("d0bbe2f703000000a34c0048"), "little")
join_fields = [(join_prefix & ((1 << 76) - 1), 76), (0, 1), (0, 1), (1, 1), (0, 8), (0, 4), (3561, 12), (1, 1),
               (0xFF, 8), (2, 8), (8, 13), (9, 8), (1, 1)]
jv, jbits = bits_from_fields(join_fields)
join = jv.to_bytes((jbits + 7) // 8, "little")
assert jbits == 142 and join[:12].hex() == "d0bbe2f703000000a34c0048", join[:12].hex()

cp = {}
for name, dec, bits, counter in (("short77", short77, 77, 617901794), ("hello232", hello, 232, 617901793),
                                 ("login142_synthetic", login, lbits, 617901810), ("join142", join, 142, 617901812)):
    frame = client_encode(dec, bits, counter, 12, CKEY)
    res = h.decode_client_ares_packet(frame, CKEY)
    assert res is not None and res[0] == dec and res[1] == bits and res[2] == counter and res[3] == 12
    insp = h.inspect_client_ares_packet(frame, CKEY)
    cp[name] = {"decoded_hex": dec.hex(), "bits": bits, "counter": counter, "client_send_key": CKEY.hex(),
                "datagram_hex": frame.hex(), "datagram_len": len(frame),
                "inspect": {k: v for k, v in insp.items()}}
out["client_packets"] = cp

# ---------------- control-channel replies ----------------
ctl = {}
fr, cs, ss = h.build_server_challenge_probe(hello, 232, short77, 77, 12, KEY)
ctl["challenge"] = {"frame_hex": fr.hex(), "len": len(fr), "acked_client_sequence": cs, "server_sequence": ss,
                    "opened": open_server_frame(fr, KEY)}
fr, cs, ss = h.build_server_welcome_probe(hello, 232, login, lbits, 12, KEY, 11197)
ctl["welcome"] = {"frame_hex": fr.hex(), "len": len(fr), "acked_client_sequence": cs, "server_sequence": ss,
                  "opened": open_server_frame(fr, KEY)}
fr, cs, ss = h.build_server_join_ack(join, 142, 12, KEY, 11198)
ctl["join_ack"] = {"frame_hex": fr.hex(), "len": len(fr), "acked_client_sequence": cs, "server_sequence": ss,
                   "opened": open_server_frame(fr, KEY)}
# equivalence: challenge == generic frame builder with canonical header
eq_fr, eq_bits = b.build_server_bunch_frame(b"\x03" + struct.pack("<i", 0), 40,
                                            b.BunchHeader(0, 255, (11196 + 1) & 0xFFF),
                                            server_sequence=11196, acked_client_sequence=15846,
                                            ack_history_mask=3, stateless_header=12, receive_key=KEY)
ctl["challenge_equals_generic_builder"] = eq_fr.hex() == ctl["challenge"]["frame_hex"]
ctl["inspect_hello"] = h.inspect_control_bunch(hello, 232)
ctl["inspect_login"] = h.inspect_control_bunch(login, lbits)
ctl["inspect_join"] = h.inspect_control_bunch(join, 142)
out["control"] = ctl

# ---------------- session acks ----------------
sa = {}
for name, cseq, sseq, mask in (("first_after_join", 15866, 11199, 0x00000001), ("full_history", 15895, 11225, 0xFFFFFFFF),
                               ("wrap", 0x3FFF, 0x3FFF, 0x00000003)):
    fr, cs, ss = h.build_server_session_ack_from_sequence(cseq, 12, KEY, sseq, mask)
    sa[name] = {"client_sequence": cseq, "server_sequence": sseq, "mask": f"{mask:08x}", "stateless_header": 12,
                "frame_hex": fr.hex(), "len": len(fr), "opened": open_server_frame(fr, KEY)}
fr2, _, _ = h.build_server_session_ack(short77, 77, 12, KEY, 11199, 1)
sa["from_decoded_short77"] = {"frame_hex": fr2.hex(), "server_sequence": 11199, "mask": "00000001"}
out["session_ack"] = sa

# ---------------- generic bunch frames ----------------
bf = {}
cases = {
    "reliable_open_exports_ch3": (b"\x00", 8, b.BunchHeader(channel_index=3, channel_name_index=102, channel_sequence=3005,
                                                            is_open=True, has_exports=True)),
    "reliable_ch4_seq3006": (bytes.fromhex("2d"), 6, b.BunchHeader(channel_index=4, channel_name_index=102, channel_sequence=3006)),
    "unreliable_ch8_20bits": (bytes.fromhex("a5c30f"), 20, b.BunchHeader(channel_index=8, channel_name_index=102, is_reliable=False)),
    "close_ch400": (b"", 0, b.BunchHeader(channel_index=400, channel_name_index=102, channel_sequence=3006, is_close=True)),
    "reliable_open_ch10000": (bytes.fromhex("0102030405"), 37, b.BunchHeader(channel_index=10000, channel_name_index=102,
                                                                            channel_sequence=0xFFF, is_open=True, has_exports=True)),
}
for name, (body, bits, hdr) in cases.items():
    fr, pbits = b.build_server_bunch_frame(body, bits, hdr, server_sequence=11230, acked_client_sequence=15903,
                                           ack_history_mask=0x7FFFFFFF, stateless_header=12, receive_key=KEY)
    bf[name] = {"header": {k: (v if not isinstance(v, bool) else int(v)) for k, v in hdr.__dict__.items()},
                "body_hex": body.hex(), "body_bits": bits, "packet_bits": pbits, "frame_hex": fr.hex(), "len": len(fr),
                "opened": open_server_frame(fr, KEY)}
out["bunch_frames"] = bf

# ---------------- bit helpers ----------------
bh = {"packed_uint": {}, "serialize_int": {}}
for v in (0, 1, 102, 127, 128, 255, 400, 3714, 10000, 0xFFFFFFFF):
    w = b.BitWriter(); w.append_packed_uint(v)
    bh["packed_uint"][str(v)] = {"hex": w.to_bytes().hex(), "bits": w.bit_count}
for v, mx in ((0, 15), (7, 15), (14, 15), (44, 179), (25, 54), (0, 2), (1, 2)):
    w = b.BitWriter(); n = w.append_serialize_int(v, mx)
    bh["serialize_int"][f"{v},{mx}"] = {"bits": n, "value": w.value}
out["bits"] = bh

# ---------------- ack history ----------------
dq = deque(maxlen=64)
hist = []
for s in (100, 101, 104, 136, 137, 0x3FFF, 0, 2):
    hist.append([s, f"{h.update_received_packet_history(dq, s):08x}"])
out["ack_history_sequence"] = hist

# ---------------- channel offsets (production) ----------------
offs = {6: 11, 8: 51, 9: 5, 12: 2, 13: 5, 14: 2, 15: 2, 16: 2, 17: 2, 18: 2}
offs[4] = h.range_shop_sequence_offset(True) + 1
offs[5] = 6 + 0
out["channel_first_dynamic_offset_production"] = offs
out["game_state_completion_sequence_offsets_true"] = h.game_state_completion_sequence_offsets(True)

print(json.dumps(out, indent=1, default=str))
```

## Verifier corrections (2026-10-14)

An adversarial pass re-read `H`, `B`, `T`, `K`, `L`, `PS` and the 2026-10-14 run log. It re-ran
the Appendix B generator offline (Python 3.12.3). Every hex vector in section 17 and every
Appendix A table matched byte for byte, and `test_range_udp_handshake` passed (21 tests). No
bit width, offset, direction constant, mask, sequence rule or vector was wrong. Changes:

1. Section 1: the session-ACK branch requires `session_ack_probe` directly (`H:2172`); the
   other three branches use `or`. Added the `main()` exclusivity check (`H:4776-4777`).
2. Section 2: added `check_project_client` (`L:110-120`, `L:192`) to the launcher
   preconditions. Recorded that the clock is `GetTickCount64` (verified with
   `time.get_clock_info`) and how tick quantisation changes each time gate.
3. Section 3.1: Rust must treat `ConnectionReset` (WSAECONNRESET from ICMP) as "keep draining".
   Added the tick-overrun policy (no catch-up bursts), highest thread priority, and an
   injectable clock so replay tests reproduce Python's gate decisions.
4. Section 3.2: console allocation must call `FreeConsole` before `AllocConsole` when the console
   window is missing or hidden (marked as Windows API behaviour). Added ANSI colour
   (`DESIGN.md`) and a `SetConsoleCtrlHandler` flush on close.
5. Section 4: listed the exact `parse_error` strings and three more uncaught Python crash
   paths: `P < 32`, `P > 8192`, and an overwritten Hello template. Parity note 11 updated.
6. Section 7: corrected the scan-bound wording. The byte bound is checked per region, the time
   bound per region and per chunk, and a failed read skips its chunk. Documented the second full
   scan run by the diagnostic `inspect_packet_notify`, whose keys a shadow diff must ignore. The
   fingerprint line count is now 171.
7. Section 10: added the `partial_flags` field and the channel index/name range checks.
8. Section 11.2: added the Welcome event keys and the builder's Login re-check.
9. Section 13.3: production takes channel sequences only through `send_reliable`. The
   `take_ch9_sequence` callers are dead paths and `take_ch8/13` have no callers.
10. Section 13.5: the replaced-item close can close the starting Classic on ch9
    (`H:1267-1272`), not only a purchased item.
11. Section 14: per-connection state is never removed. Rewrote the same-port re-handshake
    behaviour: the stateless Challenge/ACK are re-sent, but no NMT reply is, and session ACKs
    continue on the old sequences.
