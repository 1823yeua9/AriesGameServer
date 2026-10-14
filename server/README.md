# server

Cargo workspace for the Aries game server (the Rust port of
`local_backend/range_udp_handshake.py`). `platform/` is codex's backend port and is
excluded from this workspace (own lock file and target dir).

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
- `--port N` overrides the port (useful to run beside the python responder).
- The python responder must not be running on the same port.

## for the loader

| loader setting | value |
|---|---|
| `gameplay_path` | `server/target/release/aries-server.exe` |
| `gameplay_config` | `server/aries-server.toml` |

Start it after the game process exists (it needs the pid for the key scan); it retries
the key scan in the background until the client's handler exists, and drops game
packets until then (the python's `not_ready`).

## crates

| crate | what |
|---|---|
| `bits` | ue bit archive |
| `net` | ares transport: transform, gcm, framing, handshake, nmt replies, client decode |
| `rep` | content transform, content blocks, exports, every bunch builder, client rpc decode |
| `oracle` | read-only live client access (process, gobjects, fname, keys, game state reads) |
| `interp` | source-sdk-style interpolation, extrapolation, lag compensation |
| `world` | range game rules and state (shop, skins, equippables, bots, projectiles, abilities) |
| `range` | per-datagram dispatch, bring-up stages, authority systems |
| `aries-server` | the binary |

## tests

`cargo test --workspace`. Most tests replay golden fixtures exported from the python
(`python tools/export_golden.py`, sections in `tools/golden/*.py`, output in `fixtures/`).
`cargo run -p aries-oracle --example probe` is a read-only live check against a running client.
