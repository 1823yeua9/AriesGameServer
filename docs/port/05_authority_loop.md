# 05 Authority loop: per-packet authority block and run_probe helpers

Spec for the Rust `world` / `range` tick systems.

Sources (2026-10-14 tree; a frozen copy is `_preport_backup/local_backend_20261014`):
- `local_backend/range_udp_handshake.py` 715-1372 (run_probe locals and helper closures) and 1486-2079 (authenticated per-packet block).
- Builders: `range_equippables.py`, `range_gun_loader.py`, `range_abilities.py`, `range_bots.py`, `range_projectiles.py`, `range_attachments.py`, `range_store.py`, `range_weapon_cosmetics.py`, `range_server_bunch.py`.
- Inbound decoders: `range_client_rpc.py`, `range_client_input.py`.
- Oracle reads (ReadProcessMemory today): `read_equippable_state.py`, `capture_input_event_cursor.py`, `range_projectiles.read_projectile`.

`file:line` refs point into `local_backend/`; a bare `:N` means `range_udp_handshake.py:N`.

## 0. Runtime requirements (from the user)

- The external server runs at **128 ticks per second** (7.8125 ms per tick). Section 9 maps every Python timer onto that tick.
- The server **allocates a console** (`AllocConsole`; `windows-sys` feature `Win32_System_Console` is already in the workspace deps) and logs to it. Section 8 says what Python logs today and how to map it.

## 1. Production configuration for this area

These flags are on (launch.ps1 166-192 -> launch_range.py 29-53, 214-287 -> `range_udp_handshake.main` 4915):
`pistol_state_mirror`, `input_correction_responder`, `store_purchases`, `range_mode_hud`, `ability_activation`, `range_streaming_levels`, `command_queue`, `range_settings`, `range_bots` (`range_bot_count=5`, :4684), `hud_refresh`, `ability_projectiles` (`ability_projectile_stage="A"`, :4687), `movement_packet_capture`, `input_ack_cursor_trial`, `range_money_probe` (ultimate points), `starting_equippables_probe` (used as a gate), `team_state_array_probe` (moves ch4 offsets). `goto_state_probe=False` and `client_game_phase_begin_probe=False` fix the ch5 offsets.

Environment: no launcher sets any `VALORANT_RANGE_*` variable. So `VALORANT_RANGE_QUIET_PACKETS` is unset (every event is printed) and `VALORANT_RANGE_HEARTBEAT_HZ` is 0 (no heartbeat).

Base-pistol initial cosmetic trial: launch.ps1 189-192 adds `--base-pistol-initial-cosmetic-trial` unless skipped. launch_range.py 286-287 passes it on, and `main` 4908-4909 turns it into the env var `RANGE_BASE_PISTOL_INITIAL_COSMETIC_TRIAL=1`. run_probe reads that at 737-739. It then sets `base_pistol_catalog_path = local_backend/skin_catalog.json`, `base_pistol_state_path = offline_skin_state.json` and `base_pistol_player_id = backend.offline_player_id = "7c9f41a4-65e2-4d81-a8d5-f879c3e5ab61"` (754-758, backend.py:36). Only the BasePistol actor open in the setup chain consumes it (2986-2991; see the setup-chain doc). Guards: run_probe raises if the skin trial is also set (746) or if `RANGE_EQUIP_STATE_TRIAL=1` (748). Note that `backend.py:57-66` (the `/dev/range/enter` respawn path, launched at `backend.py:86`) does **not** pass this flag, so a responder respawned that way opens an unskinned Classic (quirk 14).

### Dead paths in this area (skip)

- `server_performance_rpc` (1752-1776): field 67 / max 87 is unverified and left +0xB60 at inf (range_server_bunch.py:969-977). Off.
- Cloudburst stage "B" (1034-1036 forward 0, 1069-1074 bending, 1077-1081 teleport and MulticastStopProjectile, `ignore_pawn`): off.
- `acr_layout_trial` (725-734): off.
- `RANGE_BASE_PISTOL_SKIN_TRIAL` late skin trial (736, 759-765, 3260-3289): off.
- `RANGE_ACR_ACTOR_INSPECTION_PROBE` (740, 4192-4218): off.
- `RANGE_EQUIP_STATE_TRIAL` (865, 4335-4363): off.
- `VALORANT_RANGE_CAPTURE_LOGIN` (2079-2085): off.
- Heartbeat `VALORANT_RANGE_HEARTBEAT_HZ` (1366-1369, 1416-1437): 0, so off. `latest_authenticated_packets` is still maintained.
- Quiet filter `VALORANT_RANGE_QUIET_PACKETS` (1365, 1373-1400): off. Its key list is still useful for console levels (section 8).
- `take_ch9_sequence` / `take_ch13_sequence` / `take_ch8_sequence` wrappers (795-802): only the dead trials call them (3277, 4351). Production goes through `send_reliable` -> `take_channel_sequence`.
- `--move-ack-uniform-transform` / `set_uniform_byte_transform` (:4912-4914, range_server_bunch.py:1079-1083): off. `_UNIFORM_BYTE_TRANSFORM` stays False.
- `pistol_mirror_polled/machine/auth/exported/active` and `melee_mirror_polled/machine/auth/exported/active` (881-891): declared, never read or written. Only `pistol_mirror_reader` (885) is live. Do not port.
- `VALORANT_RANGE_DROP_FLAGS` (launch_range.py:210-213): A/B hook that removes responder flags. Unset in production; ignore.

## 2. Trigger and per-packet order

Python has no tick: all of this runs inside the receive loop, once per authenticated datagram from 127.0.0.1 (1440-1579). The client sends 60-110 datagrams per second; `runtime/responder_20928_phase4_match_state.out` shows 101-112/s for its long sessions.

Per datagram:
1. `parse_handshake` raises for every in-session packet, so control enters the `except ValueError` branch (1485).
2. `inspect_client_ares_packet` (1487; :541-557). If it returns `ares_inspection == "authenticated"`:
   1. `recent_client_sequences[source]` is a `deque(maxlen=64)`. `ack_history_mask = update_received_packet_history(deque, seq)` (1490-1492; :560-576): for each remembered sequence at distance d = (seq - prev) & 0x3FFF with 1 <= d <= 32, set bit d-1. Then append seq.
   2. `latest_authenticated_packets[source] = (seq, stateless_header, mask, monotonic)` (1493-1495).
   3. Input ack (1499-1563), section 6.1.
   4. The authority block (1566-2078) runs if `receive_key_override is not None and live_client_pid and live_client_image_base and source in client_restart_sent_sources`. `now = time.monotonic()` is taken once (1569), and every feature in the pass uses it, including job scheduling.
3. Then come movement capture (2086-2106), the hello/ack/welcome/join-ack/session-ack and setup elif chain (2107+, other docs), and event append and emit (4573-4578).

So within one datagram, authority-block sends take lower server sequence numbers than the session ACK and setup-chain sends.

Fixed order inside the authority block. Keep it: several features share channels, and the order decides the reliable sequence each one gets.

| # | Feature | Lines | Cadence |
|---|---|---|---|
| 1 | state reader (lazy create) | 1591-1598 | every pass |
| 2 | equippable mirror (+ Cloudburst spawn hook, firing-seed shots -> bots) | 1600-1711 | 50 ms |
| 3 | input-correction responder | 1718-1747 | 200 ms poll, >= 1 s between sends |
| 4 | server performance RPC | 1752-1776 | dead |
| 5 | Range HUD | 1781-1799 | once |
| 6 | level streaming | 1805-1822 | once, >= 1 s after 5 |
| 7 | ultimate points | 1827-1845 | every pass until sent once |
| 8 | store purchases (decode, plan, advance jobs) | 1851-1907 | every pass |
| 9 | range settings (request once, decode, apply) | 1913-1946 | every pass |
| 10 | command queue | 1951-1982 | 200 ms |
| 11 | HUD refresh | 1991-2023 | 1 s poll, <= 6 sends |
| 12 | Cloudburst flights | 2026-2030 | 50 ms |
| 13 | bots tick | 2034-2062 | 100 ms |
| 14 | ability jobs | 2066-2078 | every pass, per-job `next` |

## 3. Reliable sends, channel sequences, packet sequence

### 3.1 `send_reliable` (1571-1585)

`send_reliable(channel, body, bits, has_exports, is_open=False, is_close=False)` sends **one bunch per datagram**:
- `BunchHeader(channel_index=channel, channel_name_index=102, channel_sequence=take_channel_sequence(source, channel), is_reliable=True, is_open, is_close, has_exports)`. Everything else is default: `replication_paused=False`, `close_reason=0` (Destroyed).
- `build_server_bunch_frame(body, bits, header, server_sequence=next_server_sequences[source], acked_client_sequence=event["client_packet_sequence"], ack_history_mask=<this packet's mask>, stateless_header=event["stateless_header"], receive_key=receive_key_override)`, then `sendto`, then `next_server_sequences[source] = (+1) & 0x3FFF`.
- A reliable header always carries the hardcoded-name bit (1) plus packed 102, open or not (range_server_bunch.py:1571-1587).
- A close bunch has an empty body (0 bits). The `SerializeInt(0, 15)` close reason after bClose is 4 zero bits (range_server_bunch.py:1576-1577).
- Python has **no retransmission, no ack tracking and no coalescing**. One lost reliable bunch stalls that client channel for good (later bunches buffer). Loopback has made this rare.
- The input ack (6.1) uses its own unreliable frame: `BunchHeader(channel_index=8, channel_name_index=102, is_reliable=False)` with no sequence and no name.

### 3.2 `take_channel_sequence` (786-793, 911-914)

```
offset = channel_next_offset.get((source, ch), channel_first_dynamic_offset[ch])   # KeyError if ch unregistered
channel_next_offset[(source, ch)] = offset + 1
seq = (initial_actor_channel_sequences[source] + offset) & 0xFFF
```

`initial_actor_channel_sequences[source] = (server_sequence + 1) & 0xFFF`, taken from the server sequence of the challenge-ack frame (2141). It is the base for **every** channel.

`channel_first_dynamic_offset` is a global dict, not per source. Production values:

| ch | Actor (GUID) | First dynamic offset | Fixed offsets used by the setup chain (other docs) |
|---|---|---|---|
| 4 | GameState (2) | 8 = `range_shop_sequence_offset(True) + 1` (911; :243-257) | +0 open, +1 bool, +2 phase, +3 team array, +4 MatchState, +5 gameplay start, +6 PlayerArray, +7 shop settings |
| 5 | controller (4) | 6 = `(7 if goto_state else 6) + (1 if game_phase_begin else 0)` (913) | +3 ClientRestart (3368), +4 ViewTarget (4320), +5 ClientSetHUD (3798) |
| 6 | PlayerState (6) | 11 (786) | +9 UniqueId (3693), +10 money (3722) |
| 8 | pawn (10) | 51 (786) | +38..+42 inventory corrections 1..5 (3338), +43 pawn Owner (3756), +44..+50 starting-equippable slot contents (4082) |
| 9 | BasePistol (94) | 5 (786) | +0..+4, InactiveState link at +4 (3246) |
| 12, 14-18 | starting equippables (200+4i) | 2 (786) | +0 open, +1 Owner (4071-4079) |
| 13 | melee (204) | 5 (786) | +2 role, +3 SM, +4 InactiveState (4120-4180) |
| purchase ch 20+, bots/dummy 400+, projectile/zone 10000+ | | 0, set just before the open (1197, 1225, 1965, 2042, 2056, 1038, 1085) | |
| queued commands with `"open"` | | `setdefault(ch, 0)` (1974) | |

Dynamic users in production, in the order the gates allow:
- ch4: Range HUD GameModeClass (+8) and HUD config (+9); then an infinite-ammo bool on each change (+10, ...).
- ch5: second ClientSetHUD (+6), streaming update (+7), flush (+8), ClientUpdateRangeSettings (+9).
- ch6: ultimate points (once) and HUD-refresh pairs, ordered by time.
- ch8: purchase attachment pre-exports, purchase slot contents and inventory corrections, armor slot contents and shield sections, input-resume corrections.
- ch9 / ch13: pistol and melee mirror; ch9 is also closed when a bought sidearm replaces the Classic.
- ch14-17: ability charge mirror, ability role/SM/Inactive, extra-machine opens and states, ability state mirror.
- ch20+: bought items (+0 open ... ), mirror, close on replacement.
- ch400+: dummy and bots (+0 open, +1 health, then hits, close).
- ch10000+: Cloudburst projectile (+0 open, close) and zone (+0 open, close).

### 3.3 Packet sequence

`next_server_sequences[source]` is 14-bit. Every datagram sent to the source (authority, input ack, session ACK, setup) takes one and increments it.

## 4. NetGUIDs and channels used by this area

| GUID(s) | Object | Source |
|---|---|---|
| 2, 4, 6, 10, 5 | GameState, controller, PlayerState, pawn, Range level | setup |
| 60 / 61 / 62+2*slot | UAresInventory / UItemSlot class / UItemSlot_<slot> | range_server_bunch.py:694-712 |
| 94, 95/97 | BasePistol actor, archetype/package | range_weapon_cosmetics.py:23-28 |
| 96; 98 Inactive, 99 Equip, 240 IdleWaitingForEvent, 242 Idle, 244 Firing, 246 Reload, 248 Readying, 250 Jammed, 252 FiringStateBurst | BasePistol SM and states | range_server_bunch.py:755-767 |
| 306 ReserveAmmo, 308 MagazineAmmo | BasePistol ammo components | range_equippables.py:326-327 |
| 200+4i (actor), +1 archetype, +3 package, i = 0..6 | starting equippables | range_server_bunch.py:701-706 |
| 204 / 206 / 256 Inactive, 258 Equip, 260+2k for 23 more states | melee actor / SM / states | range_melee_equippable.py:17-56 |
| 310, 312, 314 | Smoke EquipmentCharge, CycloneBoost (Q) EquipmentCharge, Dash (E) SignatureAbilityResource | range_equippables.py:272-274 |
| 316 | pawn ShieldDamageSection | range_equippables.py:152 |
| 109 / 111 | AresHUD_C class / package (exported by the setup ClientSetHUD) | :3789-3790 |
| 261/263, 265/267 | ShootingRangeGameMode_C, ShootingRangeHUDConfig_C class/package | range_server_bunch.py:864-868 |
| from 520 (per connection) | purchases: dynamic even, static odd pairs | range_gun_loader.py:41-70 |
| from 100000 (per connection) | ability SMs and states | range_abilities.py:25, 39-40 |
| 190000+2*index | AbilityTuning components (setup-chain opens) | range_abilities.py:158-160 |
| 300001/300003, 300005/300007 | TrainingBot_PC, TargetPracticeTrainingBot archetype/package | range_bots.py:42-43, 88-89 |
| from 300100, stride 8 | per bot: actor, health +2, handler +4, BombTeam +6 | range_bots.py:44-47, 136-146 |
| 610001/610003, 610005/610007 | smoke projectile, smoke zone archetype/package | range_projectiles.py:45-46 |
| from 610100, step 2 | projectile actor, FiniteSpeedMovement, zone actors | range_projectiles.py:61, 104-107 |

Channels: 4 GameState, 5 controller, 6 PlayerState, 7 player info, 8 pawn, 9 BasePistol, 10-11 team states, 12 Unarmed, 13 melee, 14 Smoke (C), 15 Dash (E), 16 CycloneBoost (Q), 17 Blade Storm (X), 18 Glide, 19 reserved (codex ACR), 20+ purchases, 400+ dummy and bots, 10000+ Cloudburst.

## 5. State

### 5.1 Gates read from the setup chain

| Predicate | Written at | Meaning |
|---|---|---|
| `source in client_restart_sent_sources` | 3379 | ClientRestart sent (ch5 +3); the authority block runs and the input ack is allowed |
| `source in inactive_state_link_sent_sources` | 3258 | BasePistol ch9 +4 sent |
| `auth_correct_rep_index[source] >= 5` | 3352 | setup corrections 1..5 sent (ch8 +38..+42) |
| `source in melee_inactive_state_sent_sources` | 4190 | melee ch13 +4 sent |
| `starting_equippable_index[source] >= 7` | 4100 | all 7 `wushu_starting_equippables` opened ("starting equippables done") |
| `source in range_shop_sent_sources` | 4002 | ch4 +7 shop settings sent |
| `hud_rpc_cache[source] = (field, max)` | 3791 | setup ClientSetHUD sent (ch5 +5); live-resolved, (52, 178) in all 54 logged sessions |
| `source in range_money_sent_sources` | 3734 | ch6 +10 money sent |
| `source in pawn_role_sent_sources` | 2687 | movement capture only |

### 5.2 Per-connection state (keyed by `source = (ip, port)`)

A reconnect from a new port starts fresh; old entries are never freed.

| Python name | Type | Purpose |
|---|---|---|
| `initial_actor_channel_sequences[s]` | int | reliable sequence base |
| `channel_next_offset[(s, ch)]` | int | next reliable offset per channel |
| `next_server_sequences[s]` | int (14-bit) | packet sequence |
| `recent_client_sequences[s]`, `latest_authenticated_packets[s]` | deque(64), tuple | ack history |
| `last_input_ack_numbers[s]` | int | last acked input event ID |
| `equippable_mirror_polled[s]` | float | mirror timer |
| `equippable_mirror_states[(s, key)]`, `[(s, f"{key}/{machine}")]` | MirrorState | state mirror |
| `equippable_resource_states[(s, key, resource)]` | ResourceMirrorState | ammo/charge mirror |
| `firing_seeds[(s, key)]` | int | last firing RNG seed |
| `auth_correct_rep_index[s]` | int | last CorrectionIndex sent (setup 1..5, then +1 each) |
| `input_correction_polled[s]`, `input_correction_sent_times[s]` | float | responder timers |
| `purchase_sequences_seen[s]` | set[int] | ServerPurchase dedup by 12-bit channel sequence |
| `purchase_allocators[s]` | PurchaseAllocator | channels from 20, GUIDs from 520 |
| `purchase_jobs[s]` | list[dict] | `{"load", "stage", "next", "registered_tries", "armor"?, "attachments_exported"?}`; never pruned |
| `purchased_class_guids[s]` | {class: actor GUID} | used by the input-correction responder |
| `purchase_package_guids[s]` | {package GUID: package path} | resolves `<guid:N>.Class` |
| `purchase_seen_paths[s]` | {class: full path} | fallback resolution |
| `purchased_slot_items[s]` | {slot: (ch, key, actor, class)} | replacement |
| `purchased_ready_equippables[s]` | list[EquippableActor] | mirror entries for bought guns |
| `starting_pistol_replaced` | set | Classic replaced (closed, unmirrored) |
| `shield_section_exported` | set | ShieldDamageSection exported once |
| `ability_jobs[s]`, `ability_allocators[s]`, `ability_ready_equippables[s]` | list, PurchaseAllocator(0, 100000), {key: EquippableActor} | ability activation |
| `range_hud_sent_sources`, `range_hud_sent_times[s]` | set, float | Range HUD |
| `range_streaming_sent_sources` | set | streaming |
| `ultimate_points_sent_sources` | set | ult |
| `range_settings_requested`, `range_settings_sequences_seen[s]`, `range_settings_state[s]`, `range_infinite_ammo_sent[s]` | set, set, request, bool | settings (`range_settings_state` is written at 1941 and never read) |
| `command_queue_polled[s]` | float | queue timer |
| `hud_refresh_polled[s]`, `hud_refresh_counts[s]` | float, int | HUD refresh |
| `projectile_fields[s]`, `projectile_ticks[s]` | ProjectileField, float | Cloudburst |
| `bot_fields[s]`, `bot_ticks[s]`, `bot_spawned_at[s]` | BotField, float, float | bots |

### 5.3 Global (whole run)

- `channel_first_dynamic_offset` (see 3.2).
- `pistol_mirror_reader["reader"]`: **one** EquippableStateReader for every source (1592-1595). Its `_names` cache ({SM address: {state name: state object address}}) and `_class_names` cache ({object address: name}) are never invalidated.
- `firing_rng_components[(state_machine_addr, key)]`: cached RNG component pointer (944, 949-961).
- `tag_net_index_cache`: setup-chain AbilityTuning opens (935, 4051-4060).
- `range_gun_loader._inventory_cache` (248-273): the parsed skin inventory per player id, rebuilt only when the mtime of `offline_skin_state.json` changes. Used by purchase stage 0.
- Constants: `starting_pistol_equippable`, `starting_melee_equippable` (895; range_equippables.py:312-332) and `starting_ability_equippables` (896; range_equippables.py:252-275).

## 6. Features

### 6.1 Input ack (`input_ack_cursor_trial`, 1499-1563)

Trigger: authenticated packet, `source in client_restart_sent_sources`, a receive key, and the packet's **first** bunch on channel 8 (`event["channel_index"]`, :512). It runs outside the authority block, before it.

1. `decode_client_ares_packet` -> `decode_received_actor_inputs(decoded, bunch_body_start_bit, bunch_end_bit)` (range_client_input.py:272-280). The first content block must be `is_actor`; a rep-layout block gives `()`. Its payload is content-decoded with actor map value 10, then bit-aligned 0xA3 input streams are scanned (:190-269). A stream is accepted only when fully parsed, terminator included. `max_generated_event_id` is not passed (None). The stream wire format (marker, u32 first id, packed count 1..256, per-event `[1][4-bit kind]` with fixed payload widths, terminator 0) is specified in `06_client_decode.md` (around line 614-650); `KeyError`/`ValueError` from the decode give `()`.
2. With exactly one stream, oracle `read_pending_input_bounds` (capture_input_event_cursor.py:42-65): `latest_generated = (cursor@+0xA84 + count@+0x1198) & 0xFFFFFFFF` on the pawn input component. If the read failed or `latest_id > latest_generated`, the stream is rejected (`input_ack_rejected_event_id`).
3. Still one stream: kind-1 events are logged as `received_activation_events` (event_id, slot, state, action). If `latest_id > last_input_ack_numbers.get(source, -1)`:
   - payload `build_input_ack_rpc_payload(latest_id, 0, 41, 53)` (range_server_bunch.py:463-489): `SerializeInt(41, 54)` (6 bits), packed param bits (66), `[1][32 bits id][1][32 bits 0]` = 80 bits. `encode_short_actor_rep_payload(.., 0, 10)` then `append_actor_rpc_content_block` gives a 90-bit body.
   - It is sent as an unreliable ch8 frame (3.1). Server sequence +1, `last_input_ack_numbers[source] = latest_id`.
   - Events: `input_ack_received_event_id`, `input_ack_received_frame_length`.

### 6.2 State reader (1591-1598)

`EquippableStateReader(pid, image_base)` (read_equippable_state.py:90-98) opens the process with `0x10 | 0x1000` (resolve_rpc_index.py:36) and derives the FName key. It is created on the first authority pass; on failure `state_reader = None` for that pass (`equippable_reader_error`) and creation is retried next pass. Every oracle-dependent feature checks `state_reader is not None`.

### 6.3 Equippable mirror (`pistol_state_mirror`, 1600-1711)

Gate: `state_reader`, a non-empty ready list, and `now - equippable_mirror_polled[s] >= 0.05`; polled is set to `now` first.

Ready list, `mirror_ready_equippables` (1316-1331), in this order:
1. Starting pistol (`key "BasePistol"`, ch9) if `s in inactive_state_link_sent_sources`, not in `starting_pistol_replaced`, and `auth_correct_rep_index >= 5`.
2. Starting melee (`key "Melee"`, ch13, slot 2) if `s in melee_inactive_state_sent_sources`.
3. If starting equippables are done: for each entry of `starting_ability_equippables`, in order Smoke (idx 2, ch14), CycloneBoost Q (idx 4, ch16), Dash E (idx 3, ch15), use `ability_ready_equippables[s].get(key, resource_only_entry)`. Then every ready ability whose key is not one of those three (Blade Storm X, ch17).
4. `purchased_ready_equippables[s]`, in list order.

Each pass makes one oracle read: `read_slot_machines(reader, [eq.slot for eq in ready])`. Then, per equippable, skipping it when there is no machine for its slot or `machine.class_name != eq.class_name`:

a. If `ability_projectiles` and class is `Ability_Wushu_4_Smoke_C`: `cloudburst_state(machine.predicted)` (6.13).
b. If `range_bots and machine.is_current and eq.resources`: firing-seed check (6.4).
c. Resources, in tuple order: `read_resource_amounts(reader, machine.item, eq.resources)` once, then for each resource with a reading:
   `amount = resource_target(st, predicted, authoritative, default, now, refill_after)`. If not None: `export = st.auth is None`; `send_reliable(eq.channel, build_resource_amount_body(eq, res, amount, export), export)`; `resource_confirmed(st, amount, now)`. State key `(s, eq.key, res.name)`.
d. If `not eq.mirror_states` (resource-only ability entries), stop here.
e. Extra machines, if any: `read_item_machines(reader, machine.item)` gives machines by name. For each extra: `st = states.setdefault((s, f"{key}/{name}"), MirrorState(auth=extra.starting_state, exported={starting}))`; `target = machine_mirror_target(st, live.predicted, extra.state_guids)`; `export = target not in st.exported`; send `build_machine_state_body(eq, extra, target, export)` with `has_exports=export`; then `exported.add`, `auth = target`.
f. Main machine: `st = states.setdefault((s, key), MirrorState())`, i.e. auth `InactiveState`, exported `{InactiveState}`, active False, kicked False. `target = mirror_target(eq, st, machine.predicted, machine.is_current)`; `export = target not in st.exported`; send `build_auth_state_body(eq, target, export)` with `has_exports=export`; then `exported.add`, `auth = target`.

Event `sent_equippable_auth_states`: `{key: [target, predicted], "key/machine": target, "resources": {"key.name": amount}}`.

Policies (range_equippables.py):
- `mirror_target` (287-309), in order:
  1. None if `predicted` is None or not in `state_guids`.
  2. If `predicted != "InactiveState"`, set `active = True`.
  3. If `is_current and not active and not kicked and "EquipState" in guids`: set `kicked = True` and return `"EquipState"`. This is the one-time push for an item a correction made current.
  4. None if `predicted == auth`.
  5. None if `predicted == "InactiveState" and not active`.
  6. Otherwise `predicted`.
- `machine_mirror_target` (144-148): None if predicted is None, unknown, or equal to auth; otherwise predicted.
- `resource_target` (217-242):
  - First call (`auth is None`): return `default` if 0 <= default < 10000, else None (retried next poll).
  - Then None until `authoritative == auth`. The first poll after that sets `baseline = predicted` and returns None.
  - Then return `predicted` if it differs from baseline and auth and is in [0, 10000).
  - Refill: if `refill_after` is set, `auth < default < 10000`, and `now - sent_at >= refill_after`, return `default`.
  - `resource_confirmed` (245-249) sets `auth = amount`, `baseline = None`, `sent_at = now`.
  - Resources: BasePistol and guns use MagazineAmmo (+0x1208) then ReserveAmmo (+0x1200), default at component +0x12C. Ability charges default at +0x188 with `refill_after=4.0`.

Body shapes (all `rep` payloads go through `encode_short_actor_rep_payload(rep, bits, 0, actor_guid)`):
- `build_auth_state_body` / `build_machine_state_body` (88-116, 137-141):
  `[export? 0 | u32 1 | packed state_guid | u8 5 | packed actor_guid | u8 0 | FString state | u32 0] [1][0] packed machine_guid [1] packed rep_bits enc(rep)`, with `rep = [0] packed 3 packed state_guid packed 0`.
- `build_resource_amount_body` (187-207): the same frame on the resource GUID; `rep = [0] packed 3 u32 amount packed 0`.
- `build_machine_open_body` (119-134): export plus an empty `rep = [0] packed 0`.

Extra machines:
- `plan_extra_machines` (1102-1118): reads `read_item_machines` (item+0xE30 array, count +0xE38 <= 8). For each machine whose address differs from the main SM and that has a starting state (SM+0x1E8), it builds `ExtraStateMachine(name, guid=alloc.dynamic_guid(), starting_state, state_guids={st: alloc.dynamic_guid() for st in machine.state_names})`. GUID order: the machine, then its states in client TMap order.
- `send_extra_machines` (1120-1128): per extra, `build_machine_open_body` (exports), then `build_machine_state_body(starting, export=True)` (exports). It then stores `MirrorState(auth=starting, exported={starting}, active=True)`.

### 6.4 Firing seed and bot hits (946-1012, 1624-1632)

- Seed: `u32 [[FiringState + 0x4A0] + 0xEC]`. FiringState is the state named `FiringState`, else the first name starting with `Firing`, in `reader._names[sm]`. The RNG pointer is cached globally per `(sm address, key)`.
- `firing_seeds[(s, key)]`: when a previous value exists and differs, call `bot_shots(source, class, 1, ...)`. So at most one shot per 50 ms poll, even with infinite ammo.
- `bot_shots` (969-1012) needs `bot_fields[s]` and the reader:
  - Oracle `read_camera`: controller+0x528 PlayerCameraManager, then 6 doubles at +0x17C0 (location x,y,z; pitch, yaw, roll).
  - `view_ray(p, y) = (cos p cos y, cos p sin y, sin p)`.
  - `first_hit` checks live, opened bots (`hit_region`, range_bots.py:326-338): head sphere at z+75 with r22; body cylinder r28 for z in [-22, 53]; legs cylinder r22 for z in [-98, -22]; nearest t > 0 wins.
  - `dummy_hit` checks the opened dummy, centre z + 4.0.
  - If the dummy is hit and (no bot hit, or the dummy is nearer): `send_reliable(dummy.channel, build_update_damage_text_body(dummy, float(damage)), False)`, event `dummy_hits`, and the shot ends.
  - If nothing is hit: `bot_shot_missed += 1`.
  - Otherwise `damage = shot_damage(class, region)` (table range_bots.py:71-83, default (100, 35, 30)). `bot.life -= damage`; at <= 0, set `life = 0`, `alive = False`, `died_at = now`. Send `build_bot_health_body(bot, False)` (no exports). If dead, also send `build_bot_alive_body(bot, True)` (exports DamageHandlerComponent; handle 3 = 0). Event `bot_hits`.

### 6.5 Input-correction responder (1718-1747)

Gate: `state_reader`, `auth_correct_rep_index >= 5`, starting equippables done, and `now - input_correction_polled >= 0.2` (polled = now).

- Oracle `read_input_replication` (read_equippable_state.py:247-268): `stopped = byte [input+0x12C] != 0`, `requested = u32 [input+0x130]`, `inventory = u32 [inv+0x2F0]`. The class is that of `[inv+0x260]` (desired), or else `[inv+0x278]` (current). Input component = pawn+0xB90; inventory = pawn+0xBF8.
- If `stopped` and `now - input_correction_sent_times >= 1.0`:
  - `idx = max(requested, inventory, ours) + 1`.
  - `guid = purchased_class_guids[s].get(cls) or equippable_guid_for_class(cls)` (range_server_bunch.py:803-810: `BasePistol_C` -> 94; `Default__<cls>` in the starting list -> 200+4i; otherwise None, which omits handle 25).
  - `send_reliable(8, build_inventory_correction_body(idx, guid), False)`; `auth_correct_rep_index = idx`; sent time = now.
  - Events: `sent_input_resume_correction`, `input_resume_equippable`.
- Body (range_server_bunch.py:813-839): `[1][0] packed 60 [1] packed bits enc10(rep)`, with `rep = [0] ([packed 25][packed guid])? [packed 26][u32 idx][packed 0]`.

### 6.6 Range HUD (`range_mode_hud`, 1781-1799)

Gate: `s in range_shop_sent_sources`, `s in hud_rpc_cache`, not yet sent. Three sends:
1. ch4, exports: `build_actor_class_property_body(2, "ShootingRangeGameMode_C", "/Game/GameModes/ShootingRange/ShootingRangeGameMode", 261, 263, 15)`.
2. ch4, exports: `build_actor_class_property_body(2, "ShootingRangeHUDConfig_C", "/Game/UI/InGame/HUD/GameStateHUDConfigs/ShootingRangeHUDConfig", 265, 267, 58)`.
3. ch5, no exports: `build_client_set_hud_payload(field, max, 109)` from `hud_rpc_cache` (`SerializeInt(52, 179)`, packed param bits, `[1][packed 109]`), map 4, `append_actor_rpc_content_block`.

It then sets `range_hud_sent_times[s] = now` and logs `sent_range_mode_hud`.

`build_actor_class_property_body` (range_server_bunch.py:961-967) is the class export (221-241: `[0][u32 1][packed class][u8 5][packed package][u8 5][packed 0][FString package][u32 0][FString class][u32 0]`), followed by `[1][1][packed bits]enc2([0][packed handle][packed class_guid][packed 0])`.

### 6.7 Level streaming (`range_streaming_levels`, 1805-1822)

Gate: Range HUD sent, not yet sent here, and `now - range_hud_sent_times >= 1.0`. Two ch5 bunches, no exports, each map 4 plus an RPC block:
1. `ClientUpdateMultipleLevelsStreamingStatus` (range_server_bunch.py:884-918): `SerializeInt(66, 179)`, packed param bits, then `[1][u16 2]`, then per level `[0][FString path][u32 0][u32 0xFFFFFFFF][1][1][0][0]`. The levels are `/Game/Maps/Poveglia/Range_Only_Gameplay` and `/Game/LiveStory/LiveStory_Permanent/Range_LiveStory_Permanent` (874-877).
2. `ClientFlushLevelStreaming`: `SerializeInt(26, 179)`, then packed 0.

Event `sent_range_streaming_levels`.

### 6.8 Ultimate points (`range_money_probe`, 1827-1845)

Gate: `s in range_money_sent_sources`, the reader, and not yet sent. It runs **every pass until it succeeds** (no throttle).

Oracle: `find_pawn`, then `i32 [pawn+0x14F4]` (the ult requirement). If `0 < x < 50`: `send_reliable(6, build_actor_int_property_body(6, 197, x), False)`, with `rep = [0][packed 197][u32 x][packed 0]` map 6, wrapped as `[1][1][packed bits]`. Event `sent_ultimate_points` (8 in the logs).

### 6.9 Store purchases (`store_purchases`, 1851-1907, 1172-1314)

Gate: starting equippables done. Purchases the client sent before that are acked and lost (the client never resends).

Each pass:

1. `decode_client_ares_packet` -> `decode_purchase_requests` (range_client_rpc.py:181-194):
   - Walk **every** bunch of the packet (`iter_bunches` :84-128, from bit 64: packet-info bit, plus 10 bits if set, plus 1 bit, then bunch headers). The walk continues while `position + 16 < bit_count` and stops at the first control-close bunch (bClose bit 1; no close reason is read) or the first header `ValueError`. Full edge cases are in `06_client_decode.md` 4.3.
   - For each reliable, non-partial ch5 bunch: `read_actor_content_block(map 4)`. It must be `is_actor` without a rep layout.
   - `field = SerializeInt(.., 179)`, which must be 156 (purchase), 157 (remove) or 161 (sell). Then packed param bits.
   - Params: `[p][object ref]` ItemToPurchase, `[p][u8]` TargetSlot, `[p][3 bits]` Type.
   - Object ref (:131-148): packed GUID. 0 means None. A value other than 1 gives `"<guid:N>"`. A value of 1 is followed by u8 flags; if `flags & 1` is 0 the result is `"<guid:1>"`; otherwise an outer ref (recursive, depth <= 8), FString name (negative length = UTF-16, ASCII length > 1024 rejected), and if `flags & 4` a u32 checksum, giving `"outer.name"` (just `name` when the outer is None).
2. Dedup on `purchase_sequences_seen[s]` (12-bit channel sequence). Event `received_purchases`. Only field 156 with a path acts.
3. `resolve_class_path` (range_store.py:63-85): full paths pass through. `<guid:N>.Class` maps to `purchase_package_guids[s][N] + "." + Class`, else `purchase_seen_paths[s][Class]`, else None. `store_item_for_path` (`split_class_path` requires `/Game/.../X.X_C`; the table is range_store.py:25-48, 23 items) or `purchase_unsupported`. Then `purchase_seen_paths[s][class] = path`.
4. `allocator = purchase_allocators.setdefault(s, PurchaseAllocator())` (range_gun_loader.py:45-70). `dynamic_guid`: round up to even, return it, advance by 2. `static_pair`: round up to odd, return `(n, n+2)`, advance by 4. `channel()` returns 20, 21, ...
5. Plan:
   - Armor (`is_gun False`): `plan_item_load` allocates channel, actor (dynamic), then archetype/package (pair).
   - Gun: `plan_gun_load` (123-138) allocates channel, actor, (archetype, package), SM, magazine, reserve, three cosmetic pairs, then one pair per forced attachment (`range_attachments.FORCED_ATTACHMENTS`, 38-97).
   - Then `purchase_package_guids[s][package_guid] = package`. Append the job `{load, stage 0, next now, registered_tries 0[, armor True]}`. Event `purchase_planned`.
6. Advance every job with `stage < 7 and now >= next`, in list order (1904-1907). A job planned in this pass runs stage 0 in the same pass.

Gun job (`advance_purchase_job` 1213-1314). Key = `f"{class}@{channel}"`.

| Stage | Sends (channel, has_exports) | Then |
|---|---|---|
| 0a | Only if the gun has forced attachments and they are not yet exported: ch8, exports: `attachment_pre_export_body(assets, attachment_guid_pairs)` (`[0][u32 n]`, then per asset `packed obj, u8 5, packed pkg, u8 5, packed 0, FString package, u32 0, FString cdo, u32 0`) | `next = now + 0.2`, stay at stage 0 |
| 0 | `channel_first_dynamic_offset[ch] = 0`. Gun ch, **open**, exports: `gun_open_body(load, cosmetics)`, where cosmetics is the first candidate that builds without ValueError from [`select_saved_cdo_paths(player, weapon_id)` if not None, `standard_cdo_paths(...)` if not None, None]. Then gun ch, no exports: `build_channel_actor_owner_block(actor, 10)` | stage 1, `next = now + 0.5` |
| 1 | gun ch, no exports: `gun_role_body` (RemoteRole handle 4 = 2, 3 bits) | +0.5 |
| 2 | gun ch, exports: `gun_state_machine_open_body` (EquippableStateMachine export, empty rep) | +0.5 |
| 3 | gun ch, exports: `gun_initial_state_body` (allocates the InactiveState GUID; AuthCurrentState = Inactive, exported) | +0.5 |
| 4 | ch8, no exports: `build_item_slot_contents_block(slot, actor)`. ch8, no exports: `build_inventory_correction_body(auth_correct_rep_index + 1, actor)`, then store that index. Replacement: `replaced = purchased_slot_items[s].get(slot)`, or the Classic `(9, "BasePistol", 94, "BasePistol_C")` if slot == 1 and not yet replaced (adds to `starting_pistol_replaced`). If replaced: **close** the old channel (`b"", 0, False, is_close=True`), drop the old key from the purchased ready list, and drop `purchased_class_guids[old_class]` if it still names the old GUID. Record `purchased_slot_items[slot] = (ch, key, actor, class)` and `purchased_class_guids[class] = actor` | +0.5 |
| 5 | `read_slot_machines([slot])`. If missing or the class differs: `tries += 1`; above 20, stage 7 (`purchase_register_gave_up`); else `next = now + 0.3`. Else: `names = list(reader._names[sm])` (client TMap order, includes Inactive); `extra = plan_extra_machines(...)` (allocates first); `eq = gun_equippable(load, alloc, names, extra)` (allocates the remaining state GUIDs in `names` order); `send_extra_machines`; ready = other bought items whose slot differs, plus `eq` | stage 7 |

Gun open body (range_gun_loader.py:150-186):
- Unskinned: `build_dynamic_actor_reference((Default__X_C, package), actor, archetype, package, level 5)`. With attachments, an actor block follows: `rep = [0]`, then object array handle 20 (`packed 20, u16 n, (packed i, packed guid)* for i = 1..n, packed 0`; range_attachments.py:109-117), then FGuid array handle 40 (`packed 40, u16 n, (packed 4i+j+1, u32 word)* for element i = 0.., leaf j = 0..3, packed 0`; :120-129), then `packed 0`, map = actor, wrapped as `[1][1][packed bits]`. Guard: 7600 bits. The cosmetic pairs are allocated by `plan_gun_load` even when the gun opens unskinned.
- Skinned: `build_catalog_cosmetic_actor_open` (range_weapon_cosmetics.py:265-354). It has four static exports (archetype, then skin/level/chroma CDOs on `cosmetic_guid_pairs`), the actor header with a default transform (4 zero bits), and a property block with handles in ascending order: 18 skin ptr, 19 level ptr, [20 attachment array], 21 chroma ptr, 24-27 skin FGuid, 28-31 level FGuid, [40 attachment FGuid array], 41-44 chroma FGuid. Guard: 7600 bits.
- Cosmetic data: `skin_catalog.json` and `offline_skin_state.json` through `cached_skin_inventory` (range_gun_loader.py:248-305).
- FGuid words (`guid_text_to_words`, range_weapon_cosmetics.py:51-62): drop the dashes and split the 32 hex digits into four 8-digit groups, each parsed as one u32 (`27f21d97-4c4b-bd1c-1f08-31830ab0be84` -> `0x27f21d97, 0x4c4bbd1c, 0x1f083183, 0x0ab0be84`). The attachment `uuid_words` in range_attachments.py:38-97 are already in this form.

Armor job (1191-1212):

| Stage | Sends | Then |
|---|---|---|
| 0 | `channel_first_dynamic_offset[ch] = 0`; item ch, **open**, exports: `item_open_body(load)`; item ch, no exports: Owner = 10 | +0.5 |
| 1 | ch8, no exports: `build_item_slot_contents_block(11, actor)` | +0.5 |
| 2 | `shield = armor_shield_values[class]` (Light 25, Heavy 50, Plasma 25; default 25). `export = s not in shield_section_exported`. ch8: `build_shield_section_body(shield, shield, export)` with `has_exports=export` | stage 7 |

There is no inventory correction and no replacement, so a previous armor actor stays open. The shield body (range_equippables.py:160-184) is subobject 316 `ShieldDamageSection` with outer pawn 10: `rep = [0][packed 3][1][packed 4][f32 life][packed 5][f32 max][packed 0]`, map 10.

### 6.10 Range settings (`range_settings`, 1913-1946)

Gate: `s in range_streaming_sent_sources`.

- Request (once): when `now - range_hud_sent_times >= 3.0`, `send_reliable(5, build_controller_rpc_body(build_client_update_range_settings_payload()), False)`, i.e. `SerializeInt(170, 179)` plus packed 0, map 4, RPC block. Event `sent_client_update_range_settings`.
- Every pass: decode the packet with `decode_range_settings_requests` (range_client_rpc.py:197-292), fields 172-177. Each RPC must end exactly at its param size or it is dropped.
  - 174 and 175: no params.
  - 176: `[p]`; if 1, then `[armor][strafe][ammo]` bits, `i32 skill, plant, defuse`, `[practice]`, `i32 amount`. With `p = 0` the request is all defaults, so `infinite_ammo = False` (quirk 17).
  - 177: `[p][i32 distance]`, `[bot_enabled]`, `[init]`.
  - 172 and 173: `[p][object ref]`.
- Dedup on `range_settings_sequences_seen[s]`. Event `received_range_settings` (non-falsy fields only).
- On 176: `range_settings_state[s] = req`. If `range_infinite_ammo_sent.get(s) != req.infinite_ammo`: `send_reliable(4, build_game_state_bool_body(46, value), False)` (`rep = [0][packed 46][1 bit][packed 0]`, map 2, rep-layout block), and record it. Because the first comparison is None vs a bool, the **first 176 always sends**. Event `sent_infinite_ammo`.
- All other settings are decoded and logged only.

### 6.11 Command queue (`command_queue`, 1951-1982)

Every 0.2 s per source: list `local_backend/runtime/responder_queue/*.json` sorted by name. For each file: read the JSON, **unlink** it, then act:
- `{"spawn_bot": [x, y, z]}`: `bot_fields.setdefault(s, BotField(count))`. `bot = allocate(len(bots))`; the location is overwritten, but the yaw keeps the random placement. Append it, set the channel offset to 0, send open (`is_open`, exports) and health (exports), mark it opened. Event `bot_spawned`.
- `{"channel", "body_hex", "bits", "exports"?, "open"?}`: if `open`, `setdefault(ch, 0)`; then `send_reliable(ch, bytes.fromhex(body_hex), bits, bool(exports), is_open=bool(open))`. Event `sent_queued_commands`.

Errors are recorded per file (`queued_command_errors`). The directory is global, so the first source to poll consumes it. A queued `spawn_bot` does not touch `bot_spawned_at`, and the bot counts toward the `count` limit of 6.14. This is a debug hook: port it as a dev feature, not as protocol.

### 6.12 HUD refresh (`hud_refresh`, 1991-2023)

Gate: the reader, starting equippables done, `now - hud_refresh_polled >= 1.0` (polled = now), and `hud_refresh_counts < 6`.

Oracle: `controller = find_controller`, `pawn = [controller+0x4B0]`, `widget = [pawn+0xB80]` (Widget_BottomHUD). `collapsed = widget and byte [widget+0x11C] == 1`; `cinematic = u32 [controller+0x690] & 2`. On error, `(collapsed, cinematic) = (False, True)`.

If collapsed and not cinematic, two ch6 bunches with no exports, each map 6 and `append_actor_rep_layout_payload`:
1. `[0][packed 194][packed 0][packed 0]` (33 bits): SpawnedCharacter = null.
2. `build_object_property_payload(194, 10)`: SpawnedCharacter = pawn.

Then `count += 1` and event `sent_hud_refresh`.

### 6.13 Cloudburst, stage A (`ability_projectiles`, 1016-1100, 2026-2030)

`cloudburst_state` is called from the mirror for the Smoke ability every 50 ms (1016-1044):
- `projectile_fields.setdefault(s, ProjectileField())`; this is what enables the flight tick.
- It swaps `previous_smoke_state` and fires on the transition **into** `{ProjectileThrowState, CommitTime, AirBendingState}` (range_projectiles.py:39, 110-112).
- Oracle `read_camera`. Then `throw_transform(view_loc, pitch, yaw, forward=60.0)` (267-278):
  - pitch is normalised to [-180, 180);
  - `scale = 1 - 0.25 * min(|pitch|, 90) / 90`;
  - `pitch' = min(pitch + 8 * scale, 89.9)`;
  - `dir` is computed from (pitch', yaw);
  - `location = view + dir * 60`, with `z -= 8 * scale`;
  - `velocity = dir * 2900`.
- Allocation: `channel = ProjectileField.channel()` (10000+, at most 32766), then the actor GUID, then the movement GUID (610100+, step 2). Set the channel offset to 0.
- Send **open**, exports: `smoke_projectile_open_body(actor, loc, pitch', yaw, vel, movement_guid=movement, ignore_pawn=False)`.
- Append `Flight(channel, actor, movement, opened=now, loc, vel, pitch', yaw)`. Event `cloudburst_open`.

`smoke_projectile_open_body` (range_projectiles.py:182-222):
- Export count 2: archetype 610001 with package 610003; stable `FiniteSpeedMovement` (GUID `movement`, outer the actor).
- Spawn header: actor, archetype, level 5, `[1][1]`, quantized vector10 location, then rotation shorts `round(deg * 65536 / 360) & 0xFFFF` for pitch, yaw, 0 (per axis `[nonzero][16 bits]`, preceded by an any-nonzero bit), then 2 zero bits.
- Actor block: `rep = [0][packed 11]RepMovement[packed 12][packed 10][packed 14][packed 10][packed 0]`. RepMovement = 4 zero bits, scale-1 quantized location, rotator bytes `floor(deg * 256 / 360 + 0.5) & 0xFF` with presence bits, scale-1 quantized velocity.
- Then the FiniteSpeedMovement subobject block with `rep = [0][packed 0]`. Total must be < 8192 bits.

`tick_projectiles` (1046-1100) runs every 50 ms while `s in projectile_fields`. For each flight:
- While flying: `snapshot = read_projectile(reader, flight.channel)` (oracle, errors logged). `flight_stop_point` (346-360):
  - If the snapshot's class is `Projectile_Wushu_4_Smoke_C` and it has a location: mark it seen. With collisions >= 1, stop at `hit + normal * 65` ("hit"); else, 2.0 s after the open, stop at the current location ("timeout"); else keep flying.
  - Not readable now but seen before: stop at `last_location` ("lost").
  - Never seen and >= 0.3 s since the open: `ballistic_fallback` ("fallback"; range_projectiles.py:285-295). It accumulates `elapsed += 0.005` (float sum; do not use `i * 0.005`) while `elapsed < 2.0`, with `point = loc + vel*t + 0.5*(-1125)*t^2` on z only. At the first `z <= 1253.8` it returns `(x, y, 1318.8)`; if that never happens it returns the last point.
- On stop (stage A): `state = stopped`, `close_at = now`. Allocate the zone channel, then the zone GUID; set the offset to 0. Send **open**, exports: `smoke_zone_open_body(zone_guid, loc, random.uniform(0.0, 360.0))` (unseeded global `random`). Then `zone_close_at = now + 3.2`. Event `cloudburst_zone`.
- Same pass: when `now >= close_at`, close the projectile channel. When `now >= zone_close_at`, close the zone channel (`cloudburst_zone_closed`). Drop the flight once it is closed and the zone is closed.

`smoke_zone_open_body` (253-264) is `build_dynamic_actor_reference(zone 610005/610007, level 5, location, yaw)` plus an actor block `rep = [0][12][10][14][10][0]`.

### 6.14 Bots and spray-wall dummy (`range_bots`, 2034-2062)

Gate: `s in range_streaming_sent_sources` and `now - bot_ticks >= 0.1`.

`BotField(count=5)` (range_bots.py:117-157) holds `rng = random.Random(1306)`, `next_channel 400` and `next_guid 300100`. `allocate(index)` takes the channel, then GUIDs actor, actor+2 health, +4 handler, +6 BombTeam (stride 8), then `place()`.

`place()`: up to 64 attempts of `a = radians(rng.uniform(-30, 30))`, `d = rng.uniform(700, 1800)`, `x = 4825 + d sin a`, `y = -13600 - d cos a`. An attempt is accepted if it is >= 200 (XY) from every alive bot in `bots`. Fallback `(4825, -14600)`. `z = 1351.8`. `yaw = degrees(atan2(-13600 - y, 4825 - x))`. Team 0 (the player is team 1).

Per tick:
1. If there is no dummy: `allocate_dummy()` = `allocate(-1)` (this **consumes the placement draws**), then location (6190, -11640, 1270), yaw 180, life 1000. Offset 0. Send **open**, exports: `build_dummy_open` (TargetPracticeTrainingBot 300005/300007, level 5, location, yaw). Send health, exports: `build_dummy_health_body` (HealthDamageSection export; bAlive 1, Life 1000, Max 1000). Event `dummy_spawned`.
2. Retire: for every bot with `not alive and died_at is not None` and `now - died_at >= 1.5`, close its channel and remove it. Event `bot_retired`.
3. Spawn at most one: if `len(alive) < count` and `now - bot_spawned_at >= 0.3` (initially 0): `bot = allocate(len(bots))`, append, offset 0. Send **open**, exports: `build_bot_open`. Its export count is 2 (TrainingBot_PC archetype 300001 with package 300003; stable `BombTeam` with outer the bot), followed by the actor header (location, yaw) and a BombTeam block `rep = [0][packed 3][u8 team][packed 0]`. Then send health, exports (`bAlive 1, Life 100, Max 100`). Set `bot_spawned_at = now`. Event `bot_spawned`.

Damage-text RPC (range_bots.py:210-226): `SerializeInt(57, 59)`, packed 65, `[1][f64 damage]`, map dummy actor, RPC block, reliable, no exports.

### 6.15 Ability activation jobs (`ability_activation`, 2066-2078, 1130-1170)

Gate: `s in melee_inactive_state_sent_sources` and starting equippables done.

On the first gated pass: `alloc = ability_allocator()`, i.e. `PurchaseAllocator(next_channel=0, next_guid=100000)`. For `order, index in enumerate((2, 3, 4, 5))`: `plan_ability_load(index)` gives channel 12+index, actor 200+4*index, the slot from `wushu_starting_equippables`, and SM then Inactive GUIDs (dynamic). `next = now + 0.25 * order`.

Stages, all on `load.channel`:
- 0: `gun_role_body` shape (no exports), then `next = now + 0.4`.
- 1: `build_machine_open_body(actor, sm, "EquippableStateMachine")` (exports), then +0.4.
- 2: `build_auth_state_body(.., "InactiveState", True)` (exports), then +0.4.
- 3: `read_slot_machines([slot])`. The class must match; otherwise retry every 0.3 s and give up after 21 failures (`ability_register_gave_up`). On success:
  - `names` = the SM's state names except InactiveState (TMap order);
  - `resources` = the matching `starting_ability_equippables` entry's resources (Blade Storm has none);
  - `extra = plan_extra_machines` (allocates first);
  - `eq = ability_equippable(load, {n: alloc.dynamic_guid() for n in names}, resources, extra)`;
  - `send_extra_machines`;
  - `ability_ready_equippables[s][key] = eq` (key = class name, so the charge mirror state carries over);
  - event `ability_ready [class, len(names) + 1, extra names]`; stage 7.

The starting ability opens with AbilityTuning are in the setup chain (4045-4068), not here.

### 6.16 Movement packet capture (diagnostic, 2086-2106)

No sends. After `pawn_role_sent_sources`, it adds `decoded_packet_hex` to events for up to 300 ch8 packets (>= 0.25 s apart) and up to 16 distinct ch5 packets (by reliable sequence, or by second for unreliable ones). In Rust, port it as an optional trace-level packet dump.

## 7. Oracle reads (13.06; ReadProcessMemory today, the `internal` oracle later)

| Read | Chain / offsets | Used by |
|---|---|---|
| GEngine | `rws.OWNER_RVA = 0xD3E7140` GObjects owner -> `decode_array_pointer` -> chunks (count 1..256) -> object 0x107E2 (`range_authority_probe/read_world_state.py:10, 38-90`, a sibling of `local_backend`, imported by resolve_rpc_index.py:17-18) | everything |
| world / controller / pawn | engine+0xBD0 viewport, +0x80 world; world+0x1D8 game instance, +0x40 local players [0], +0x38 controller; controller+0x4B0 pawn (read_equippable_state.py:30-63) | all |
| slot machines | pawn+0xBF8 inventory; inv+0x278 current; inv+0x1B8+8*slot UItemSlot, +0x38 item; class name = FName u32 at `[item+0x10]+0x18` (the UClass's name, cached per item address); item+0xE00 SM; SM+0x160 states (24-byte elements; first u64 = state object; name at state+0x18), count SM+0x168 (<= 64); SM+0x1B0 predicted, +0x1C8 auth (read_equippable_state.py:132-164) | mirror, purchase stage 5, ability stage 3 |
| item machines | item+0xE30 array, +0xE38 count (<= 8); machine name at +0x18; starting state SM+0x1E8 (177-202) | extra machines |
| resource amounts | `[item + item_offset]`; name at +0x18 must equal the resource name; i32 predicted +0x10C, auth +0x110; default +0x12C (ammo) or +0x188 (charges) (212-236) | mirror |
| firing seed | `[FiringState + 0x4A0]` RNG component, `u32 +0xEC` (946-964) | bot shots |
| camera | controller+0x528 manager; 6 doubles at +0x17C0 (66-74) | bot shots, Cloudburst |
| input replication | pawn+0xB90 input; +0x12C stopped (u8), +0x130 requested (u32); inv+0x2F0 correction; inv+0x260 desired / +0x278 current -> class FName (247-268) | input-correction responder |
| input bounds | input+0xA84 cursor, +0x1198 count (<= 4096) (capture_input_event_cursor.py:42-65; opens a **new process handle per call**) | input ack |
| ult requirement | `i32 [pawn+0x14F4]` (1835-1837) | ultimate points |
| HUD widget | pawn+0xB80 widget, `u8 [widget+0x11C] == 1` Collapsed; `u32 [controller+0x690] & 2` cinematic (1999-2004) | HUD refresh |
| client channel actor | world+0x40 NetDriver, +0xC0 ServerConnection; conn+0x13F8 Channels, +0x1400 count; `[channel]+0x78` actor; actor+0x290 root, root+0x320 location (3 doubles); actor+0x610 FiniteSpeedMovement, +0x198 NumCollisions, +0x228 hit location (3 doubles), +0x258 hit normal (range_projectiles.py:308-343) | Cloudburst |
| ClientSetHUD field index | `resolve_rpc_index.resolve(thunk = image_base + 0x4B6B340, "controller")` (3784-3791) | Range HUD (via `hud_rpc_cache`) |

FName decoding: `read_fname_pool.decode_name` / `derive_name_key`.

## 8. Logging (`emit_event`) and the Rust console

- One dict `event` per received datagram (1463). Every stage adds keys. At the end it is appended to `events` (a `deque(maxlen=8192)`, written to `--output` at exit, 4581-4584) and printed as one JSON line with flush (4578, 1373-1376). `handler_elapsed_ms` is added when handling took >= 25 ms (4575-4577).
- Production prints **every** event, about 60-110 lines/s per client.
- Quiet mode (off) would print only events that have keys outside this routine set (1378-1393): `elapsed_seconds, source_port, length, ares_inspection, ciphertext_length, counter, stateless_header, payload_bits, control_prefix_hex, bunch_*, channel_index, has_exports, has_must_map, header_flag_4, channel_sequence*, hardcoded_name, channel_name_index, control_message_type, sent_session_ack_length, acked_client_sequence, server_sequence, sent_move_ack_length, move_ack_*, decoded_packet_hex, client_packet_sequence`. It adds a 60 s status line.
- Authority-loop event keys: `equippable_reader_error, equippable_mirror_error, resource_mirror_error, machine_mirror_error, sent_equippable_auth_states, bot_camera_error, dummy_hits, bot_shot_missed, bot_hits, cloudburst_camera_error, cloudburst_open, cloudburst_read_error, cloudburst_zone, cloudburst_zone_closed, input_correction_error, sent_input_resume_correction, input_resume_equippable, sent_range_mode_hud, sent_range_streaming_levels, ultimate_points_error, sent_ultimate_points, purchase_decode_error, received_purchases, purchase_unsupported, purchase_resolved_path, purchase_planned, purchase_attachments_exported, purchase_cosmetic_error, purchase_opened, purchase_slotted, purchase_replaced, purchase_shield, purchase_register_error, purchase_register_gave_up, purchase_extra_machines, purchase_ready, extra_machines_error, range_settings_decode_error, sent_client_update_range_settings, received_range_settings, sent_infinite_ammo, sent_queued_commands, queued_command_errors, bot_spawned, dummy_spawned, bot_retired, hud_refresh_error, sent_hud_refresh, ability_register_error, ability_register_gave_up, ability_ready, received_activation_events, input_ack_received_event_id, input_ack_received_frame_length, input_ack_rejected_event_id`.

Proposed console (user requirement):
- `AllocConsole` at startup. It fails (`ERROR_ACCESS_DENIED`) when the process already owns a console, for example when it is started from a terminal; treat that as success and keep the existing console. After allocation, reopen stdout/stderr on `CONOUT$` so the logger's writes reach it. Python's responder runs with `CREATE_NO_WINDOW` and stdout redirected to `runtime/responder_<pid>_phase4_match_state.out` (launch_range.py:288-291), so it has no console today.
- info: the authority keys above, minus the high-rate ones (`sent_equippable_auth_states`, `input_ack_*`, `received_activation_events`).
- debug: those high-rate keys and the per-packet routine keys.
- warn: any tick whose work exceeds 7.8125 ms (the analogue of `handler_elapsed_ms >= 25`), and every `*_error`.
- Keep a JSON-lines file sink with the same keys, so diffs against Python logs stay possible.

## 9. Rust mapping at 128 ticks per second

- **Timers.** Keep every threshold in seconds against one monotonic `now` sampled per tick. Compare with `>=` and set `last = now` when it fires, as Python does. Python's effective cadence was `max(threshold, packet gap)` with gaps of about 9-16 ms; at 7.8125 ms per tick a threshold T fires every `ceil(T * 128)` ticks:

| T | Ticks | Effective period |
|---|---|---|
| 0.05 (mirror, Cloudburst) | 7 | 54.7 ms |
| 0.1 (bots) | 13 | 101.6 ms |
| 0.2 (input-correction poll, command queue, gun stage 0a) | 26 | 203.1 ms |
| 0.25 (ability job stagger) | 32 | 250 ms |
| 0.3 (bot spawn spacing, register retries, unseen fallback) | 39 | 304.7 ms |
| 0.4 (ability stages) | 52 | 406.3 ms |
| 0.5 (purchase stages) | 64 | 500 ms |
| 1.0 (input-correction resend, HUD refresh, streaming delay) | 128 | 1 s |
| 1.5 (bot retire) | 192 | |
| 2.0 (projectile timeout) | 256 | |
| 3.0 (settings request) | 384 | |
| 3.2 (zone lifetime) | 410 | |
| 4.0 (charge refill) | 512 | |

- **Receive vs tick.** Decode each authenticated packet **once** on receipt: input streams (first bunch on ch8), ServerPurchase and settings RPCs (all ch5 bunches). Python AES-decodes the same datagram up to five times: inspection, input ack, purchases, settings and movement capture. Queue the facts for the next tick, and keep the per-source dedup sets. Evaluate each feature's gate **at receipt**, as Python does: a ServerPurchase that arrives before "starting equippables done", or a settings RPC that arrives before streaming was sent, is dropped (and its sequence is never added to the dedup set). Queueing it and applying it later would differ from Python (quirk 2).
- **Exact thresholds.** Thresholds whose `T * 128` is an integer (0.25, 0.5, 1.0, 1.5, 2.0, 3.0, 4.0) sit exactly on a tick boundary. With a wall-clock `now` that jitters by microseconds, `now - last >= T` can miss by one tick. Keep a monotonic tick counter and compare whole ticks (`ticks - last_tick >= ceil(T * 128)`, with deadlines `next_tick = tick + ceil(dt * 128)`), so the periods in the table above are exact.
- **Frame fields.** Python stamps each frame with the triggering packet's sequence and mask. A tick-driven send should use the latest authenticated packet (sequence, stateless header, mask), as `send_due_server_heartbeats` does (1421-1428).
- **Order.** Keep the order from section 2 within a tick, and only advance per-channel offsets when a bunch is really sent.
- **Oracle.** Reads must not block the tick. Use a reader thread or the `internal` oracle pushing snapshots: slot machines, resources, extra machines, firing seeds, camera, input replication, input bounds, HUD widget, ult requirement, projectile snapshots.
- **Parity scope.** Bit-exact targets are bunch bodies, bunch header fields (channel, reliable sequence, open/close/exports) and per-channel send order. Server packet sequence and ack fields depend on timing and cannot be diffed exactly.
- **Retransmission.** If the Rust channel manager adds retransmission, a resend must reuse the original channel sequence and body. First transmissions stay bit-identical.
- **Firing seed.** Polling the seed every tick instead of every 50 ms would register more shots than Python. Keep 50 ms for parity unless that behaviour change is approved.
- **Server performance RPC.** The (dead) `build_server_performance_rpc_payload` defaults to 7.8125 ms, which is 128 Hz. Once its field index is verified, report the real tick time.

## 10. Quirks (keep for parity or decide explicitly)

1. Dedup by 12-bit reliable sequence (purchases, settings) wrongly drops a new request whose sequence matches one seen 4096 reliable ch5 bunches earlier.
2. Purchases and settings arriving before their gates are acked by the session ACK and lost (the client does not resend).
3. Ultimate points re-read the pawn every authenticated packet until a value in 1..49 appears (no throttle).
4. A second armor purchase opens a new armor actor and leaves the old one open. Only the slot contents change.
5. Gun stage 0: if every candidate open raises ValueError, `open_body` is unbound and NameError escapes `run_probe` (1242-1248). This is unreachable in practice: the unskinned candidate fits under 7600 bits for every listed gun (test_range_attachments.py:204).
6. Python's `round()` is round-half-even: rotation shorts (range_server_bunch.py:186, range_projectiles.py:172). Quantized vectors (`floor(x + 0.5)` / `ceil(x - 0.5)`) and rotator bytes (`floor(x + 0.5)`) are not. Rust must use `round_ties_even` for the shorts only.
7. Bot placement needs CPython's MT19937 seeded with `init_by_array([1306])`: `random() = ((a >> 5) * 67108864 + (b >> 6)) / 2^53`, `uniform(a, b) = a + (b - a) * random()`. The dummy consumes the first placement. `math.degrees` / `math.radians` multiply by `180/pi` / `pi/180` (same as Rust `to_degrees` / `to_radians`). sin/cos/atan2 come from the platform libm; only the quantized (x10 and 65536/360) results reach the wire. Vectors are in 11.
8. The smoke-zone yaw uses the unseeded global `random`, so tests must inject the yaw.
9. Reader caches are keyed by raw address and never invalidated (stale names after reuse of a freed address).
10. `channel_first_dynamic_offset` is global, so a queued `"open"` command's `setdefault` persists across sources.
11. The ability-job and purchase-job lists are never pruned; done jobs stay at stage 7.
12. A queued `spawn_bot` keeps the random yaw while overriding the location.
13. `bot_shots` uses the camera at poll time, not at shot time. Several shots inside one 50 ms poll count as one.
14. The `/dev/range/enter` respawn path (`backend.py:57-66, 86`) launches `launch_range.py` without `--base-pistol-initial-cosmetic-trial`, so a respawned responder opens the Classic unskinned while a `launch.ps1` start opens it with the saved skin. Decide which one the Rust server reproduces.
15. Command queue: `json.loads` runs before `unlink` (1956-1957). A file that does not parse stays in the directory and logs a `queued_command_errors` entry every 0.2 s until it is removed by hand.
16. The firing seed is sampled only while the item is current (1624). Any seed change while it is not current is counted as one shot on the first poll after it becomes current again.
17. A `176` request whose struct presence bit is 0 decodes as all defaults, so `infinite_ammo = False`; if the last sent value was True (or nothing was sent yet), a `false` GameState handle-46 update goes out.
18. Client packets whose ack header carries extra history words are misparsed by Python's first-bunch inspector and by `iter_bunches` (see `06_client_decode.md`, around line 192): no input ack, purchase or settings RPC is decoded from them, though they are acked. They come in bursts after the client goes > 32 packets without an ack.

## 11. Test vectors

All vectors were generated offline by importing the Python builders (no sockets, no live process). Hex is the `BitWriter.to_bytes()` little-endian bit order; `bits` is the exact bit count. "body" means what `send_reliable` receives as `(body, bits)`.

### Mirror: AuthCurrentState (build_auth_state_body)

BasePistol: actor 94, SM 96, states range_server_bunch.py:757. Melee: actor 204, SM 206, states range_melee_equippable.py:51.

```
auth_state.pistol.EquipState.export  bits=261
  020000008c0b7801160000008ae2ead2e0a6e8c2e8ca0000000000022ec3a10b05
auth_state.pistol.IdleState.export  bits=269
  02000000ca050a78011400000092c8d8caa6e8c2e8ca0000000000022e647d89b80f
auth_state.pistol.IdleState.noexport  bits=52
  0117b2be44dc07
auth_state.pistol.InactiveState.noexport  bits=44
  0197e1908302
auth_state.melee.EquipState.export  bits=293
  020000000a080a320500160000008ae2ead2e0a6e8c2e8ca0000000000ea142864edf11115
auth_state.melee.InactiveState.noexport  bits=60
  750a14b2f6488f0a
```

### Mirror: AuthResourceAmount (build_resource_amount_body)

Ability `*.export` amounts are the first-send seeds (MaxCharges at +0x188): Smoke 2, Q 1, E 1. Entries from `starting_ability_resources()`: Smoke actor 208 / GUID 310, Q actor 216 / GUID 312, E actor 212 / GUID 314.

```
resource.pistol.MagazineAmmo.12.export  bits=317
  02000000d2080a78011a0000009ac2cec2f4d2dcca82dadade00000000004a232866dd4a6e851d10
resource.pistol.ReserveAmmo.36.export  bits=309
  02000000ca080a780118000000a4cae6cae4ecca82dadade00000000002a232866dd0a66851d10
resource.pistol.MagazineAmmo.11.noexport  bits=76
  a51114b36e95b7c20e08
resource.Ability_Wushu_4_Smoke_C.EquipmentCharge.2.export  bits=349
  02000000da080a420500200000008ae2ead2e0dacadce886d0c2e4ceca00000000006a2328666da2ed9b6e1a
resource.Ability_Wushu_4_Smoke_C.EquipmentCharge.0.noexport  bits=76
  b51114b336b9f64d370d
resource.Ability_Wushu_Q_CycloneBoost_C.EquipmentCharge.1.export  bits=349
  02000000e2080a620500200000008ae2ead2e0dacadce886d0c2e4ceca00000000008a232866ed921e06d312
resource.Ability_Wushu_Q_CycloneBoost_C.EquipmentCharge.0.noexport  bits=76
  c51114b376b90e836909
resource.Ability_Wushu_E_Dash_C.SignatureAbilityResource.1.export  bits=421
  02000000ea080a52050032000000a6d2cedcc2e8eae4ca82c4d2d8d2e8f2a4cae6deeae4c6ca0000000000aa2328667d920e563c16
resource.Ability_Wushu_E_Dash_C.SignatureAbilityResource.0.noexport  bits=76
  d51114b33eb9062b1e0b
```

### Inventory correction (build_inventory_correction_body, pawn ch8)

```
inventory_correction.6.guid94  bits=84
  e1150c230b43b8cb7aa60f
inventory_correction.7.none  bits=68
  e115b3c6c523c59000
inventory_correction.8.guid520  bits=92
  e1950cc30f43c94419c20709
inventory_correction.9.guid200  bits=92
  e1950c3b0e43c944198a0509
```

### Range HUD (ch4 x2 with exports, then ch5)

`*.payload` is the plain RPC payload before encode_short_actor_rep_payload; `*.body` is what send_reliable receives.

```
range_hud.class_property.ShootingRangeGameMode_C  bits=868
  0200000016080a1e080a00680000005e8ec2daca5e8ec2daca9adec8cae65ea6d0dedee8d2dccea4c2dcceca5ea6d0dedee8d2dccea4c2dcceca8ec2daca9adec8ca000000000030000000a6d0dedee8d2dccea4c2dcceca8ec2daca9adec8cabe86000000000016b22e94a902
range_hud.class_property.ShootingRangeHUDConfig_C  bits=964
  0200000026080a2e080a007e0000005e8ec2daca5eaa925e92dc8ec2daca5e90aa885e8ec2dacaa6e8c2e8ca90aa8886dedcccd2cee65ea6d0dedee8d2dccea4c2dcceca90aa8886dedcccd2ce000000000032000000a6d0dedee8d2dccea4c2dcceca90aa8886dedcccd2cebe86000000000016b22e640e05
range_hud.client_set_hud.payload  bits=24
  3489da
range_hud.client_set_hud.body  bits=34
  c2a88cad03
```

### Level streaming (ch5, two RPC bodies)

```
streaming.update_multiple_levels.payload  bits=1058
  c20b880200500000005e8ec2daca5e9ac2e0e65ea0deeccaced8d2c25ea4c2dccecabe9edcd8f2be8ec2dacae0d8c2f20000000000feffffff870f0000c0cb51585bd90b539a5dd914dd9b5cde0b539a5dd914dd9b5cde1754995c5b985b991bdd8b5498db59d917539a5dd914dd9b5cde1754995c5b985b991b1d00000000c0ffffffff00
streaming.update_multiple_levels.body  bits=1076
  1641108685b58ff5d07e4e5537a9c973e9e0f1254a5fb1042396696a0dfa39d0c9b9c52f9383c0d60f306b8c3572513bbaf64ab54f9e5f4b8500c526b6b901a643bb14adcb90313b618654e1cc4d226b915cd5569d6d2be4b1d3a58355bb9ef88a15af973bf5c4679f46700b98bc70930913db5c6ce927d8fc8c99af6c93b5e6f0738f2090a20a
streaming.flush.payload  bits=16
  1a00
streaming.flush.body  bits=26
  82c4c202
```

### Ultimate points (ch6, handle 197)

```
ultimate_points.8  bits=67
  cb598fb04dcb7a4c07
ultimate_points.7  bits=67
  cb59cbb24dcb7a4c07
```

### Range settings (ch5 request, ch4 infinite ammo)

```
settings.client_update_range_settings.payload  bits=16
  aa00
settings.client_update_range_settings.body  bits=26
  8290c002
settings.infinite_ammo.true  bits=28
  9350d10f
settings.infinite_ammo.false  bits=28
  9350f10d
```

### HUD refresh (ch6 SpawnedCharacter null, then pawn)

```
hud_refresh.null.payload  bits=33
  0a05000000
hud_refresh.null.body  bits=43
  0b59d7a28e01
hud_refresh.pawn.payload  bits=33
  0a05280000
hud_refresh.pawn.body  bits=43
  0b5953a08e01
```

### Input ack (ClientAckInputAndMovement, unreliable ch8)

```
input_ack.1.payload  bits=80
  29e10000008000000000
input_ack.1.body  bits=90
  825e85856534a5e984b10e00
input_ack.300.payload  bits=80
  29619600008000000000
input_ack.300.body  bits=90
  825e85856554a5e9e8b20e00
input_ack.70000.payload  bits=80
  2961b888008000000000
input_ack.70000.body  bits=90
  825e85856554a5cd30b10e00
```

### Purchase: Vandal on a fresh PurchaseAllocator (gun job, all stages)

Allocation: channel 20, actor 520, archetype 523, package 525 (the live client echoed `<guid:525>`), SM 528, MagazineAmmo 530, ReserveAmmo 532, cosmetic pairs [[535, 537], [539, 541], [543, 545]], attachment pairs [[547, 549]]; allocator after plan = (next_channel 21, next_guid 551). Stage 3 allocates InactiveState = 552.

```
purchase.vandal.stage0.attachment_pre_export  bits=1305
  020000008e100a96100a00b80000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e82965e82e8e8c2c6d0dacadce8e65ea4caccd8caf05ea4cac888dee85e82e8e8c2c6d0dacadce8be8296bea4caccd8caf0a0e4d2dac2e4f282e6e6cae800000000005800000088caccc2ead8e8bebe82e8e8c2c6d0dacadce8be8296bea4caccd8caf0a0e4d2dac2e4f282e6e6cae8be86000000000000
purchase.vandal.stage0.open.unskinned  bits=1144
  020000002e100a36100a00620000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e82965e82e6e6c2ead8e8a4d2ccd8cabe829600000000003600000088caccc2ead8e8bebe82e6e6c2ead8e8a4d2ccd8cabe8296be86000000000022102e1014e00182f8d46ea01f6c85b002061137cf2d7b889e351ee6012ddca232bca453034ef3c8
purchase.vandal.stage0.owner  bits=35
  cb94985a05
purchase.vandal.stage1.role  bits=30
  a37c832f
purchase.vandal.stage2.state_machine_open  bits=365
  0200000042100a2210002e0000008ae2ead2e0e0c2c4d8caa6e8c2e8ca9ac2c6d0d2dcca00000000000a4128f109
purchase.vandal.stage3.initial_state  bits=317
  02000000a2100a2210001c00000092dcc2c6e8d2eccaa6e8c2e8ca00000000000a4128644d421506
purchase.vandal.stage4.slot_contents  bits=70
  f1a1074fc8da042028
purchase.vandal.stage4.correction6  bits=92
  e1950c130b43c94419c20709
```

### Purchase: Vandal open with the Standard skin (standard_cdo_paths, skin_catalog.json)

Skin `Default__AssaultRifle_AK_Standard_PrimaryAsset_C` (`/Game/Equippables/Guns/Rifles/AK/Standard/AssaultRifle_AK_Standard_PrimaryAsset`) 27f21d97-4c4b-bd1c-1f08-31830ab0be84; level `Default__AssaultRifle_AK_Standard_Lv1_PrimaryAsset_C` 1ab72e66-4da3-33a0-164f-908113e075a4; chroma `Default__AssaultRifle_AK_Standard_Standard_PrimaryAsset_C` 19629ae1-4996-ae98-7742-24a240d41f99. Depends on local_backend/skin_catalog.json.

```
purchase.vandal.stage0.open.standard_skin  bits=5688
  080000002e100a36100a00620000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e82965e82e6e6c2ead8e8a4d2ccd8cabe829600000000003600000088caccc2ead8e8bebe82e6e6c2ead8e8a4d2ccd8cabe8296be8600000000005e100a66100a00a00000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e82965ea6e8c2dcc8c2e4c85e82e6e6c2ead8e8a4d2ccd8cabe8296bea6e8c2dcc8c2e4c8bea0e4d2dac2e4f282e6e6cae800000000006200000088caccc2ead8e8bebe82e6e6c2ead8e8a4d2ccd8cabe8296bea6e8c2dcc8c2e4c8bea0e4d2dac2e4f282e6e6cae8be8600000000006e100a76100a00a80000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e82965ea6e8c2dcc8c2e4c85e82e6e6c2ead8e8a4d2ccd8cabe8296bea6e8c2dcc8c2e4c8be98ec62bea0e4d2dac2e4f282e6e6cae800000000006a00000088caccc2ead8e8bebe82e6e6c2ead8e8a4d2ccd8cabe8296bea6e8c2dcc8c2e4c8be98ec62bea0e4d2dac2e4f282e6e6cae8be8600000000007e100a86100a00d40000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e82965ea6e8c2dcc8c2e4c85e86d0e4dedac2e65ea6e8c2dcc8c2e4c85e82e6e6c2ead8e8a4d2ccd8cabe8296bea6e8c2dcc8c2e4c8bea6e8c2dcc8c2e4c8bea0e4d2dac2e4f282e6e6cae800000000007400000088caccc2ead8e8bebe82e6e6c2ead8e8a4d2ccd8cabe8296bea6e8c2dcc8c2e4c8bea6e8c2dcc8c2e4c8bea0e4d2dac2e4f282e6e6cae8be86000000000022102e1014e02986fc11bf80eeb95afc2de0911d761aac11c3065f8865f577e4f64a6c04f19915e84e32cbcd3cdab0b9ac08f099da9afe5fe51b0692977ccce5146a202cce5f758187767216535861c8ce87ee80e66ef9d8ed320b4b76fc8b4885bb81d83e57a6927ea4211fc4
```

### Purchase: stage-5 registration and mirror bodies (same allocator, after stage 3)

Illustrative registration: ZoomStateMachine planned first (machine 554, states {'ZoomStateInactive': 556, 'ZoomStateUnzoomed': 558, 'ZoomStateLevel1': 560}), then main states in name order {'InactiveState': 552, 'EquipState': 562, 'IdleState': 564, 'FiringState': 566, 'ReloadState': 568}. Allocator after: [21, 570]. Real state names and order come from the oracle.

```
purchase.vandal.stage5.extra_machine_open  bits=317
  02000000aa100a22100022000000b4dededaa6e8c2e8ca9ac2c6d0d2dcca0000000000aa4228f109
purchase.vandal.stage5.extra_machine_initial  bits=349
  02000000b2100a22100024000000b4dededaa6e8c2e8ca92dcc2c6e8d2ecca0000000000aa4228644d121806
purchase.vandal.mirror.EquipState.export  bits=293
  02000000ca100a221000160000008ae2ead2e0a6e8c2e8ca00000000000a4128644df21d06
purchase.vandal.mirror.zoom.Unzoomed.export  bits=349
  02000000ba100a22100024000000b4dededaa6e8c2e8caaadcf4dededacac80000000000aa422864dd441d06
purchase.vandal.mirror.MagazineAmmo.25.export  bits=325
  020000004a100a2210001a0000009ac2cec2f4d2dcca82dadade00000000002a412866ad2bfa418519
```

### Purchase: Sheriff as the second purchase on the same allocator (no attachments)

Allocation: channel 21, actor 570, archetype 573, package 575, SM 578, mag 580, reserve 582, cosmetic pairs [[585, 587], [589, 591], [593, 595]].

```
purchase.sheriff.stage0.open.unskinned  bits=917
  02000000f6100afe100a00700000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6d2c8cac2e4dae65ea4caecded8eccae45ea4caecded8eccae4a0d2e6e8ded800000000003400000088caccc2ead8e8bebea4caecded8eccae4a0d2e6e8ded8be860000000000ea10f6101400
purchase.sheriff.stage4.slot_contents  bits=70
  01a2074fc8dae43d28
```

### Purchase: Warden on a fresh allocator

Allocation: channel 20, actor 520, archetype 523, package 525, attachment pairs [[547, 549]].

```
purchase.warden.stage0.attachment_pre_export  bits=1569
  020000008e100a96100a00e00000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e84c2e8e8d8caa4d2ccd8ca5e82e8e8c2c6d0dacadce8e65ea4caccd8caf05e82c6dece5e82e8e8c2c6d0dacadce8be84c2e8e8d8caa4d2ccd8cabe82869e8ea6c6dee0cabea0e4d2dac2e4f282e6e6cae800000000007200000088caccc2ead8e8bebe82e8e8c2c6d0dacadce8be84c2e8e8d8caa4d2ccd8cabe82869e8ea6c6dee0cabea0e4d2dac2e4f282e6e6cae8be86000000000000
purchase.warden.stage0.open.unskinned  bits=1152
  020000002e100a36100a006c0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e84c2e8e8d8caa4d2ccd8ca5e84c2e8e8d8caa4d2ccd8ca00000000002e00000088caccc2ead8e8bebe84c2e8e8d8caa4d2ccd8cabe86000000000022102e1014e00182f8d46ea01f6c85b0ce241159cf2d7688a9b5316376ff20bba8b8a4d35dd7dfbd
```

### Purchase: Heavy Shields (armor job) on a fresh allocator

Allocation: channel 20, actor 520, archetype 523, package 525, slot 11.

```
purchase.heavy_armor.stage0.open  bits=677
  020000002e100a36100a00340000005e8ec2daca5e8ecac2e45e90cac2ecf282e4dadee492e8cada00000000003400000088caccc2ead8e8bebe90cac2ecf282e4dadee492e8cadabe86000000000022102e101400
purchase.heavy_armor.stage0.owner  bits=35
  cb94985a05
purchase.heavy_armor.stage1.slot_contents  bits=70
  a1a2074fc8da042028
purchase.armor.stage2.shield.50.export  bits=422
  02000000f2080a280028000000a6d0d2cad8c888c2dac2cecaa6cac6e8d2dedc0000000000ca23481ca415069ec91716f687ca5229
purchase.armor.stage2.shield.25.noexport  bits=125
  e511247ed60a03cfe40b0bfb5361a914
```

### Abilities (ability_setup_bodies, fresh ability_allocator, plans made in ability_indices order)

```
ability.2 Ability_Wushu_4_Smoke_C channel=14 actor=208 slot=3 sm=100000 inactive=100002
  stage0 has_exports=False bits=30
    a3807b08
  stage1 has_exports=True bits=381
    020000008236180a4205002e0000008ae2ead2e0e0c2c4d8caa6e8c2e8ca9ac2c6d0d2dcca00000000000ada60281101
  stage2 has_exports=True bits=341
    020000008a36180a4205001c00000092dcc2c6e8d2eccaa6e8c2e8ca00000000000ada6028e539d0f80113
ability.3 Ability_Wushu_E_Dash_C channel=15 actor=212 slot=5 sm=100004 inactive=100006
  stage0 has_exports=False bits=30
    a360452e
  stage1 has_exports=True bits=381
    020000009236180a5205002e0000008ae2ead2e0e0c2c4d8caa6e8c2e8ca9ac2c6d0d2dcca00000000004ada60287110
  stage2 has_exports=True bits=341
    020000009a36180a5205001c00000092dcc2c6e8d2eccaa6e8c2e8ca00000000004ada6028e529407bce18
ability.4 Ability_Wushu_Q_CycloneBoost_C channel=16 actor=216 slot=4 sm=100008 inactive=100010
  stage0 has_exports=False bits=30
    a3b03f3c
  stage1 has_exports=True bits=381
    02000000a236180a6205002e0000008ae2ead2e0e0c2c4d8caa6e8c2e8ca9ac2c6d0d2dcca00000000008ada6028d11c
  stage2 has_exports=True bits=341
    02000000aa36180a6205001c00000092dcc2c6e8d2eccaa6e8c2e8ca00000000008ada6028e5b990dc2404
ability.5 Ability_Wushu_X_Dagger_Production_C channel=17 actor=220 slot=9 sm=100012 inactive=100014
  stage0 has_exports=False bits=30
    a324f138
  stage1 has_exports=True bits=381
    02000000b236180a7205002e0000008ae2ead2e0e0c2c4d8caa6e8c2e8ca9ac2c6d0d2dcca0000000000cada60285110
  stage2 has_exports=True bits=341
    02000000ba36180a7205001c00000092dcc2c6e8d2eccaa6e8c2e8ca0000000000cada6028e5a9d03a500c
allocator after the four plans: next_guid=100016
```

### Bots: RNG and allocation (BotField(count=5), Random(1306))

```
random.Random(1306).getrandbits(32) x8: 3373406805, 3418166518, 2062407454, 2377898186, 420059223, 4164481652, 722410666, 3889646494
random.Random(1306).random() x4: 0.785432478754624 (0x1.922434b2ef453p-1), 0.480191652131435 (0x1.ebb75c46ddec6p-2), 0.09780266113295355 (0x1.909985f071e48p-4), 0.16819934476635523 (0x1.5878e5e7d74b8p-3)
dummy: channel=400 actor=300100 health=300102 handler=300104 team=300106 location=[6190.0, -11640.0, 1270.0] yaw=180.0 (its placement draws are consumed, then overwritten)
bot0: channel=401 actor=300108 health=300110 handler=300112 team_guid=300114 team=0 location=(4463.170774427629, -14407.674895086804, 1351.8) yaw=65.86815966797721
bot1: channel=402 actor=300116 health=300118 handler=300120 team_guid=300122 team=0 location=(4572.564072216778, -14792.2698646287, 1351.8) yaw=78.04547395567045
bot2: channel=403 actor=300124 health=300126 handler=300128 team_guid=300130 team=0 location=(4795.468318650681, -14628.70432286504, 1351.8) yaw=88.35562457025777
bot3: channel=404 actor=300132 health=300134 handler=300136 team_guid=300138 team=0 location=(5186.609028363901, -14270.230718226876, 1351.8) yaw=118.34815636375865
bot4: channel=405 actor=300140 health=300142 handler=300144 team_guid=300146 team=0 location=(4754.492781286616, -14341.763633110617, 1351.8) yaw=84.5701504353565
respawn after bot0 retired: index=4 channel=406 actor=300148 location=(4972.203165679736, -15364.057498008775, 1351.8) yaw=94.77004029843086
```

### Bots: bodies

```
bots.dummy.open  bits=1030
  02000000969f480a9e9f480a005a0000005e8ec2daca5e90cae4decae65e88cac6def25ea8c2e4cecae8a0e4c2c6e8d2c6caa8e4c2d2dcd2dcce84dee800000000004a00000088caccc2ead8e8bebea8c2e4cecae8a0e4c2c6e8d2c6caa8e4c2d2dcd2dcce84dee8be86000000000012a348969f48149632c7039523670c050004
bots.dummy.health.export  bits=454
  020000001aa3480a12a348002800000090cac2d8e8d088c2dac2cecaa6cac6e8d2dedc00000000006a8c2249acac1dc6d880ab43a6b03cb832
bots.dummy.damage_text.26  bits=89
  7ad69d85217523cf93e51900
bots.dummy.damage_text.160  bits=89
  7ad69d85217523cf93f51b00
bots.0.open  bits=1197
  04000000869f480a8e9f480a00580000005e8ec2daca5e86d0c2e4c2c6e8cae4e65ea8e4c2d2dcd2dcce84dee85ea8e4c2d2dcd2dcce84dee8bea08600000000003400000088caccc2ead8e8bebea8e4c2d2dcd2dcce84dee8bea086be8600000000004aa3480a32a348001200000084dedac4a8cac2da000000000032a348869f48149e62b962a6b9ce34e8b50b2a8d22297311b70f
bots.0.health.export  bits=454
  020000003aa3480a32a348002800000090cac2d8e8d088c2dac2cecaa6cac6e8d2dedc0000000000ea8c22492cc015c6da50a643760f36d007
bots.1.open  bits=1197
  04000000869f480a8e9f480a00580000005e8ec2daca5e86d0c2e4c2c6e8cae4e65ea8e4c2d2dcd2dcce84dee85ea8e4c2d2dcd2dcce84dee8bea08600000000003400000088caccc2ead8e8bebea8e4c2d2dcd2dcce84dee8bea086be8600000000006aa3480a52a348001200000084dedac4a8cac2da000000000052a348869f48149e7acaa2c5b7ce3428e00daa8d2229d3dc4217
bots.2.open  bits=1197
  04000000869f480a8e9f480a00580000005e8ec2daca5e86d0c2e4c2c6e8cae4e65ea8e4c2d2dcd2dcce84dee85ea8e4c2d2dcd2dcce84dee8bea08600000000003400000088caccc2ead8e8bebea8e4c2d2dcd2dcce84dee8bea086be8600000000008aa3480a72a348001200000084dedac4a8cac2da000000000072a348869f48149e4eed2292b8ce3468b50f2a8e2229131b6705
bots.3.open  bits=1197
  04000000869f480a8e9f480a00580000005e8ec2daca5e86d0c2e4c2c6e8cae4e65ea8e4c2d2dcd2dcce84dee85ea8e4c2d2dcd2dcce84dee8bea08600000000003400000088caccc2ead8e8bebea8e4c2d2dcd2dcce84dee8bea086be860000000000aaa3480a92a348001200000084dedac4a8cac2da000000000092a348869f48149e6a2a4352bace34680a15aa8e222953581c05
bots.4.open  bits=1197
  04000000869f480a8e9f480a00580000005e8ec2daca5e86d0c2e4c2c6e8cae4e65ea8e4c2d2dcd2dcce84dee85ea8e4c2d2dcd2dcce84dee8bea08600000000003400000088caccc2ead8e8bebea8e4c2d2dcd2dcce84dee8bea086be860000000000caa3480ab2a348001200000084dedac4a8cac2da0000000000b2a348869f48149ee6e6c2f8b9ce3428090f2a8f22293327540c
bots.0.health.74.noexport  bits=133
  75469124e6340c636d28d321bb071be803
bots.0.health.dead.noexport  bits=133
  75469124ae3b0b631e29d321bb071be803
bots.0.alive_false.export  bits=398
  0200000042a3480a32a348002e00000088c2dac2ceca90c2dcc8d8cae486dedae0dedccadce800000000000a8d2249c21205
bots.respawn.open  bits=1197
  04000000869f480a8e9f480a00580000005e8ec2daca5e86d0c2e4c2c6e8cae4e65ea8e4c2d2dcd2dcce84dee85ea8e4c2d2dcd2dcce84dee8bea08600000000003400000088caccc2ead8e8bebea8e4c2d2dcd2dcce84dee8bea086be860000000000eaa3480ad2a348001200000084dedac4a8cac2da0000000000d2a348869f48149eea08e3fab4ce3428d910aa8f222983542c19
```

### Cloudburst stage A

Camera view location [4825.0, -14600.0, 1415.8], rotation [5.0, -90.0, 0.0] -> throw_transform(forward=60): location (4825.0, -14658.488268184772, 1421.2947759584595), pitch 12.88888888888889, yaw -90.0, velocity (1.7309972018203743e-13, -2826.932962263996, 646.877134288504). Projectile channel 10000, actor 610100, movement 610102; zone channel 10001, zone GUID 610104 at (5000.0, -15000.0, 1318.8). Production zone yaw is random.uniform(0, 360) (unseeded).

```
cloudburst.projectile_open.stageA  bits=1689
  040000004677940a4e77940a007a0000005e8ec2daca5e86d0c2e4c2c6e8cae4e65eaeeae6d0ea5ea6605e82c4d2d8d2e8f2be685ea0e4ded4cac6e8d2d8cabeaeeae6d0eabe68bea6daded6ca00000000004800000088caccc2ead8e8bebea0e4ded4cac6e8d2d8cabeaeeae6d0eabe68bea6daded6cabe860000000000da7a940ad27a9400280000008cd2dcd2e8caa6e0cacac89adeeccadacadce80000000000d27a94467794149eeaf1e26cb885375825210030ae1278bdf672847d8768726f462f623c17270106230aa2a9b61ea5125601
cloudburst.zone_open.yaw123  bits=1268
  020000005677940a5e77940a00820000005e8ec2daca5e86d0c2e4c2c6e8cae4e65eaeeae6d0ea5ea6605e82c4d2d8d2e8f2be685e8ec2daca9ec4d4cac6e8beaeeae6d0eabe68bea6daded6cab4dedcca00000000005000000088caccc2ead8e8bebe8ec2daca9ec4d4cac6e8beaeeae6d0eabe68bea6daded6cab4dedccabe860000000000e27a94567794149e420d03c2b68433e8dd1596a29f2ea54303
cloudburst.zone_open.yaw0  bits=1249
  020000005677940a5e77940a00820000005e8ec2daca5e86d0c2e4c2c6e8cae4e65eaeeae6d0ea5ea6605e82c4d2d8d2e8f2be685e8ec2daca9ec4d4cac6e8beaeeae6d0eabe68bea6daded6cab4dedcca00000000005000000088caccc2ead8e8bebe8ec2daca9ec4d4cac6e8beaeeae6d0eabe68bea6daded6cab4dedccabe860000000000e27a94567794149e420d03c2b68433c052f4d3a5746800
```

### Full frames (send_reliable and the unreliable input-ack framing)

Inputs: receive_key = bytes 00..1f, stateless_header 16, server_sequence 8330, acked_client_sequence 4132, ack_history_mask 0x0000FFFF, channel_name_index 102, base (initial_actor_channel_sequences) = 139. Bodies: `build_inventory_correction_body(6, 94)`, `build_auth_state_body(BasePistol, "EquipState", True)`, close `(b"", 0)`, `build_dummy_open` of a fresh `BotField(5).allocate_dummy()`, and the `input_ack.300.body` above. `plain` is the decoded packet handed to transform_short_packet; `frame` is the datagram (wrap_gcm uses counter 0 every time, so it is deterministic).

```
frame.ch8.correction6.guid94.seq_base+51  header={'channel_index': 8, 'channel_sequence': 190, 'is_open': False, 'is_close': False, 'is_reliable': True, 'has_exports': False}
  plain bits=212 40022982ffff0000011021c017325381f00a86918521dc653dd30f
  frame 0000000010000000c071a998eb2334b100edfcc7fa21ef90e1f4e52e50f8833f8820dfacd468eb3307
frame.ch9.pistol_equip_export.seq_base+5  header={'channel_index': 9, 'channel_sequence': 144, 'is_open': False, 'is_close': False, 'is_reliable': True, 'has_exports': True}
  plain bits=389 40022982ffff0000011025021232170401000000c605bc000b0000004571756970537461746500000000000197e1d08512
  frame 00000000100000000020dc9dc62334b100edfcc7faa16eddf1f4e5a66ed5f8594dfa24b170481c6f590220d8f07dc531bc981259f5c30d02df66904f9b5a0e
frame.ch20.close.seq_base+7  header={'channel_index': 20, 'channel_sequence': 146, 'is_open': False, 'is_close': True, 'is_reliable': True, 'has_exports': False}
  plain bits=132 40022982ffff0000015010052421330008
  frame 0000000010000000409042d8c92334b100edfcc7ba19897afcd025b67c3507
frame.ch400.dummy_open.seq_base+0  header={'channel_index': 400, 'channel_sequence': 139, 'is_open': True, 'is_close': False, 'is_reliable': True, 'has_exports': True}
  plain bits=1166 40022982ffff00000130430c6211321b1001000000cb4f2405cf4f2405002d0000002f47616d652f4865726f65732f4465636f792f5461726765745072616374696365547261696e696e67426f7400000000002500000044656661756c745f5f5461726765745072616374696365547261696e696e67426f745f430000000000895124cb4f240a4b99e381ca913386020022
  frame 000000001000000080c1aa55d82334b100edfcc73a0e047473d4654d4f81520fec3923b170b63c75f5748fe73e5c2a7f1a7c292b55f013cc177f6fb8a34e2ca4f1a2eca9fac90e8d17b5b53fd0f20f557df8356db85fe4391e3cc77a2f4e3b237d831d0f2fd3ddd125f2bde541c6cd70635ef270fa8ebb94c23abf2546da05bb52137155f8b723d19b5a217589a68bb0e4cb2f08855d3dfdd6ee731846b9e71c
frame.ch8.input_ack300.unreliable  header={'channel_index': 8, 'channel_sequence': 0, 'is_open': False, 'is_close': False, 'is_reliable': False, 'has_exports': False}
  plain bits=195 40022982ffff0000010008d002825e85856554a5e9e8b20e04
  frame 0000000010000000409b386ece2334b100edfcc7fac8079253e4a5ac9b150d2dc2193348039a03
```

### Inbound decode vectors

Captured Warden ServerPurchase (local_backend/fixtures/server_purchase_warden_packet.json):

```
decoded packet bits=883
  70e0ca72ffffffffc34c05f89accf5c263853ff6b0a2f1303e443490f6b7c1cf6f62d82c528570892e7ea0c3eeaa850a6b3cad92f9a426ae671d6eb19d7127cd9320a2e5f51b320f728552a5220f16f6fd8c03a4d5757d5c3c049a3ee8c81b35e0231e62d9d4dbbe9b0bc73bb23c64
decode_purchase_requests -> [{'field': 156, 'item_path': '/Game/Equippables/Guns/Rifles/BattleRifle/BattleRifle.BattleRifle_C', 'target_slot': 0, 'purchasable_type': 0, 'channel_sequence': 863}]
decode_range_settings_requests -> []
```

Synthetic settings packet (built with the inverse content transform, actor map value 4; two reliable ch5 bunches without the hardcoded-name bit; 176 then 177). Not a capture: use it for decoder unit tests only.

```
payload176 bits=157 b00b020900000000000000000000002000000000
payload177 bits=51 b1460100000004
decoded packet bits=395
  a008224effffffff014005487bafc01d81b1b030280ba02a3fd7f8baec1b229ad93ec67d4b05507b3d4033ebe1e379f3dd05
bunch {'start_bit': 76, 'is_control': False, 'is_reliable': True, 'is_partial': False, 'channel_index': 5, 'channel_sequence': 3945, 'body_start_bit': 117, 'end_bit': 292}
bunch {'start_bit': 292, 'is_control': False, 'is_reliable': True, 'is_partial': False, 'channel_index': 5, 'channel_sequence': 3946, 'body_start_bit': 333, 'end_bit': 394}
request {'field': 176, 'channel_sequence': 3945, 'infinite_ammo': True, 'bot_amount_index': 1}
request {'field': 177, 'channel_sequence': 3946, 'is_init_settings_flow': True}
decode_purchase_requests on the same packet -> []
```

### Policy traces

mirror_target on the BasePistol (state starts as MirrorState(); after each non-None target the caller adds it to `exported` and sets `auth`). Columns: predicted, is_current, target.

```
['InactiveState', True, 'EquipState']
['InactiveState', True, None]
['EquipState', True, None]
['IdleState', True, 'IdleState']
['IdleState', True, None]
['FiringState', True, 'FiringState']
['IdleState', True, 'IdleState']
['InactiveState', False, 'InactiveState']
```

resource_target for ammo (refill_after None). Columns: now, predicted, authoritative, default, sent amount (resource_confirmed is called when it is not None).

```
[0.0, 0, 0, 12, 12]
[0.05, 0, 0, 12, None]
[0.1, 12, 12, 12, None]
[0.15, 12, 12, 12, None]
[0.2, 11, 12, 12, 11]
[0.25, 11, 11, 12, None]
[0.3, 11, 11, 12, None]
[0.35, 10, 11, 12, 10]
```

resource_target for an ability charge (refill_after 4.0):

```
[0.0, 0, 0, 2, 2]
[0.05, 2, 2, 2, None]
[0.1, 2, 2, 2, None]
[1.0, 1, 2, 2, 1]
[1.05, 1, 1, 2, None]
[1.1, 1, 1, 2, None]
[4.99, 1, 1, 2, None]
[5.0, 1, 1, 2, 2]
[5.05, 2, 2, 2, None]
```


### Observed bring-up timeline (older build log, for ordering only)

`runtime/responder_20928_phase4_match_state.out`, source 49272, seconds since responder start. This build used bot GUID stride 6 and no dummy, so do not use it for GUIDs.

```
95.0 ClientRestart | 132.6 pistol InactiveState link | 136.2-145.6 corrections 1..5
141.7 ClientSetHUD (setup) | 142.1 money | 142.2 ultimate points 8
145.7 first mirror: BasePistol EquipState kick + MagazineAmmo 12 / ReserveAmmo 36 seeds
145.8 pawn Owner | 146.0-151.8 starting equippables 0..6 (Smoke/Dash/Q opens carry AbilityTuning)
151.5 shop settings | 151.6 Range HUD | 151.8 HUD refresh 1
152.6 streaming + first bot | 152.9-154.0 bots 2..5 (about 0.33 s apart)
153.4 melee InactiveState | 154.6 ClientUpdateRangeSettings | 154.6 settings 176 + 177 -> infinite ammo true
154.7-155.5 abilities ready: Smoke (11 states), Dash (15, +StateMachine_Priming), Q (12, +BoostStateMachine), X (35)
```

## Verifier corrections

An adversarial pass re-read `range_udp_handshake.py` 715-1372 and 1486-2106 and every builder module, then regenerated all vectors offline from a fresh script (`scratchpad/verify05/gen.py`, `frames.py`, `inbound.py`). All 95 named vectors in section 11 match bit for bit: bodies, the 5 full frames (plain and encrypted), the captured Warden decode, the synthetic 176/177 packet, the RNG draws and bot placements, the Cloudburst transform, and the mirror and resource traces. Every handle, GUID, channel, offset, timer and gate line reference in sections 2-7 was checked against the source. The changes below are clarifications and gaps, not wrong values.

1. Section 1: the dangling "see open questions" reference now points to the new quirk 14 (the `/dev/range/enter` respawn path omits the base-pistol cosmetic trial; `backend.py:57-66, 86`).
2. Dead paths: added the never-used `pistol_mirror_*` / `melee_mirror_*` dicts (881-891) and the `VALORANT_RANGE_DROP_FLAGS` launcher hook (launch_range.py:210-213).
3. 5.2: `range_settings_state` is write-only. 5.3: added the global skin-inventory cache `range_gun_loader._inventory_cache`.
4. 6.1 and 6.9: cross-referenced `06_client_decode.md` for the input-stream wire format and for the `iter_bunches` stop conditions (`position + 16 < bit_count`; stop at a control-close bunch or a header error). Object refs: added the `"<guid:1>"` case and the FString limits.
5. 6.9: object-array element handles start at 1. FGuid leaves are `4i+j+1`. Cosmetic pairs are allocated even for unskinned opens. Added how `guid_text_to_words` splits the UUID text.
6. 6.10: a `176` with presence bit 0 decodes as all defaults (`infinite_ammo = False`); see quirk 17.
7. 6.11: a queued `spawn_bot` does not update `bot_spawned_at` and counts toward the bot limit.
8. 6.13: `ballistic_fallback` details: float-accumulated `elapsed += 0.005`, and the case where the projectile never reaches the floor (it returns the last point).
9. 6.14: the retire test is `died_at is not None`.
10. 7: `read_world_state.py` is `range_authority_probe/read_world_state.py`, not in `local_backend`. The slot class name is the FName at `[item+0x10]+0x18`, not at `item+0x18`.
11. 8: `AllocConsole` fails when a console already exists; reopen stdout/stderr on `CONOUT$`. Python runs with `CREATE_NO_WINDOW` and a redirected stdout.
12. 9: feature gates must be evaluated at packet receipt (pre-gate purchases and settings are dropped and never deduped). Use integer tick arithmetic for thresholds that land exactly on a tick boundary.
13. 10: added quirks 14-18: respawn path without the cosmetic trial, unparseable queue files repeating, the firing seed sampled only while current, the presence-0 `176`, and the multi-history-word packets that Python never decodes.
14. 11: renamed the ambiguous `*.seed.export` resource vectors to their amounts (Smoke 2, Q 1, E 1) and `frame.ch8.correction6` to `...guid94`, and listed the frame bodies. The hex is unchanged.
15. The timeline log `runtime/responder_20928_phase4_match_state.out` is still being appended by a running responder that loaded older code; bot stride 6 and no dummy were confirmed from its head (first spawns 300100, 300106, 300112, ...).
