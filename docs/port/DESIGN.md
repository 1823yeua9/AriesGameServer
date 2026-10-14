# Aries game server: Rust design

The Rust server replaces `local_backend/range_udp_handshake.py` (the "responder") for the
production Range. Platform services (`backend.py`) and the loader are ported separately by
codex (`server/platform`, `loader/`). The Python stays untouched; a snapshot is in
`_preport_backup/local_backend_20261014`.

## Ground rules

- **Parity first.** Every byte we put on the wire must match what the Python sends for the
  same input. Every builder is golden-tested against fixtures exported from the Python
  (`server/tools/export_golden.py`, `server/fixtures/*.json`).
- **Production path only.** The Python has ~110 flags; production uses a fixed set
  (`docs/port/*.md` lists it). Dead probes are not ported.
- **Never guess** a width, handle, index, offset, GUID or channel: take it from the Python
  (which took it from IDA/live data) and keep the source reference next to it.
- **128 Hz fixed tick** (7.8125 ms), like retail servers. High-resolution waitable timer plus
  a short spin; the tick thread runs at highest priority.
- **Console + log file.** The server allocates a console if it was started hidden and logs
  there in color and to `logs/aries_server_<pid>.log`.
- **Style.** snake_case functions and variables, PascalCase types, SCREAMING_SNAKE consts,
  short lowercase comments that say why.

## Crates (`server/crates/*`)

| crate | contents | python source |
|---|---|---|
| `bits` | lsb-first BitWriter/BitReader (sticky error), packed ints, SerializeInt, FString, quantized vectors, compressed rotators | `range_server_bunch.BitWriter`, quantizers |
| `net` | s-boxes, packet transform, 4-byte-tag AES-GCM, server framing, client decode, stateless handshake, packet header + ack history, reliable channels (sequence, ack tracking, retransmit) | `range_udp_transform.py`, `range_udp_handshake.py` transport, `build_server_bunch_frame` |
| `rep` | content transform, content blocks, NetGUID exports, actor headers, and every bunch builder (pure functions: inputs -> `Bits`) | `range_server_bunch.py`, `range_*.py` builders |
| `oracle` | reads of the live client (ReadProcessMemory now, the in-process `internal` DLL over a named pipe later) behind one trait; snapshot thread | `read_*.py`, `resolve_rpc_index.py`, `read_fname_pool.py`, `capture_input_event_cursor.py`, `check_live_packet_keys.py` |
| `interp` | Source-SDK-style interpolated history, extrapolation clamp, lag compensation | (new; `docs/port/10_interp_source_sdk.md`) |
| `world` | per-connection model: channels and actors, NetGUID allocation, equippables, state machines and the mirror policy, resources, bots, projectiles | `range_equippables.py`, `range_gun_loader.py`, `range_bots.py`, `range_projectiles.py` state |
| `range` | the Range game mode: bring-up stage graph, per-tick systems (mirror, input correction, purchases, settings, streaming, HUD, bots, abilities, projectiles, move acks), client RPC decode | `run_probe` |
| `aries-server` | binary: config, process discovery, console + logging, socket, 128 Hz loop | `launch_range.py`, `main()` |

## Runtime

```
main thread (128 Hz, highest priority)          key-scan thread
  drain udp socket (non-blocking)                 retries the ares handler scan
  per datagram, in the python's order:            every 100 ms until it is found
    resend lost reliables, input ack,
    authority block, capture, dispatch
  per tick: lag-compensation records, replay
    datagrams held for the keys
  flush outgoing datagrams
  every 1 s: stop when the client process exits
  wait for the next tick
```

- Systems stay datagram-driven like the Python (every gate compares the same seconds
  thresholds against the datagram's time), so their timing matches it; the 128 Hz tick adds
  only what the Python did not have (lag-compensation records, held-datagram replay).
- Oracle reads are synchronous ReadProcessMemory calls inside the datagram, as in the Python
  (each is microseconds). A datagram slower than one tick logs a warning, and one over 25 ms
  gets `handler_elapsed_ms` like the Python.
- Bring-up keeps the Python's pacing (one stage per >= 0.1 s session-ack tick, same order,
  same gates). Stages are an enum with a gate table, not an `elif` chain.
- Reliable bunches get real tracking: packet-level ack history from the client and
  retransmission of lost reliables with their original channel sequence. The Python never
  retransmits; loopback rarely drops, so this only matters when it does (switch `retransmit`).
- One bunch per datagram, like the Python.

## Decided deviations from the Python

Everything else is meant to match it bit for bit (bodies, bunch headers, per-channel order,
gates, event keys and values).

| What | Python | Rust | Why |
|---|---|---|---|
| lost reliable bunches | never resent | resent with the original channel sequence | real reliability; switch `retransmit` |
| key scan | blocks the receive loop per datagram | background thread; datagrams wait and replay in order, and one is dropped (not_ready) only when a scan that started after it fails | the tick never blocks; same result |
| skin load error at A21 | raises out of the stage | plain Classic open, `base_pistol_initial_cosmetic_error` | keep the session |
| ClientSetHUD index read fails | raises | 52/178 (every logged session), `hud_rpc_error` | keep the session |
| lifetime | 3600 s deadline | none; exits when the client exits | the loader owns restarts |
| range re-entry | one responder served several entries; keys read once per process (the same in every logged run) | the same, with the oracle's address caches cleared when a handshake arrives from a new client socket | the actors of a new entry are new objects; python's caches could name a stale address (05 quirk 9) |
| shots | hit-tested at the bots' current position | rewound by lag compensation | bots never move, so every hit is identical (tested) |
| reader reopen after a failure | every pass | at most once a second (`equippable_reader_error` still logged every pass) | no handle churn |
| `pre_control_sequence`, `pre_in_sequence` | diagnostic packet-notify read at the nmt challenge | not logged | diagnostic only |
| console | prints every event to a redirected stdout | its own visible console (a hidden or missing one is replaced), levels: errors warn, authority work info, high-rate keys debug; every event still goes to the jsonl file | the user watches the log |

## Configuration

`aries-server.toml` (next to the exe) replaces the flags; every production feature is on by
default and can be turned off for A/B tests. CLI: `--pid`, `--image-base`, `--port`,
`--config`. Without `--pid` the server finds the single `VALORANT-Win64-Shipping.exe`, checks it
is the project client, and reads its image base.

## Verification

1. Golden fixtures per builder (bit-exact).
2. Stage-table tests: the bring-up order and gates match `docs/port/03_bringup_a.md` and
   `04_bringup_b.md`.
3. A loopback harness (`server/crates/range/tests`): a scripted client and scripted live
   reads drive handshake, bring-up, purchases, mirror, bots, cloudburst and abilities through
   `RangeServer`, decoding every bunch and checking it against the spec vectors.
4. Shadow diff (`server/tools/shadow_diff.py`): a harness or live events log against a
   Python responder log. Against the production log every bring-up frame length matches.
5. Live test with the user (needs their go-ahead; never relaunch a live session unasked).
