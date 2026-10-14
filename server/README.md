# server

Cargo workspace for the Aries game server, the Rust port of
`local_backend/range_udp_handshake.py` (its production flag set). `platform/` is codex's
backend port and is excluded from this workspace (own lock file and target dir).

## build and run

```
cargo build --release -p aries-server
target/release/aries-server.exe --config aries-server.toml
```

- The server allocates its own console when started hidden and logs there and to
  `logs/aries_server_<pid>.log`. Per-datagram events (python key names, for shadow
  diffs) go to `logs/aries_events_<pid>.jsonl`.
- It finds the single running `VALORANT-Win64-Shipping.exe` itself (or use `--pid` and
  `--image-base`), reads the session keys from the client's ares packet handler
  (read-only), and serves `127.0.0.1:58100` at 128 ticks per second.
- It exits when the client process exits (the python's `live_client_exited`), so the
  port is free for the next range entry.
- `--port N` overrides the port (useful to run beside the python responder).
- The python responder must not be running on the same port.

## config

`aries-server.toml` holds every setting with its production default. Paths under
`[range]` are relative to the config file. `[range.features]` switches each system;
`[range.lagcomp]` sets lag compensation (source sdk values; `view_interp` is measured,
see `docs/port/10_interp_source_sdk.md` 4.13).

## for the loader

| loader setting | value |
|---|---|
| `gameplay_path` | `server/target/release/aries-server.exe` |
| `gameplay_config` | `server/aries-server.toml` |

The loader should also pass `--skin-catalog` and `--skin-state` with the same files it
gives the platform (`runtime/account/skins.json`), so bought guns and the starting
Classic/knife open with the skins the player equipped through the platform. Without them
the server reads the paths in `aries-server.toml` (the python's `local_backend` files).

Start it after the game process exists (it needs the pid for the key scan). It scans for
the client's ares handler in the background; datagrams that arrive meanwhile wait and are
handled in order once the keys are read (one is dropped as `not_ready` only when a scan
that started after it fails, like the python's blocking scan). It can stay up for the
whole game session: each range entry (a handshake from a new client socket) starts a
fresh session with the new handler's keys.

## what runs per datagram

Same order as the python (`docs/port/05_authority_loop.md` section 2):

1. stateless handshake, then decode and ack history (lost reliable bunches are resent
   with their original channel sequence; new, switch `retransmit`)
2. input ack (event ids parsed off the packet's own pawn bunch)
3. after ClientRestart, the authority block: equippable mirror (+ cloudburst throws and
   shots on firing-seed changes), input-correction responder, range hud, level
   streaming, ultimate points, store purchases, range settings, command queue, hud
   refresh, cloudburst flights, bots, ability jobs
4. movement packet capture (event `decoded_packet_hex`)
5. nmt challenge / welcome / join ack, then the 0.1 s session-ack tick: one bring-up
   stage, then the move ack

Every 128 hz tick also records bot poses for lag compensation.

## crates

| crate | what |
|---|---|
| `bits` | ue bit archive |
| `net` | ares transport: transform, gcm, framing, handshake, nmt replies, client decode, reliable resend, rtt |
| `rep` | content transform, content blocks, exports, every bunch builder, client rpc decode |
| `oracle` | read-only live client access (process, gobjects, fname, keys, game state reads) |
| `interp` | source-sdk-style interpolation, extrapolation, lag compensation |
| `world` | range game rules and state (shop, skins, equippables, bots, projectiles, abilities) |
| `range` | per-datagram dispatch, bring-up stages, authority systems |
| `aries-server` | the binary: settings, console, 128 hz tick, oracle adapter |

## tests

`cargo test --workspace`. Most tests replay golden fixtures exported from the python
(`python tools/export_golden.py`, sections in `tools/golden/*.py`, output in `fixtures/`).
`crates/range/tests` drive whole sessions through `RangeServer` with a scripted client
and check every bunch against the spec vectors.
`cargo run -p aries-oracle --example live_reads` is a read-only live check against a
running client.

## shadow diff against the python

```
python tools/shadow_diff.py <local_backend/runtime/responder_*.out> <logs/aries_events_<pid>.jsonl>
```

Both servers log one json event per datagram with the same keys. The tool compares the
order of first appearances, the deterministic frame lengths and bit counts, and per-key
counts (timing-dependent and rust-only keys are left out).
`cargo test -p aries-range --test gameplay_flow -- --ignored` writes a scripted session to
`target/harness_events.jsonl` for the same comparison.
