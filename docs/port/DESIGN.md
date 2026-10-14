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
main thread (128 Hz, highest priority)          oracle thread
  drain udp socket (non-blocking)                 polls the client at its own cadence
  decode + route client packets                   publishes Arc<Snapshot> (latest wins)
  range systems step (uses latest snapshot)       on-demand queries via channel
  net: retransmit unacked reliables
  flush outgoing datagrams
  wait for the next tick
```

- The server never blocks the tick on the oracle. Oracle data is a timestamped snapshot; the
  `interp` history smooths and rewinds it when we need a value at a specific time (shots).
- Bring-up keeps the Python's pacing (one stage per >= 0.1 s, same order, same gates) so
  timing-sensitive steps (HUD, cinematic, inventory) behave identically. Stages are a table:
  `{ id, prerequisites, gate, build }`, not an `elif` chain.
- Reliable bunches get real tracking: per-channel sequence, packet-level ack history from the
  client, retransmission of unacked reliables with their original channel sequence. The Python
  never retransmits; loopback rarely drops, so this only matters when it does.
- One bunch per datagram, like the Python. Batching is a later optimization.

## Configuration

`aries-server.toml` (next to the exe) replaces the flags; every production feature is on by
default and can be turned off for A/B tests. CLI: `--pid`, `--image-base`, `--port`,
`--config`. Without `--pid` the server finds the single `VALORANT-Win64-Shipping.exe`, checks it
is the project client, and reads its image base.

## Verification

1. Golden fixtures per builder (bit-exact).
2. Stage-table tests: the bring-up order and gates match `docs/port/03_bringup_a.md` and
   `04_bringup_b.md`.
3. A loopback harness: a scripted client drives the handshake and bring-up against the Rust
   server, decoding every bunch it receives.
4. Live test with the user (needs their go-ahead; never relaunch a live session unasked).
