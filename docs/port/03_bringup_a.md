# 03 - Connection bring-up, part A: handshake replies through `auth_correct_rep_probe`

Source of truth: `local_backend/range_udp_handshake.py` (abbreviated `ruh`), lines 2107-3352, plus the builders it calls in `range_server_bunch.py` (`rsb`), `range_weapon_cosmetics.py` (`rwc`), `range_move_correction.py` (`rmc`). Every number below is copied from code; the file:line is given next to it. Test vectors (appendix V) were produced by importing those builders offline (`PYTHONDONTWRITEBYTECODE=1`, no socket, no live process) and re-running the inline `run_probe` bodies verbatim. The inline bodies were cross-checked against the library helpers that produce identical bytes (section 9.5), and every frame vector was re-serialized independently and decrypted back to its plaintext. `test_range_udp_handshake` and `test_range_server_bunch` (85 tests) pass on the current tree.

Part B (`ruh:3353` onward: `client_restart_probe` and later stages, the move-ack block, per-packet authority work) is a separate document. Part B is mentioned here only where it shares state or sequence space with part A.

## 1. Production configuration for this range

### 1.1 How the flags arrive

`launch.ps1:166-192` -> `launch_range.py:29-53, 214-287` -> `range_udp_handshake.py main()` -> `run_probe(...)` (`ruh:4915`).

| Item | Value in production | Where |
|---|---|---|
| `session_ack_probe` | on | `launch_range.py:30` |
| `player_pawn_probe_once`, `pawn_spawn_location_probe` | on | `launch_range.py:30` |
| `game_state_bool_probe_value` | 1 | `launch_range.py:31` |
| `controller_player_links_probe`, `pawn_player_links_probe`, `pawn_autonomous_role_probe` | on | `launch_range.py:31-32` |
| `inventory_open_probe`, `item_slot_create_probe`, `item_slot_link_probe`, `item_slot_scale_probe` | on | `launch_range.py:35-36` |
| `base_pistol_open/link/owner/role_probe`, `equippable_sm_open_probe`, `inactive_state_link_probe`, `auth_correct_rep_probe` | on | `launch_range.py:36-39` |
| `movement_packet_capture` | on (logging only) | `launch_range.py:42` |
| `pawn_authority_role` | off (role value 2) | `launch.ps1:22-24` throws if requested |
| `desired_class_guid` | `ADD6443A-41BDE414-F6ADE58D-267F4E95` (CLI default) | `ruh:4753`, parsed at `ruh:720-723` |
| `welcome_map_url` | `/Game/Maps/Poveglia/Range` (CLI default) | `ruh:4756` |
| `item_slot_scale_interval_seconds` | 1.0 (module constant) | `ruh:29` |
| Base-pistol initial cosmetic trial | on by default | `launch.ps1:189-192` adds `--base-pistol-initial-cosmetic-trial` unless `-skip_saved_pistol_cosmetic` or the late skin trial; `launch_range.py:286-287` forwards it; `ruh:4908-4909` turns it into env `RANGE_BASE_PISTOL_INITIAL_COSMETIC_TRIAL=1`; `ruh:737-739` reads the env into `base_pistol_initial_cosmetic_trial`. `ruh:745-758` then fixes catalog `local_backend/skin_catalog.json`, state `local_backend/offline_skin_state.json`, player `backend.offline_player_id` = `7c9f41a4-65e2-4d81-a8d5-f879c3e5ab61` (`backend.py:36`). |
| `VALORANT_RANGE_HEARTBEAT_HZ` | unset -> 0 -> no unsolicited heartbeats | `ruh:1366-1369, 1416-1419`. No production launcher sets it. |
| `VALORANT_RANGE_QUIET_PACKETS` | unset -> every packet event is printed | `ruh:1365, 1373-1377` |
| Keys | `client_send_key` and `receive_key_override` are read once, from the live client handler at image base + RVA `0xA5F3BB0` | `ruh:1465-1482`. They are global, not per source. The read is retried on every datagram until it succeeds; until then each datagram is logged with `handler_state: "not_ready"` and dropped before even the stateless-handshake parse (`ruh:1476-1481`). Transport doc. |
| `VALORANT_RANGE_DROP_FLAGS` | unset -> no flag removed | `launch_range.py:210-213`. An A/B hook that deletes responder flags; no production launcher sets it. |

`session_ack_probe` implies the challenge/welcome/join-ack branches: each of those conditions contains `or session_ack_probe` (`ruh:2111, 2115, 2143, 2158`). `main()` rejects `session_ack_probe` combined with any `*_probe_once` handshake flag (`ruh:4776-4777`) and rejects `player_pawn_probe_once` combined with the other actor probes (`ruh:4790-4791`).

### 1.2 Dead paths inside 2107-3352 (skip in the port)

- `ruh:2131-2133` `ack_probe_once` -> `build_server_ack_probe`: superseded by the challenge path, and forbidden with `session_ack_probe`.
- (Not dead, but no wire effect.) `ruh:2116-2126` `inspect_packet_notify` pre-state read **does run in production**, once, inside the challenge arm (its condition includes `session_ack_probe`), before the challenge is built. It is a live ReadProcessMemory diagnostic. `OSError`/`RuntimeError`/`ValueError` are caught and logged as `pre_state_error=<exception class name>`. On success it logs `pre_control_sequence` and `pre_in_sequence`. The port may omit it; a shadow-mode log diff must ignore those three keys.
- `ruh:3008-3014` the non-cosmetic plain BasePistol open (`base_pistol_initial_cosmetic_trial` false): dead, because launch.ps1 passes the trial by default. The cosmetic path's own fallback (`ruh:3002-3007`) builds the byte-identical plain body (section 8.1).
- `--move-ack-uniform-transform` (`ruh:4733, 4912-4914` -> `rsb:1079-1083`): it would switch **every** `encode_short_actor_rep_payload` call (all part-A rep payloads) to the per-byte path. No launcher passes it, so `_UNIFORM_BYTE_TRANSFORM[0]` stays `False`, and all vectors here use the 64/32/8/partial staged transform.
- `ruh:2079-2085` `VALORANT_RANGE_CAPTURE_LOGIN` login capture: diagnostic env, unset.
- `ruh:2188-2194` null-GUID / inline-static ch3 bodies (`actor_open_probe_once`, `static_actor_probe_once`): off. Production takes the exported-static body.
- `ruh:2224-2229` `empty_game_state_rep_layout_probe` / `empty_game_state_content_block_probe` appendices to the GameState open: off.
- `ruh:2249-2285` empty GameState update / rep-layout sweep / encoded-offset update: off.
- `ruh:2668` `pawn_authority_role` (role 3): off. Production sends role 2.
- `ruh:2984-2985, 3028-3032` `acr_layout_trial` ACR open on ch9: off.
- `ruh:3260-3289` `base_pistol_skin_trial` (env `RANGE_BASE_PISTOL_SKIN_TRIAL`): off. It is mutually exclusive with the initial cosmetic trial (`ruh:746-747`).
- `ruh:3292-3297` the `else` arms of the `auth_correct_rep_probe` prerequisite: unused, because `inactive_state_link_probe` is on.
- `RANGE_EQUIP_STATE_TRIAL`, `RANGE_ACR_ACTOR_INSPECTION_PROBE`: Part B branches, off.

## 2. State

### 2.1 Per connection (key = UDP source `(ip, port)`)

| Name | Type | Written by | Read by |
|---|---|---|---|
| `pending` | dict src->(timestamp, cookie) | stateless initial handshake `ruh:4547`; never removed | every branch here requires `source in pending` |
| `hello_packets` | dict src->decoded tuple | `ruh:2110`: any authenticated packet with `payload_bits == 232` (overwritten by each later one) | challenge, welcome. The welcome copies its header from the **latest** stored 232-bit packet (`ruh:2147`), not necessarily the one the challenge used. If that later packet is not a type-0 reliable ch0 control bunch, `build_server_control_probe` raises ValueError and the responder dies (section 8.1). |
| `next_server_sequences` | dict src->14-bit | challenge `ruh:2140`, then +1 after every send | every send |
| `initial_actor_channel_sequences` | dict src->12-bit base B | `ruh:2141`: (challenge server seq + 1) & 0xFFF | every reliable actor bunch |
| `recent_client_sequences` | dict src->deque(maxlen=64) | `ruh:1490-1492`, every authenticated packet (including 232-bit ones and packets that get no ack) | ack history |
| `ack_history_mask` | local, per packet; reset to 0 for every datagram (`ruh:1488`) | `update_received_packet_history` (`ruh:560-576`), only when authenticated | session ack + stage bunches + move ack |
| `latest_authenticated_packets` | dict src->(client seq, stateless header, mask, monotonic) | `ruh:1493-1495`, every authenticated packet | only the heartbeat (`ruh:1421`), which is off. The port does not need it. |
| `last_session_ack_times` | dict src->monotonic | join ack `ruh:2171`, session ack `ruh:2184` | 0.1 s gate |
| `ack_sent_sources`, `welcome_sent_sources`, `join_ack_sent_sources` | sets | `ruh:2142, 2157, 2170` | handshake ordering |
| stage sets and times | one `*_sent_sources` set + `*_sent_times` dict per stage; see the section 6 table and section 7 | each stage | next stage |
| `controller_link_stage`, `pawn_link_stage` | dict src->1/2 | A08/A09, A14/A15 | A10, A14, A16, Part B |
| `item_slot_scale_index`, `item_slot_scale_last_time` | dict | A19 sets index 1 (`ruh:2871-2872`); A20 increments | A20, A21 |
| `auth_correct_rep_index` | dict src->int | A27 (`ruh:3352`); **also** Part B input-correction (`ruh:1744`) and purchases (`ruh:1261-1263`) | A27 gate (<5), Part B gates (>=5) |
| `movement_capture_counts/_last_times`, `input_capture_sequences` | dicts | logging only (`ruh:2086-2106`) | - |

### 2.2 Global (not per source)

- `client_send_key`, `receive_key_override` (`ruh:767-768, 1474`).
- `channel_first_dynamic_offset` (`ruh:786, 911-914`) = `{4: 8, 5: 6, 6: 11, 8: 51, 9: 5, 12: 2, 13: 5, 14..18: 2}` in production (`ch4 = range_shop_sequence_offset(True)+1 = 7+1`, `ruh:243-257`; `ch5 = 6`, because goto-state and phase-begin are off). Part A does not call `take_channel_sequence`. Every part-A reliable sequence is `(B + fixed offset) & 0xFFF`. The dynamic counters start after part A's fixed offsets (section 9.1).

### 2.3 What the server does NOT do (copy this behaviour)

- It never retransmits a bunch. The client's acks of server packets are ignored: the client's acked-server field is read only once, to seed the challenge sequence (`ruh:656`).
- Every UDP frame carries exactly one bunch (`rsb:1524-1611`). A "pair" stage (A11, A20) sends two frames with consecutive server sequences.
- It sends nothing without a triggering client packet (heartbeat is off).
- It never resets per-source state. A reconnect from the same source port would find every one-shot set already populated and would not get the chain again (open question 6).

## 3. Where this branch runs inside one received datagram

`ruh:1440-4579`. For each datagram from 127.0.0.1 (any other source address is dropped silently, `ruh:1460-1461`):

0. If the keys have not been read yet, try to read them. On failure, log `handler_state: "not_ready"` and drop the datagram (`ruh:1465-1481`).
1. `parse_handshake` succeeds -> stateless path (`ruh:4538-4572`, other doc). Otherwise:
2. `inspect_client_ares_packet` (`ruh:1487`). If authenticated: update the ack history (`ruh:1489-1495`).
3. Part B per-packet work, gated on `client_restart_sent_sources` (`ruh:1499-2078`). It can send frames (consuming server sequences) **before** the session ack of the same datagram.
4. Logging-only captures (`ruh:2079-2106`; movement capture is gated by A16).
5. The `if/elif` dispatch at `ruh:2107-2172`. **Exactly one** arm runs:
   1. not authenticated -> record `parse_error`, send nothing (`ruh:2107-2108`)
   2. `payload_bits == 232 and source in pending` -> store as hello, send nothing (`ruh:2109-2110`). **This catches every 232-bit packet for the life of the connection**, including after join: such a packet gets no session ack, no stage and no move ack.
   3. challenge (`ruh:2111`)
   4. welcome (`ruh:2143`)
   5. join ack (`ruh:2158`)
   6. session-ack tick (`ruh:2172`): session ack, then **at most one** stage arm of the elif chain `ruh:2185-4407` (part A = `2185-3352`, part B = `3353-4407`), then the independent move-ack `if` (`ruh:4412`, Part B).
6. Append the event and print it as one JSON line (`ruh:4573-4578`). Exception: the challenge arm's `continue` (`ruh:2135`) is the only loop-level `continue` in the handler, and it skips the event entirely.

Frame order within one tick is therefore: [Part B per-packet frames] -> session ack (S) -> stage frame(s) (S+1[, S+2]) -> move ack.

## 4. Handshake replies

All three use the decoded client packet `decoded, bit_count, _, stateless_header = decode_client_ares_packet(payload, client_send_key)` (`ruh:445-473`; stateless_header = raw bits 64..69 of the datagram) and `receive_key_override`.

### 4.1 Challenge (NMT_Challenge, type 3), `ruh:2111-2142`

- Trigger: authenticated, `payload_bits == 77`, `source not in ack_sent_sources`, `source in pending`. If `source not in hello_packets` the code runs `continue` (`ruh:2134-2135`): nothing is sent and no event is printed.
- Build: `build_server_challenge_probe(hello_decoded, 232, decoded, 77, stateless_header, receive_key)` -> `build_server_control_probe(..., message_type=3, string_fields=("",))` (`ruh:697-700, 640-694`):
  - `client_sequence = (u32(decoded[0:4]) >> 18) & 0x3FFF`.
  - **server packet sequence S0 = (((u32 >> 4) & 0x3FFF) + 1) & 0x3FFF** (the client's acked-server field + 1).
  - Packet header: `(S0 << 18) | (client_sequence << 4)` (32 bits), history mask **3** (32 bits), bit 64 = 1, bits 65-75 = 0.
  - Bunch header: the hello's own header bits `[76, bunch_body_start-13)` copied, with bit index 1 of that region (bOpen) cleared and its 12-bit chseq replaced by `(S0 + 1) & 0xFFF` (`ruh:664-669`). The channel (0) and the hardcoded name (the hello's) are carried over.
  - 13-bit size = 40, body = `03` + int32 0 (empty FString), terminator bit. 176 bits for the synthetic hello.
  - The hello must be a reliable control bunch, ch0, type 0, ending at bit 231 (`ruh:645-653`). Otherwise a ValueError is raised inside the `except ValueError` handler. Nothing catches it, so it ends `run_probe` and the responder process (section 8.1, failure behaviour).
- Before the build, the production-only diagnostic `inspect_packet_notify` runs (`ruh:2116-2126`, section 1.2). It has no wire effect.
- Records: `next_server_sequences = S0+1`, **`B = initial_actor_channel_sequences = (S0+1) & 0xFFF`**, `ack_sent_sources`. The ch0 reliable sequence used by the challenge, `(S0+1)&0xFFF`, therefore equals B.
- Event: `sent_control_message_type=3, sent_ack_probe_length, acked_client_sequence, server_sequence`, plus either `pre_state_error` or `pre_control_sequence` + `pre_in_sequence` from the diagnostic.

### 4.2 Welcome (NMT_Welcome, type 1), `ruh:2143-2157`

- Trigger: `source in ack_sent_sources`, `not in welcome_sent_sources`, `in pending`, `control_message_type == 5` (first bunch body byte, from `inspect_control_bunch`).
- Build: `build_server_welcome_probe(hello_decoded, 232, decoded, bits, sh, key, next_server_sequences[src], "/Game/Maps/Poveglia/Range")` (`ruh:703-712`). `hello_decoded` is the latest stored 232-bit packet (section 2.1). There is no `payload_bits` check here, but a 232-bit login would already have been caught by the hello arm. Same copied header, with server sequence = `next_server_sequences` (S0+1), control chseq = (S+1)&0xFFF = (S0+2)&0xFFF = B+1, history mask **7**, acked client sequence = the login packet's sequence. Body = `01` + FString(map, int32 26 + 25 chars + NUL) + int32 0 + int32 0 (312 bits).
- Records: next+1, `welcome_sent_sources`. Event: `sent_control_message_type=1, sent_welcome_probe_length, acked_client_sequence, server_sequence`.

### 4.3 Join ACK, `ruh:2158-2171`

- Trigger: `in welcome_sent_sources`, `not in join_ack_sent_sources`, `in pending`, `control_message_type == 9`. `build_server_join_ack` additionally requires `client_bit_count >= 128`, ch0 and reliable (`ruh:601-606`). A mismatch raises ValueError, which ends the responder process (section 8.1).
- Wire: an ACK-only 77-bit packet, identical to a session ack with the **default mask 3** (`ruh:607-608, 611-613`), not the computed history.
- Records: next+1, `join_ack_sent_sources`, **`last_session_ack_times[src] = now`**. Event: `sent_join_ack_length, acked_client_sequence, server_sequence`.

## 5. Session-ack tick (cadence), `ruh:2172-2184`

- Trigger: `session_ack_probe and source in join_ack_sent_sources and source in pending and monotonic() - last_session_ack_times[src] >= 0.1`, and none of the earlier arms matched.
- Cadence: at most one per 0.1 s per source. The tick fires on the **first** authenticated, non-232-bit client packet arriving >= 0.1 s after the previous session ack or join ack. Other packets get no session ack.
- Build: `build_server_session_ack(decoded, bits, sh, key, next_server_sequences[src], ack_history_mask)` (`ruh:611-637`). The 77-bit plaintext is `u32((S<<18)|(C<<4))`, `u32(ack_history_mask)`, then bytes `01 10` (bit 64 = packet-info present, bits 65-75 = 0, bit 76 = terminator). `C` = this packet's sequence. The mask is computed over the last 64 authenticated client sequences: bit d-1 is set for each previous sequence at distance d in 1..32 (`ruh:560-576`).
- Records: next+1, `last_session_ack_times = now`. Event: `sent_session_ack_length, acked_client_sequence, server_sequence`.
- Then the stage chain runs with `server_sequence = next_server_sequences[src]` (= S+1), `acked_client_sequence = C`, the same `ack_history_mask`, the same `stateless_header`, `receive_key_override`.

### 5.1 Stage-chain semantics

- It is one `if/elif` list (`ruh:2185-4407`). Per tick, the **first** arm whose condition is true fires, and only that arm.
- Each part-A arm requires the previous part-A arm's record plus a monotonic delay measured from that record's timestamp. The timestamp is taken after `sendto`, not at the tick. So part A fires strictly in source order A01 -> A27.
- Part A arms sit before every part-B arm, so a part-A stage is never delayed by part B. Part-B stages fire only on ticks where no part-A arm is eligible. Example: `client_restart_probe` (`ruh:3353-3357`) becomes eligible 3.0 s after A16 and fires in the gap between A17 (A16+1.5) and A18 (A17+2.0).
- The move-ack block (`ruh:4412`) is a separate `if`, not part of the chain, and runs on the same tick after the stage.

## 6. Ordered stage table (production, part A)

B = `initial_actor_channel_sequences[src]`. All bunches are reliable, `channel_name_index=102`, `is_close=False`. "Nominal t" is the cumulative minimum delay after A01 (A01 fires on the first session-ack tick, >= 0.1 s after the join ack). Real times are later by tick quantization.

| # | Stage (flag) | ruh lines | Prerequisite (all also need the session-ack tick) | Delay | Nominal t (s) | Ch | Open | Exports | chseq | Body |
|---|---|---|---|---|---|---|---|---|---|---|
| A01 | WorldSettings open (`player_pawn_probe_once`) | 2185-2217 | join ack | - | 0.0 | 3 | 1 | 1 | B | `build_exported_static_actor_reference(("AresWorldSettings","PersistentLevel","Range","/Game/Maps/Poveglia/Range"))` |
| A02 | GameState open | 2218-2248 | A01 | 2.0 | 2.0 | 4 | 1 | 1 | B | `build_dynamic_range_game_state_reference()` |
| A03 | Controller open | 2286-2307 | A02 | 2.0 | 4.0 | 5 | 1 | 1 | B | `build_dynamic_range_controller_reference()` |
| A04 | PlayerState open | 2308-2329 | A03 | 2.0 | 6.0 | 6 | 1 | 1 | B | `build_dynamic_range_player_state_reference()` |
| A05 | PlayerInfo open | 2330-2351 | A04 | 2.0 | 8.0 | 7 | 1 | 1 | B | `build_dynamic_player_info_reference()` |
| A06 | Pawn open (+spawn loc) | 2352-2379 | A05 | 2.0 | 10.0 | 8 | 1 | 1 | B | `build_dynamic_wushu_pawn_reference((3581.634, -12019.322, 1294.153))` |
| A07 | GameState bool h18=1 (`game_state_bool_probe_value`) | 2380-2408 | A06 | 3.0 | 13.0 | 4 | 0 | 0 | B+1 | rep block, `bool(18,True)`, seed 2 |
| A08 | Controller.PlayerState (`controller_player_links_probe`) | 2409-2435 | A07 | 3.0 | 16.0 | 5 | 0 | 0 | B+1 | rep block, `obj(15,6)`, seed 4 |
| A09 | Controller.Pawn | 2436-2461 | A08 (stage 1) | 3.0 | 19.0 | 5 | 0 | 0 | B+2 | rep block, `obj(16,10)`, seed 4 |
| A10 | PlayerState.PlayerInfo (`pawn_player_links_probe`) | 2462-2491 | A09 (stage 2) | 3.0 | 22.0 | 6 | 0 | 0 | B+1 | rep block, `obj(30,8)`, seed 6 |
| A11 | PlayerState.Spawned/PossessedCharacter (2 frames) | 2492-2528 | A10 | 0.5 | 22.5 | 6 | 0 | 0 | B+2, B+3 | rep blocks `obj(194,10)`, `obj(195,10)`, seed 6 |
| A12 | PlayerState.DesiredClass | 2529-2569 | A11 | 0.5 | 23.0 | 6 | 0 | 0 | B+4 | rep block, `desired_class(A,B,C,D, base_handle=201)`, seed 6 |
| A13 | PlayerInfo.AresController | 2570-2606 | A12, A05 | 0.5 after A12 | 23.5 | 7 | 0 | 0 | B+1 | rep block, `obj(15,4)`, seed 8 |
| A14 | Pawn.PlayerState | 2607-2634 | A09 (stage 2), A12 | 1.0 after A12 | 24.0 | 8 | 0 | 0 | B+1 | rep block, `obj(15,6)`, seed 10 |
| A15 | Pawn.Controller | 2635-2660 | A14 (stage 1) | 3.0 | 27.0 | 8 | 0 | 0 | B+2 | rep block, `obj(16,4)`, seed 10 |
| A16 | Pawn.RemoteRole=2 (`pawn_autonomous_role_probe`) | 2661-2688 | A15 (stage 2) | 1.0 | 28.0 | 8 | 0 | 0 | B+3 | rep block, `pawn_role(width 3, value 2)`, seed 10 |
| A17 | Inventory subobject open GUID 60 | 2689-2747 | A16 | 1.5 | 29.5 | 8 | 0 | 1 | B+4 | inline (9.4) |
| A18 | ItemSlot 0 create GUID 62 | 2748-2825 | A17 | 2.0 | 31.5 | 8 | 0 | 1 | B+5 | inline |
| A19 | Inventory.ItemSlots[0]=62 | 2826-2872 | A18 | 1.5 | 33.0 | 8 | 0 | 0 | B+6 | inline |
| A20 | ItemSlot i create + ItemSlots[i] link, i=1..15 (2 frames per tick) | 2873-2968 | A19, then previous i | 1.0 (`item_slot_scale_interval_seconds`) | 34.0 .. 48.0 | 8 | 0 | 1 / 0 | B+7+2(i-1), B+8+2(i-1) | inline |
| A21 | BasePistol actor open GUID 94 | 2969-3047 | A19 and index >= 16 | 2.0 after last A20 | 50.0 | 9 | 1 | 1 | B | cosmetic open or plain (8.1) |
| A22 | ItemSlot_1.Contents=94 | 3048-3093 | A21 | 2.0 | 52.0 | 8 | 0 | 0 | B+37 | inline (= `build_item_slot_contents_block(1,94)`) |
| A23 | BasePistol h12=10 | 3094-3130 | A22 | 1.0 | 53.0 | 9 | 0 | 0 | B+1 | inline (= `build_channel_actor_owner_block(94,10)`) |
| A24 | BasePistol RemoteRole=2 | 3131-3173 | A23 | 1.0 | 54.0 | 9 | 0 | 0 | B+2 | inline, `build_base_pistol_autonomous_role_payload()`, seed 94 |
| A25 | EquippableStateMachine open GUID 96 | 3174-3226 | A24 | 1.5 | 55.5 | 9 | 0 | 1 | B+3 | inline |
| A26 | InactiveState export + AuthCurrentState=98 | 3227-3259 | A25 | 1.0 | 56.5 | 9 | 0 | 1 | B+4 | `build_equippable_state_link_body("InactiveState", 98)` |
| A27 | Inventory.AuthServerCorrectRepVariables, idx 1..5 | 3290-3352 | A26 | 2.0 after max(A26, A25, A24, A23, last A27) | 58.5, 60.5, 62.5, 64.5, 66.5 | 8 | 0 | 0 | B+37+idx | inline (= `build_inventory_correction_body(idx,94)`) |

## 7. Per-stage detail

Notation: `obj(h,g)` = `build_object_property_payload(h,g)` (`rsb:1277-1297`): bit 0, packed h, packed g, packed 0. `bool(h,v)` = `rsb:1229-1238`: bit 0, packed h, 1 bit v, packed 0. A "rep block" is `append_actor_rep_layout_payload(b"", 0, enc, nbits)` (`rsb:244-253`): bit 1 (bHasRepLayout), bit 1 (bIsActor = channel actor), packed nbits, then nbits of `enc = encode_short_actor_rep_payload(plain, nbits, 0, seed)` (`rsb:1131-1216`). Times are `time.monotonic()` taken right after the send. "Unlocks" lists only readers of the record.

**A01 WorldSettings open.** Gate `source not in actor_probe_sent_sources` (`ruh:2185`). Body `rsb:118-138`: bit 0, u32 count 1, then for each chain element in order AresWorldSettings, PersistentLevel, Range, package: packed GUID 3, 5, 7, 9 (`actor_net_guid + 2*i`, `rsb:131`) + 8 bits flags 5. Then packed 0, then reversed names (package first), each as FString (u32 len+1, ASCII, NUL) + u32 checksum 0, then packed actor GUID 3. 897 bits. Records `actor_probe_sent_sources`, `actor_probe_sent_times` (`ruh:2216-2217`). Event `sent_actor_open_probe_length, actor_probe_channel_name_index=102, actor_probe_channel_sequence, actor_probe_body="exported_static_world_settings_path"`.

**A02 GameState open.** Gate in A01 set, not in `game_state_probe_sent_sources`, >= 2.0 s after A01 (`ruh:2218-2220`). Body `build_dynamic_actor_reference(("Default__ShootingRangeGameState_C", "/Game/GameModes/ShootingRange/ShootingRangeGameState"), actor 2, archetype 11, package 13, level 5)` (`rsb:198-203`). Generic dynamic layout (`rsb:141-195`): bit 0, u32 1, packed archetype GUID + flags 5 (8), packed package GUID + flags 5, packed 0, FString package + u32 0, FString archetype + u32 0, packed actor, packed archetype, packed level, then 4 zero bits when there is no spawn location. 925 bits. Records `game_state_probe_sent_sources/_times`. Event `sent_game_state_probe_length, empty_game_state_rep_layout_probe=false, empty_game_state_content_block_probe=false, game_state_probe_channel_sequence`.

**A03 Controller open.** >= 2.0 s after A02 (`ruh:2286-2288`). `("Default__ShootingRangeController_C", "/Game/Characters/_Core/ShootingRangeController")`, GUIDs 4/15/17, then **8 extra zero bits** (local-player index, `rsb:1476`). 893 bits. Records `player_controller_probe_sent_sources/_times`. Event `sent_player_controller_probe_length`.

**A04 PlayerState open.** >= 2.0 s after A03. `("Default__ShootingRangePlayerState_C", "/Game/GameModes/ShootingRange/ShootingRangePlayerState")`, GUIDs 6/19/21 (`rsb:1480-1485`). 957 bits. Records `player_state_probe_sent_sources/_times`. Event `sent_player_state_probe_length`.

**A05 PlayerInfo open.** >= 2.0 s after A04. `("Default__OwnerExclusivePlayerInfo", "/Script/ShooterGame")`, GUIDs 8/23/25 (`rsb:1488-1492`). 661 bits. Records `player_info_probe_sent_sources/_times`. Event `sent_player_info_probe_length`.

**A06 Pawn open.** >= 2.0 s after A05. `("Default__Wushu_PC_C", "/Game/Characters/Wushu/Wushu_PC")`, GUIDs 10/27/29, spawn location from the **literal** `(3581.634, -12019.322, 1294.153)` at `ruh:2357-2360` (gated by `pawn_spawn_location_probe`, not taken from the CLI). Spawn tail (`rsb:183-195`): bit 1 (position), bit 1 (quantized format), the quantized vector (V.5: 7-bit header `18|0x40`, 3 x 18-bit two's complement of round-half-away(x*10) = 35816, -120193, 12942), then 3 zero bits (yaw rounds to 0 -> no rotation; default scale and velocity). 707 bits. Records `pawn_probe_sent_sources/_times`. Event `sent_wushu_pawn_probe_length, pawn_spawn_location_probe=[3581.634,-12019.322,1294.153]`.

**A07 GameState bool.** `game_state_bool_probe_value is not None`, in A06 set, not in `game_state_bool_sent_sources`, >= 3.0 s after A06. Payload `bool(18, True)` = 18 bits, seed 2 (GameState GUID). The code does not name handle 18. Records `game_state_bool_sent_sources/_times`. Event `sent_game_state_bool_probe_length, game_state_bool_probe_value=1, game_state_bool_probe_payload=<enc hex>`.

**A08 Controller -> PlayerState.** In A07 set, `source not in controller_link_stage`, >= 3.0 s after A07. `obj(15,6)` (25 bits), seed 4. Sets `controller_link_stage=1` and `controller_link_sent_times`. Event `sent_controller_player_state_link_length, controller_player_state_link_payload`.

**A09 Controller -> Pawn.** `controller_link_stage == 1`, >= 3.0 s after A08. `obj(16,10)`, seed 4, chseq B+2. Sets stage 2. Event `sent_controller_pawn_link_length, controller_pawn_link_payload`.

**A10 PlayerState -> PlayerInfo.** `controller_link_stage == 2`, not in `player_state_info_sent_sources`, >= 3.0 s after A09. `obj(30,8)`, seed 6 (comment `ruh:2468-2470`: PlayerInfo RepIndex 23 -> cmd 29 -> wire handle 30). Records `player_state_info_sent_sources/_times`. Event `sent_player_state_info_link_length, player_state_info_link_payload`.

**A11 PlayerState -> SpawnedCharacter / PossessedCharacter.** In A10 set, not in `player_state_character_sent_sources`, >= 0.5 s after A10. Two frames sent back to back in one tick (`ruh:2507-2526`): `obj(194,10)` at chseq B+2 and `obj(195,10)` at chseq B+3, both seed 6, both 33-bit payloads (handles >= 128 take 2 packed bytes, so the 32-bit word transform stage applies). Server sequences S+1, S+2. The time is recorded once, after both frames. Events `sent_player_state_spawned_character_link_length`, `player_state_spawned_character_link_payload`, and the same pair for `possessed_character`.

**A12 PlayerState -> DesiredClass.** In A11 set, not in `player_state_desired_class_sent_sources`, >= 0.5 s after A11. `build_desired_class_payload(0xADD6443A, 0x41BDE414, 0xF6ADE58D, 0x267F4E95, base_handle=201)` (`rsb:1241-1274`): bit 0, then 4 x (packed 201+k, raw u32 word k), packed 0 = 201 bits, seed 6, chseq B+4. Records `player_state_desired_class_sent_sources/_times`. Event `sent_player_state_desired_class_link_length, player_state_desired_class_link_payload, player_state_desired_class_guid="ADD6443A-41BDE414-F6ADE58D-267F4E95"`.

**A13 PlayerInfo -> Controller.** In A12 set **and** in A05 set, not in `player_info_controller_sent_sources`, >= 0.5 s after A12. `obj(15,4)`, seed 8, ch7 chseq B+1. Records `player_info_controller_sent_sources/_times` (no later reader in part A). Event `sent_player_info_controller_link_length, player_info_controller_link_payload`.

**A14 Pawn -> PlayerState.** `controller_link_stage == 2`, in A12 set, `source not in pawn_link_stage`, >= 1.0 s after **A12** (not after A13). `obj(15,6)`, seed 10, chseq B+1. Sets `pawn_link_stage=1` and `pawn_link_sent_times`. Event `sent_pawn_player_state_link_length, pawn_player_state_link_payload`.

**A15 Pawn -> Controller.** `pawn_link_stage == 1`, >= 3.0 s after A14. `obj(16,4)`, seed 10, chseq B+2. Sets stage 2. Event `sent_pawn_controller_link_length, pawn_controller_link_payload`.

**A16 Pawn RemoteRole.** `pawn_link_stage == 2`, not in `pawn_role_sent_sources`, >= 1.0 s after A15. `build_pawn_autonomous_role_payload(role_value=2)` (`rsb:1419-1447`): bit 0, packed 4, **3 bits** value 2, packed 0 = 20 bits, seed 10, chseq B+3. Records `pawn_role_sent_sources/_times`. Unlocks movement capture logging (`ruh:2086, 2095`) and part B `client_restart_probe` (>= 3.0 s after A16). Event `sent_pawn_autonomous_role_length, pawn_autonomous_role_payload`.

**A17 Inventory subobject open.** In A16 set, not in `inventory_open_sent_sources`, >= 1.5 s after A16. The body is inline (`ruh:2706-2729`, section 9.4 "stable subobject export + content block"): export of GUID 60, outer 10, name `InventoryComponent`, then a content block on 60 (stably named) with the empty rep `[bit 0, packed 0]` (9 bits), seed 10. 309 bits, chseq B+4, has_exports. Records `inventory_open_sent_sources/_times`. Event `sent_inventory_open_length, inventory_open_subobj_guid=60`.

**A18 ItemSlot 0 create.** In A17 set, not in `item_slot_create_sent_sources`, >= 2.0 s after A17. Inline (`ruh:2767-2806`): class export 61 (outer package 63 `/Script/ShooterGame`, root terminator), class name `ItemSlot`, then a dynamic-subobject content block: new GUID 62, class 61, outer 60, empty rep (9 bits), seed 10. 479 bits, chseq B+5, has_exports. Records `item_slot_create_sent_sources/_times`. Event `sent_item_slot_create_length, item_slot_create_slot_guid=62, item_slot_create_class_guid=61`.

**A19 Inventory.ItemSlots[0] = 62.** In A18 set, not in `item_slot_link_sent_sources`, >= 1.5 s after A18. Inline (`ruh:2840-2851`): stably-named content block on 60 with `obj(3,62)`, seed 10. 44 bits, chseq B+6. Records `item_slot_link_sent_sources/_times`; sets **`item_slot_scale_index=1`, `item_slot_scale_last_time=now`**. Event `sent_item_slot_link_length, item_slot_link_wire_handle=3, item_slot_link_slot_guid=62`.

**A20 Scale slots 1..15.** In A19 set, `item_slot_scale_index < 16`, `monotonic - item_slot_scale_last_time >= 1.0`. For `i = index`: slot GUID `62+2i` (64..92), wire handle `3+i` (4..18), seq_offset `7+2(i-1)` (`ruh:2889-2892`). Frame 1 (server seq S+1): the A18 body with GUID `62+2i`, chseq B+seq_offset, has_exports. **The class/package export 61/63 is repeated every time.** Frame 2 (server seq S+2): the A19 body with `obj(3+i, 62+2i)`, chseq B+seq_offset+1. Index += 1, last_time = now. 15 ticks in total. Event `sent_item_slot_scale_pair_length` (sum of both frame lengths), `item_slot_scale_slot_index, item_slot_scale_slot_guid, item_slot_scale_wire_handle`.

**A21 BasePistol actor open.** (`base_pistol_open_probe`) in A19 set, `item_slot_scale_index >= 16`, not in `base_pistol_open_sent_sources`, not in `acr_layout_open_sent_sources`, >= 2.0 s after the last A20. The body depends on the cosmetic trial (section 8.1). ch9, chseq B, open, exports. Records `base_pistol_open_sent_sources/_times`. Unlocks part B `team_state_open_probe` (`ruh:3411-3414`). Event `sent_base_pistol_open_length, base_pistol_actor_guid=94, base_pistol_initial_cosmetic_bits=<body bits>`, plus either `base_pistol_initial_skin_id/level_id/chroma_id`, or `base_pistol_initial_cosmetic_unshipped=true`, or `base_pistol_initial_cosmetic_unselected=true`.

**A22 ItemSlot_1.Contents = BasePistol.** In A21 set, not in `base_pistol_link_sent_sources`, >= 2.0 s after A21. Inline (`ruh:3062-3076`): dynamic-subobject content block on slot GUID **64**, class 61, outer 60, `obj(1,94)`, seed 10. 62 bits, **ch8** chseq **B+37** (`7+2*15`). Records `base_pistol_link_sent_sources/_times`. Event `sent_base_pistol_link_length`.

**A23 BasePistol handle 12 = pawn.** >= 1.0 s after A22. Inline (`ruh:3107-3113`): bit 1, bit 1 (bIsActor), packed 25, `obj(12,10)` encoded with **seed 94**. 35 bits, ch9 chseq B+1. Records `base_pistol_owner_sent_sources/_times`. Event `sent_base_pistol_owner_length`.

**A24 BasePistol RemoteRole.** >= 1.0 s after A23. `build_base_pistol_autonomous_role_payload()` (`rsb:1450-1465`: bit 0, packed 4, 3 bits value 2, packed 0 = 20 bits), seed 94, actor content block. 30 bits, ch9 chseq B+2. Records `base_pistol_role_sent_sources/_times`. Event `sent_base_pistol_role_length`.

**A25 EquippableStateMachine open.** >= 1.5 s after A24. Inline (`ruh:3188-3208`): stable-subobject export of GUID 96, outer 94, name `EquippableStateMachine`, then a stably-named content block on 96 with the empty rep (9 bits), seed 94. 341 bits, ch9 chseq B+3, has_exports. Records `equippable_sm_open_sent_sources/_times`. Event `sent_equippable_sm_open_length, equippable_sm_subobj_guid=96`.

**A26 InactiveState link.** >= 1.0 s after A25. `build_equippable_state_link_body("InactiveState", 98)` (`ruh:118-145`): stable-subobject export of GUID 98, outer 94, name `InactiveState`, then a stably-named content block on SM 96 with rep `[bit 0, packed 3, packed 98, packed 0]` (25 bits), seed 94. 285 bits, ch9 chseq B+4, has_exports. Records `inactive_state_link_sent_sources/_times`. Event `sent_inactive_state_link_length, inactive_state_guid=98`.

**A27 Inventory correction x5.** `source in inactive_state_link_sent_sources` (because `inactive_state_link_probe` is on), `auth_correct_rep_index < 5`, `monotonic - max(A26_t, A25_t, A24_t, A23_t, last_A27_t) >= 2.0`. `idx = index+1`. Rep (`ruh:3316-3322`): bit 0, packed 25, packed 94, packed 26, raw u32 idx, packed 0 = 65 bits (the 64-bit wide transform stage + 1 partial bit), seed 10. Stably-named content block on 60. 84 bits, ch8 chseq **B+37+idx** (B+38..B+42). Records `auth_correct_rep_sent_sources`, `auth_correct_rep_sent_times`, `auth_correct_rep_index=idx`. After idx 5 (index >= 5): `mirror_ready_equippables` includes the pistol (`ruh:1316-1321`), the input-correction responder (`ruh:1719`) and part B `pawn_owner_probe` (`ruh:3735-3738`) unlock. Event `sent_auth_correct_rep_length, auth_correct_rep_index`.

## 8. Data-dependent inputs

### 8.1 BasePistol initial cosmetic (A21), `ruh:2986-3007`

1. At send time (not at launch), `select_base_pistol_cosmetic_loadout(catalog, state, player)` (`ruh:96-115`) builds a fresh `SkinInventory` from the two JSON files and resolves the equipped BasePistol (`rwc:24` weapon id `29a0cfab-485b-f5d5-779a-b59f85e204a8`) through `equipped_cosmetic.resolve_equipped_cosmetic` and `resolve_equipped_cdo_paths`.
2. `None` (nothing equipped) -> plain open, event `..._unselected`.
3. If a selection exists but any of skin/level/chroma has `packaged == False` -> plain open, event `..._unshipped`. Per `equipped_cosmetic.py:200-204`, only the **level** can be unpackaged: `kind == "level"`, the catalog entry's `source.level_asset_packaged is False`, **and** the selected level's asset equals `source.level_asset`. Skin and chroma are always `packaged=True`.
4. Otherwise `build_base_pistol_initial_cosmetic_open(cosmetic, cdo_paths)` (`rwc:147-222`): bit 0, u32 export count **4**, then 4 static export entries (`rwc:133-144`: packed object GUID, flags 5, packed package GUID, flags 5, packed 0, FString package, u32 0, FString object, u32 0): archetype 95/97 (`Default__BasePistol_C`, `/Game/Equippables/Guns/Sidearms/BasePistol/BasePistol`), skin 229/231, level 233/235, chroma 237/239 (`rwc:40-44`). Then packed 94, packed 95, packed 5, 4 zero bits. Then an actor rep block (seed 94) with: bit 0; pointers h18=229, h19=233, h21=237; FGuid leaves h24..27 (skin uuid), h28..31 (level), h41..44 (chroma), each raw u32 (`rwc:51-62`: the UUID hex split into four 8-digit big-endian-text words); packed 0. Body < 8192 bits, else ValueError.
- Plain open: `build_dynamic_actor_reference(("Default__BasePistol_C", "/Game/Equippables/Guns/Sidearms/BasePistol/BasePistol"), 94, 95, 97, level 5, spawn_location=None)`, 837 bits.
- Failure behaviour: any `SkinInventoryError` (a ValueError subclass, `skin_inventory.py:15`), a JSON decode error or a catalog mismatch raised here propagates out of the `except ValueError` handler that contains the stage chain. Nothing up the stack catches it, so the responder process dies. Two consequences:
  - The output JSON (`ruh:4581-4584`) is never written.
  - `launch_range --keepalive` (`launch_range.py:304-322`) restarts the responder with the same command and empty state, but only if the same client PID and base are still running and the port is free.
  
  The restarted process has an empty `pending`, so every packet of the existing connection fails the `source in pending` gates and gets no reply. Nothing is sent to that client until it performs a new stateless handshake. The same holds for every other ValueError raised in sections 4-7.
- Side effect: `SkinInventory._read_state` rewrites `offline_skin_state.json` when it migrates legacy IDs (`skin_inventory.py:170-198`).

### 8.2 Movement correction and spawn location

- Spawn location (A06): the literal `(3581.634, -12019.322, 1294.153)` at `ruh:2357-2360`. It is independent of the CLI.
- `move_correction_location`: `launch.ps1:176` passes `--move-correction-location 3581.634 -12019.322 1294.153`. `launch_range.py:220-221` forwards it (not with authority role), and `run_probe` receives the tuple (`ruh:4915`). It is used **only** in the move-ack block (`ruh:4412-4535`, Part B), never in part A:
  - Gate: `move_ack_probe`, `source in client_restart_sent_sources`, >= 0.05 s since `last_move_ack_times`, and it runs inside the session-ack tick, so it is effectively <= 10 Hz. The move number comes from `read_saved_moves` (live ReadProcessMemory, `read_move_number.py:45`); `move_ack_value_source="number"` uses `moves["newest"]`. It sends only if that is strictly greater than `last_move_ack_numbers[src]`.
  - First qualifying tick with `moves["count"] > 0` and the source not in `move_correction_sent_sources`: `build_correction_rpc_payload(move_number, (3581.634,-12019.322,1294.153), 25, 53)` (`rmc:58-76`): SerializeInt(25, 54) = 5 bits, packed param bits, presence bit 1, then `build_packed_bits_struct(container)` (`rsb:399-423`). The container (`rmc:25-55`): bAckGoodMove 0, packed move number, 4 zero bits, NewLoc 3 x f64, NewVel 3 x 0.0 f64, bit 0, u32 0, 13 x u32 0, bit 0, 2 bits 0, u32 0, then bits 0,0,0,0,0. Encoded with seed 10, wrapped in `append_actor_rpc_content_block` (`rsb:256-265`: bit 0, bit 1, packed bits, payload) on an **unreliable** ch8 bunch. Adds the source to `move_correction_sent_sources`.
  - Every later tick (follow/track modes are off): `build_input_ack_rpc_payload(0xFFFFFFFF, move_number, 41, 53)` (`rsb:463-489`), the same wrapping. Vectors `M_correction_move100`, `M_input_ack_move101` in V.3.

## 9. Reference maps

### 9.1 Reliable sequence offsets (all `(B + off) & 0xFFF`)

| Ch | Actor | Part A offsets | Next user |
|---|---|---|---|
| 3 | WorldSettings | +0 A01 | none |
| 4 | GameState | +0 A02, +1 A07 | Part B +2 phase, +3 team array, +4..+6 match/gameplay/player array, +7 shop, dynamic from +8 |
| 5 | Controller | +0 A03, +1 A08, +2 A09 | Part B +3 ClientRestart, +4 ViewTarget, +5 ClientSetHUD, dynamic from +6 |
| 6 | PlayerState | +0 A04, +1 A10, +2 A11a, +3 A11b, +4 A12 | Part B +5..+10, dynamic from +11 |
| 7 | PlayerInfo | +0 A05, +1 A13 | no dynamic counter registered |
| 8 | Pawn | +0 A06, +1 A14, +2 A15, +3 A16, +4 A17, +5 A18, +6 A19, +7..+36 A20 (create +7+2(i-1), link +8+2(i-1)), +37 A22, +38..+42 A27 | Part B +43 pawn Owner, +44..+50 starting equippables, dynamic from +51 |
| 9 | BasePistol | +0 A21, +1 A23, +2 A24, +3 A25, +4 A26 | dynamic from +5 (`take_ch9_sequence`) |

### 9.2 NetGUIDs allocated in part A

| GUID | Object | Kind | Source |
|---|---|---|---|
| 2 | ShootingRangeGameState actor | dynamic actor | `rsb:203` |
| 3, 5, 7, 9 | AresWorldSettings, PersistentLevel, Range, `/Game/Maps/Poveglia/Range` | static export chain (5 = level used by every dynamic actor) | `rsb:131`, `rsb:143` |
| 4 / 15 / 17 | Controller actor / archetype / package | dynamic | `rsb:1473` |
| 6 / 19 / 21 | PlayerState | dynamic | `rsb:1485` |
| 8 / 23 / 25 | OwnerExclusivePlayerInfo | dynamic | `rsb:1492` |
| 10 / 27 / 29 | Wushu_PC pawn | dynamic | `rsb:1501` |
| 11 / 13 | GameState archetype / package | export | `rsb:203` |
| 60 | InventoryComponent (outer 10) | stable subobject | `ruh:2706` |
| 61 / 63 | `/Script/ShooterGame.ItemSlot` class / package | export (re-sent 16 times) | `ruh:2767-2769` |
| 62, 64, ..., 92 | ItemSlot 0..15 | dynamic subobject (outer 60, class 61) | `ruh:2769, 2890` |
| 94 / 95 / 97 | BasePistol actor / archetype / package | dynamic | `ruh:3005-3006` |
| 96 | EquippableStateMachine (outer 94) | stable subobject | `ruh:3188` |
| 98 | InactiveState (outer 94) | stable subobject | `ruh:3239` |
| 229/231, 233/235, 237/239 | skin, level, chroma CDO / package | static export (cosmetic open only) | `rwc:40-44` |

### 9.3 Handles and content-transform seeds

Seed = `actor_map_value` passed to `encode_short_actor_rep_payload`. The internal seed is `actor_map_value ^ payload_bits` (`rsb:1148`). It is always the **channel actor's** GUID, including for subobjects on that channel.

| Ch / object | Seed | Handles written (value) | Code label |
|---|---|---|---|
| ch4 GameState | 2 | 18 (bool 1) | unnamed |
| ch5 Controller | 4 | 15 (obj 6), 16 (obj 10) | PlayerState, Pawn |
| ch6 PlayerState | 6 | 30 (obj 8), 194 (obj 10), 195 (obj 10), 201..204 (u32 x4) | PlayerInfo, SpawnedCharacter, PossessedCharacter, DesiredClass A..D (`ruh:2468-2540`) |
| ch7 PlayerInfo | 8 | 15 (obj 4) | AresController (`ruh:2575-2577`) |
| ch8 Pawn | 10 | 15 (obj 6), 16 (obj 4), 4 (3-bit role 2) | PlayerState, Controller, RemoteRole |
| ch8 Inventory 60 | 10 | 3..18 (obj 62+2i), 25 (obj 94), 26 (u32 idx) | ItemSlots[0..15], CurrentEquippable, CorrectionIndex (`ruh:3308-3311`; 27 LastSeen is never sent) |
| ch8 ItemSlot 64 | 10 | 1 (obj 94) | Contents |
| ch9 BasePistol | 94 | 12 (obj 10), 4 (3-bit role 2); cosmetic 18/19/21 obj, 24-27/28-31/41-44 u32 | `ruh:3098` says OwnerPawn +0x210; `rsb:697,722` says AActor.Owner. The wire is the same. |
| ch9 SM 96 | 94 | 3 (obj 98) | AuthCurrentState |

### 9.4 Inline body shapes (`BitWriter`, LSB-first; `rsb:16-75`)

- Packed uint (`rsb:27-35`): 7 bits per byte, continuation in bit 0: `byte = (part << 1) | more`.
- FString (`rsb:67-72`): u32 (len+1), ASCII bytes, NUL.
- **Stable subobject export + content block** (A17, A25, A26): `0`(1) `1`(32) `packed sub` `5`(8) `packed outer` `0`(8) `FString name` `0`(32) | `1`(1) `0`(1) `packed target` `1`(1) | `packed repbits` `enc`. The target is the exported subobject itself for A17 (60) and A25 (96). **For A26 they differ:** the export is InactiveState 98, but the content-block target is the state machine 96 (`ruh:135`).
- **Class export** (A18, A20 create): `0`(1) `1`(32) `packed 61` `5`(8) `packed 63` `5`(8) `packed 0` `FString "/Script/ShooterGame"` `0`(32) `FString "ItemSlot"` `0`(32).
- **Dynamic subobject content block** (A18, A20, A22): `1`(1) `0`(1) `packed new/target guid` `0`(1 bStablyNamed) `0`(1 extra writer bit, `ruh:2786-2789`) `packed 61` `0`(1 bActorIsOuter) `packed 60` `packed repbits` `enc`.
- **Stably-named subobject content block** (A19, A20 link, A27): `1`(1) `0`(1) `packed 60` `1`(1) `packed repbits` `enc`.
- **Channel-actor content block** (A07-A16, A23, A24): `1`(1) `1`(1) `packed repbits` `enc`.
- Empty rep payload: `0`(1) `packed 0` = 9 bits.

### 9.5 Library equivalents (verified byte-identical offline)

A22 = `build_item_slot_contents_block(1, 94)` (`rsb:733`); A23 = `build_channel_actor_owner_block(94, 10)` (`rsb:721`); A26 = `build_pistol_auth_state_body("InactiveState", True)` (`rsb:771`); A27 = `build_inventory_correction_body(idx, 94)` (`rsb:813`). The Rust port can implement each once.

## 10. Dependency graph

```
stateless handshake (pending[src])
  -> hello (232-bit, stored)  -> [77-bit pkt] challenge  (sets S0, B)
  -> [NMT_Login 5] welcome -> [NMT_Join 9] join-ack (sets last_session_ack_time)
  -> session-ack tick (>=0.1 s since last ack; carries every stage below)
A01 ch3 open ──2.0──> A02 ch4 open ──2.0──> A03 ch5 open ──2.0──> A04 ch6 open ──2.0──> A05 ch7 open ──2.0──> A06 ch8 open
A06 ──3.0──> A07 GS h18 ──3.0──> A08 PC h15 ──3.0──> A09 PC h16 ──3.0──> A10 PS h30 ──0.5──> A11 PS h194+h195 ──0.5──> A12 PS h201-204
A12 (+A05) ──0.5──> A13 PI h15
A12 (+A09) ──1.0──> A14 Pawn h15 ──3.0──> A15 Pawn h16 ──1.0──> A16 Pawn role
A16 ──1.5──> A17 Inv open ──2.0──> A18 Slot0 create ──1.5──> A19 Slots[0] ──1.0──> A20 i=1 ──1.0──> ... ──> A20 i=15
A20 i=15 ──2.0──> A21 ch9 pistol open ──2.0──> A22 Slot1.Contents ──1.0──> A23 h12 ──1.0──> A24 role ──1.5──> A25 SM open ──1.0──> A26 InactiveState
A26 ──2.0──> A27 idx1 ──2.0──> idx2 ──2.0──> idx3 ──2.0──> idx4 ──2.0──> idx5
Outward (Part B): A16(+A15) ─3.0─> client_restart; A21 (+game_state_phase) ─1.0─> team_state_open; A26 + A27x5 -> pistol mirror (+ not replaced by a purchase), input correction (+ starting equippables done), pawn_owner (+ player_unique_id)
```

## 11. Port notes for the 128 tps external server, and console logging

- Python has no tick. It is receive-driven (`recvfrom` with a 0.25 s timeout, `ruh:1412`), and every gate above is a `time.monotonic()` difference evaluated while one received datagram is handled. In a 128 tps loop (7.8125 ms), drain the socket every tick and handle each datagram in arrival order with the same gates, comparing against a monotonic clock (0.1 s = 12.8 ticks; do not round the gate to a tick count). A tick-quantized implementation shifts each stage by at most one tick. The shifts **accumulate**, because every gate is measured from the previous stage's own send time. In the verifier's re-run, the client sent a packet every 1/128 s and A27 idx 5 landed 67.64 s after the join ack, against 66.6 s nominal. Quantization changes no bits beyond which client packet gets acked, that is, the C, mask and S values in the frame headers.
- Keep the per-datagram order of section 3 (Part B per-packet frames, then session ack, then at most one stage, then move ack), the single shared 14-bit server sequence per connection, and "one stage arm per session-ack tick".
- Do not emit periodic acks from the tick in the Python-parity mode (production heartbeat is off).
- Console: Python prints one JSON object per received datagram to stdout (`ruh:1373-1377, 4578`). The exceptions are non-loopback datagrams and the challenge arm's `continue` (`ruh:2135`), which print nothing. Python also prints non-JSON lines: `listening on ...` (`ruh:1414`) and the handler/receive-key fingerprint line (`ruh:1482`). `launch_range.py:288-291` redirects all of this, stderr tracebacks included, to `runtime/responder_<pid>_phase4_match_state.out` with no window. For the external server, allocate a console (Windows `AllocConsole`) and log one line per datagram with the same keys: `elapsed_seconds` (monotonic seconds since the responder started, rounded to 1 ms, `ruh:1463`), `source_port`, `length`, the inspection fields (`client_packet_sequence, payload_bits, channel_index, control_message_type, ...`), and the per-stage keys listed in section 7 (`sent_session_ack_length, acked_client_sequence, server_sequence, sent_*_length, *_payload`). Keeping the key names lets the shadow-mode diff in `PORT_PLAN_v2.md` section 4 compare the two logs line by line.

## 12. Open questions

1. The stage delays (0.5-3.0 s, about 67 s nominal for part A) are empirical pacing. No code comment says the client requires them. Whether an ack-driven graph (port plan) may shorten them needs a live test.
2. Keys and move numbers come from ReadProcessMemory of the client (`ruh:1474`, `read_move_number.py:45`). The Rust server needs another source (internal oracle, or decoding ServerMove from client packets) before A-chain parity can be tested live.
3. On a cosmetic-resolution error at A21, Python crashes and restarts empty (section 8.1). The Rust behaviour (crash parity vs. plain-open fallback) needs a user decision.
4. Reading the skin state at A21 can rewrite `offline_skin_state.json` (legacy-ID migration). Ownership of that file in the port (Rust `platform`/codex) needs agreeing.
5. Any authenticated 232-bit packet after join is swallowed as "hello" (no ack that tick). Reproduce this or treat it as a bug?
6. Per-source one-shot sets are never cleared. A same-port reconnect would not get a new chain. The real client uses a new port per travel, which is unverified for every case.
7. No retransmission and no tracking of the server's sent packets. The port plan adds retransmission; retransmitted frames cannot be bit-compared with Python.
8. The GameState handle 18 bool and BasePistol handle 12 (OwnerPawn vs AActor.Owner) have no settled names in code. The wire is unaffected.
9. `channel_first_dynamic_offset` is process-global (keyed by channel only, not by source). Part B sets it to 0 for newly allocated channels (`ruh:1038, 1085, 1197, 1225, 1965, 2042, 2056`): projectiles from 10000 (`range_projectiles.py:60`), bots from 400 (`range_bots.py:44`) and purchases from 20 (`range_gun_loader.py:38`). None of those writes touches ch3-9. Part A interacts with this table only through the ch8 (+51) and ch9 (+5) entries, whose `take_channel_sequence` counters start past part A's last fixed offsets (+42 and +4). The per-(source, channel) counters live in `channel_next_offset` (`ruh:787-793`).

## Appendix V. Test vectors

Generator: `gen_bringup_a.py` (scratchpad, not committed). It imports `range_udp_handshake`, `range_server_bunch`, `range_move_correction` and `range_weapon_cosmetics` read-only and copies the inline bodies from `ruh:2706-3333`. Frames were verified by AES-GCM decrypt (nonce `u32 0, u32 0x7908CBDD`, 4-byte tag) plus `inverse_short_packet`, and by an independent re-serialization of the plaintext header (38/38 match). `wrap_gcm` always uses counter 0 (`ruh:419-421`), so frames are deterministic for a given key.

### V.1 Synthetic client packets (decoded plaintext, pre-transform)

Built exactly as `test_range_udp_handshake.py:260-298` builds them. `hello`, `login`, `join` are 232-bit control-bunch packets on ch0 (channel name 255, chseq 357) whose first body byte is the NMT type (0, 5, 9). `short` is a 77-bit ack-only client packet. **These are builder inputs only.** In the live dispatch, any 232-bit packet is caught by the hello arm (`ruh:2109`), so a 232-bit login or join would never reach the welcome or join-ack arm. The verifier's dispatch re-run (Verifier corrections) therefore used a 536-bit login and a 336-bit join (same header template, longer bodies).

| packet | bits | hex |
|---|---|---|
| hello | 232 | `0000000000000000ff3701a02cfe0b8001000000000000000000000080` |
| login | 232 | `0000000000000000ff3701a02cfe0b8081020000000000000000000080` |
| join | 232 | `0000000000000000ff3701a02cfe0b8081040000000000000000000080` |
| short | 77 | `d036676500000000059a` |

`short` header: client packet sequence C = 6489, acked-server field = 13165, so the challenge packet sequence S0 = 13166 and the actor-channel reliable base B = (S0+1) & 0xFFF = 879.

### V.2 Handshake replies and session ack (key 00..1f, stateless_header 24)

#### Challenge (NMT 3)

Call: `build_server_challenge_probe(hello, 232, short, 77, 24, key)` -> client_sequence 6489, server_sequence 13166, frame 36 bytes, plaintext 176 bits.

plaintext packet (pre-transform):
```
9095b9cd03000000011001e06dfe0ba0800100000080
```
UDP frame:
```
0000000018000000400bf1da37aa06b1c0347cd49233e226c1f4a58eb034d13bd9351173
```

#### Welcome (NMT 1)

Call: `build_server_welcome_probe(hello, 232, login, 232, 24, key, 13167, "/Game/Maps/Poveglia/Range")` -> client_sequence 0, server_sequence 13167, frame 70 bytes, plaintext 448 bits.

plaintext packet (pre-transform):
```
0000bccd07000000011001006efe0be084000d00008097a3b0b6b297a630b8b917a837bbb233b6b4b017a930b7b332000000000000000080
```
UDP frame:
```
0000000018000000c058b04016aa0db140ec68548933a202c1f4658af0a492cc4d3ae98996cdf811ae4cbc2929e4fa9a90b4605ccda7a9aae9270f14920bac18
2a09c2dd0273
```

#### Join ACK

Call: `build_server_join_ack(join, 232, 24, key, 13168)` -> client_sequence 0, server_sequence 13168, frame 24 bytes, plaintext 77 bits.

plaintext packet (pre-transform):
```
0000c0cd030000000110
```
UDP frame:
```
000000001800000080ca343d0eaa06b140ec68947e8e760e
```

#### Session ACK

Call: `build_server_session_ack(short, 77, 24, key, 13169, ack_history_mask=3)` -> client_sequence 6489, server_sequence 13169, frame 24 bytes, plaintext 77 bits.

plaintext packet (pre-transform):
```
9095c5cd030000000110
```
UDP frame:
```
000000001800000040101ff627aa06b1c0347c54448e760e
```

### V.3 Stage bunches

All frames: receive_key = bytes 00..1f, stateless_header = 24, acked_client_sequence = 6489, ack_history_mask = 3, channel_name_index = 102, base B = 879. Server sequences are synthetic (one session ack assumed before each stage tick, so stage S values step by 2; two-frame ticks use S and S+1). `body` is the key-independent bunch body passed to `build_server_bunch_frame`; `rep_plain` is the property stream before `encode_short_actor_rep_payload`; `rep_encoded` is after it (seed = actor_map_value). `plaintext` is the whole packet before `transform_short_packet`/`wrap_gcm`.

| vector | ch | reliable | open | exports | chseq (B+off) | S | body bits | rep bits | seed | packet bits | frame bytes |
|---|---|---|---|---|---|---|---|---|---|---|---|
| A01_world_settings_open | 3 | 1 | 1 | 1 | 879 (+0) | 13170 | 897 | - | - | 1025 | 142 |
| A02_game_state_open | 4 | 1 | 1 | 1 | 879 (+0) | 13172 | 925 | - | - | 1053 | 146 |
| A03_controller_open | 5 | 1 | 1 | 1 | 879 (+0) | 13174 | 893 | - | - | 1021 | 142 |
| A04_player_state_open | 6 | 1 | 1 | 1 | 879 (+0) | 13176 | 957 | - | - | 1085 | 150 |
| A05_player_info_open | 7 | 1 | 1 | 1 | 879 (+0) | 13178 | 661 | - | - | 789 | 113 |
| A06_pawn_open | 8 | 1 | 1 | 1 | 879 (+0) | 13180 | 707 | - | - | 835 | 119 |
| A07_game_state_bool_h18 | 4 | 1 | 0 | 0 | 880 (+1) | 13182 | 28 | 18 | 2 | 156 | 34 |
| A08_controller_playerstate_h15 | 5 | 1 | 0 | 0 | 880 (+1) | 13184 | 35 | 25 | 4 | 163 | 35 |
| A09_controller_pawn_h16 | 5 | 1 | 0 | 0 | 881 (+2) | 13186 | 35 | 25 | 4 | 163 | 35 |
| A10_playerstate_playerinfo_h30 | 6 | 1 | 0 | 0 | 880 (+1) | 13188 | 35 | 25 | 6 | 163 | 35 |
| A11a_playerstate_spawned_character_h194 | 6 | 1 | 0 | 0 | 881 (+2) | 13190 | 43 | 33 | 6 | 171 | 36 |
| A11b_playerstate_possessed_character_h195 | 6 | 1 | 0 | 0 | 882 (+3) | 13191 | 43 | 33 | 6 | 171 | 36 |
| A12_playerstate_desired_class_h201_204 | 6 | 1 | 0 | 0 | 883 (+4) | 13193 | 219 | 201 | 6 | 347 | 58 |
| A13_playerinfo_controller_h15 | 7 | 1 | 0 | 0 | 880 (+1) | 13195 | 35 | 25 | 8 | 163 | 35 |
| A14_pawn_playerstate_h15 | 8 | 1 | 0 | 0 | 880 (+1) | 13197 | 35 | 25 | 10 | 163 | 35 |
| A15_pawn_controller_h16 | 8 | 1 | 0 | 0 | 881 (+2) | 13199 | 35 | 25 | 10 | 163 | 35 |
| A16_pawn_remote_role_h4 | 8 | 1 | 0 | 0 | 882 (+3) | 13201 | 30 | 20 | 10 | 158 | 34 |
| A17_inventory_open_guid60 | 8 | 1 | 0 | 1 | 883 (+4) | 13203 | 309 | 9 | 10 | 437 | 69 |
| A18_item_slot0_create_guid62 | 8 | 1 | 0 | 1 | 884 (+5) | 13205 | 479 | 9 | 10 | 607 | 90 |
| A19_item_slot0_link_h3 | 8 | 1 | 0 | 0 | 885 (+6) | 13207 | 44 | 25 | 10 | 172 | 36 |
| A20_scale_slot1_create_guid64 | 8 | 1 | 0 | 1 | 886 (+7) | 13209 | 479 | - | - | 607 | 90 |
| A20_scale_slot1_link_h4 | 8 | 1 | 0 | 0 | 887 (+8) | 13210 | 44 | 25 | 10 | 172 | 36 |
| A20_scale_slot15_create_guid92 | 8 | 1 | 0 | 1 | 914 (+35) | 13212 | 479 | - | - | 607 | 90 |
| A20_scale_slot15_link_h18 | 8 | 1 | 0 | 0 | 915 (+36) | 13213 | 44 | 25 | 10 | 172 | 36 |
| A21_base_pistol_open_plain | 9 | 1 | 1 | 1 | 879 (+0) | 13215 | 837 | - | - | 965 | 135 |
| A21_base_pistol_open_cosmetic | 9 | 1 | 1 | 1 | 879 (+0) | 13215 | 5104 | - | - | 5232 | 668 |
| A22_slot1_contents_pistol_h1 | 8 | 1 | 0 | 0 | 916 (+37) | 13217 | 62 | 25 | 10 | 190 | 38 |
| A23_pistol_owner_h12 | 9 | 1 | 0 | 0 | 880 (+1) | 13219 | 35 | 25 | 94 | 163 | 35 |
| A24_pistol_remote_role_h4 | 9 | 1 | 0 | 0 | 881 (+2) | 13221 | 30 | 20 | 94 | 158 | 34 |
| A25_pistol_state_machine_open_guid96 | 9 | 1 | 0 | 1 | 882 (+3) | 13223 | 341 | 9 | 94 | 469 | 73 |
| A26_inactive_state_link_guid98 | 9 | 1 | 0 | 1 | 883 (+4) | 13225 | 285 | - | - | 413 | 66 |
| A27_auth_correct_idx1 | 8 | 1 | 0 | 0 | 917 (+38) | 13227 | 84 | 65 | 10 | 212 | 41 |
| A27_auth_correct_idx2 | 8 | 1 | 0 | 0 | 918 (+39) | 13229 | 84 | 65 | 10 | 212 | 41 |
| A27_auth_correct_idx3 | 8 | 1 | 0 | 0 | 919 (+40) | 13231 | 84 | 65 | 10 | 212 | 41 |
| A27_auth_correct_idx4 | 8 | 1 | 0 | 0 | 920 (+41) | 13233 | 84 | 65 | 10 | 212 | 41 |
| A27_auth_correct_idx5 | 8 | 1 | 0 | 0 | 921 (+42) | 13235 | 84 | 65 | 10 | 212 | 41 |
| M_correction_move100 | 8 | 0 | 0 | 0 | - | 13237 | 942 | 924 | 10 | 1047 | 145 |
| M_input_ack_move101 | 8 | 0 | 0 | 0 | - | 13239 | 90 | 80 | 10 | 195 | 39 |

#### A01_world_settings_open

- body: 897 bits

body:
```
020000000c0a140a1c0a240a00340000005e8ec2daca5e9ac2e0e65ea0deeccaced8d2c25ea4c2dcceca00000000000c000000a4c2dcceca0000000000200000
00a0cae4e6d2e6e8cadce898caeccad800000000002400000082e4cae6aedee4d8c8a6cae8e8d2dccee600000000000c00
```
plaintext packet (1025 bits):
```
9095c9cd0300000001300de26d32070e0100000006050a050e051205001a0000002f47616d652f4d6170732f506f7665676c69612f52616e6765000000000006
00000052616e676500000000001000000050657273697374656e744c6576656c00000000001200000041726573576f726c6453657474696e6773000000000006
01
```
UDP frame (142 bytes):
```
000000001800000080003d9e1daa06b1c0347cd4ea21628e73d465a741551a7c63fa24b1f0182bb5ab8974a2cd3d01c9f266a93ba862facc74955e0ea066ac8f
b1caae604b12e13207b57528162a0ab0acdf2fa57684dc395e6654d70a94bfd9398f9df5d122b5787fdcbd7db9d523182763dd4d06bbab9491bd0473908d5a6e
9128a830dfb7e3f21bda2c751be2
```

#### A02_game_state_open

- body: 925 bits

body:
```
020000002c0a340a006a0000005e8ec2daca5e8ec2daca9adec8cae65ea6d0dedee8d2dccea4c2dcceca5ea6d0dedee8d2dccea4c2dcceca8ec2dacaa6e8c2e8
ca00000000004400000088caccc2ead8e8bebea6d0dedee8d2dccea4c2dcceca8ec2dacaa6e8c2e8cabe860000000000082c1400
```
plaintext packet (1053 bits):
```
9095d1cd03000000013011e26d32770e0100000016051a0500350000002f47616d652f47616d654d6f6465732f53686f6f74696e6752616e67652f53686f6f74
696e6752616e676547616d65537461746500000000002200000044656661756c745f5f53686f6f74696e6752616e676547616d6553746174655f430000000000
04160a10
```
UDP frame (146 bytes):
```
00000000180000004023335026aa06b1c0347cd4df21628e5ad465274ad50fbc5cfa24b1b078b46e6a2c0518155c2a491a57d24ae3d6a57c145c4bc9bba6998f
b19185524b441d0d11b535ee122a0ab02ceb353e9c9fcba5c55ee9cf274ebb457dc35416afd4a4f8ab243ef5438c3604f1cb4a83deeb1b5a8c774bea8ccd24f1
10a28330e9b723d19b9a3f75894d48e3600e
```

#### A03_controller_open

- body: 893 bits

body:
```
020000003c0a440a005e0000005e8ec2daca5e86d0c2e4c2c6e8cae4e65ebe86dee4ca5ea6d0dedee8d2dccea4c2dcceca86dedce8e4ded8d8cae40000000000
4600000088caccc2ead8e8bebea6d0dedee8d2dccea4c2dcceca86dedce8e4ded8d8cae4be860000000000103c140000
```
plaintext packet (1021 bits):
```
9095d9cd03000000013015e26d32f70d010000001e052205002f0000002f47616d652f436861726163746572732f5f436f72652f53686f6f74696e6752616e67
65436f6e74726f6c6c657200000000002300000044656661756c745f5f53686f6f74696e6752616e6765436f6e74726f6c6c65725f430000000000081e0a0010
```
UDP frame (142 bytes):
```
000000001800000000cc54ab23aa06b1c0347cd4dd2122f34fd465676a55197c72fa24b1b078b46eaa3c05d830c07cea1457d27b63baffc225d860fbfe46d11a
3c92c5d4e609f55a04f8b0649341fa43712c93149583dc395e0c69074be81d9ed0737b166f2d127bd4cd2e7b79f75e9b3c285bc4a5f44c6aef575fdacf793a6d
cb38684a70011fe2c4e90255460e
```

#### A04_player_state_open

- body: 957 bits

body:
```
020000004c0a540a006e0000005e8ec2daca5e8ec2daca9adec8cae65ea6d0dedee8d2dccea4c2dcceca5ea6d0dedee8d2dccea4c2dccecaa0d8c2f2cae4a6e8
c2e8ca00000000004800000088caccc2ead8e8bebea6d0dedee8d2dccea4c2dccecaa0d8c2f2cae4a6e8c2e8cabe860000000000184c1400
```
plaintext packet (1085 bits):
```
9095e1cd03000000013019e26d32f70e0100000026052a0500370000002f47616d652f47616d654d6f6465732f53686f6f74696e6752616e67652f53686f6f74
696e6752616e6765506c61796572537461746500000000002400000044656661756c745f5f53686f6f74696e6752616e6765506c6179657253746174655f4300
000000000c260a10
```
UDP frame (150 bytes):
```
000000001800000040fe45050faa06b1c0347c54c12162ce4fd465275e15253c5dfa24b1b078b46e2a0f0558385c2a491a57d24ae3d6a57c145c4bc9bba6998f
b19185524b441d0d11b535ee122a0ab02ceb353e5cebf665bc5c2ce31d4ebb057d4192dc51e98cb67477416f3d38134eea2f0bf1acc5393f1c0a7d4bea18dd05
2d5eab490fcb0591331d3b3298872be6393cef075a0e
```

#### A05_player_info_open

- body: 661 bits

body:
```
020000005c0a640a00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000004400000088caccc2ead8e8bebe9eeedccae48af0c6d8eae6d2ecca
a0d8c2f2cae492dcccde0000000000205c1400
```
plaintext packet (789 bits):
```
9095e9cd0300000001301de26d32570a010000002e05320500140000002f5363726970742f53686f6f74657247616d6500000000002200000044656661756c74
5f5f4f776e65724578636c7573697665506c61796572496e666f0000000000102e0a10
```
UDP frame (113 bytes):
```
0000000018000000c0de941e26aa06b1c0347c54ca21e26b50d42579445506bc42fa24b1b078b4e549170558f879d267719b974810efc68c1e584bfbfe0bec0c
2a09c2dd82e0258d52b307ed354f639f0fd0196a8259a8df33b03ff6e526dd46e5343d561f8fd82a99e1676f23b81c4a0e
```

#### A06_pawn_open

- body: 707 bits

body:
```
020000006c0a740a00400000005e8ec2daca5e86d0c2e4c2c6e8cae4e65eaeeae6d0ea5eaeeae6d0eabea08600000000002800000088caccc2ead8e8bebeaeea
e6d0eabea086be860000000000286c1496a22ff2a7a2a30c00
```
plaintext packet (835 bits):
```
9095f1cd03000000013021e26d320f0b0100000036053a0500200000002f47616d652f436861726163746572732f57757368752f57757368755f504300000000
001400000044656661756c745f5f57757368755f50435f43000000000014360a4bd117f953d1510604
```
UDP frame (119 bytes):
```
0000000018000000c080517c3eaa06b1c0347c54d621627871d465277f9532fc63fa24b1b078b46e2a1905d830c07cea1457d27b63ba7f980ad860fb7ef9bac1
6ec7fd1458122111876dab2ac88fdb3b41ce2f9576ff3a8b8d29947b6126ffedc17910893997583cbb247e6fbdb3eb3b604cf997518503
```

#### A07_game_state_bool_h18

- rep_plain (18 bits): `480200`
- rep_encoded (18 bits, seed 2): `b2f403`
- body: 28 bits

body:
```
93c8d20f
```
plaintext packet (156 bits):
```
9095f9cd03000000011011006e3273804964e90f
```
UDP frame (34 bytes):
```
0000000018000000c031a42531aa06b1c0347c94c521e22ec7f465ca342975e13407
```

#### A08_controller_playerstate_h15

- rep_plain (25 bits): `3c180000`
- rep_encoded (25 bits, seed 4): `56815d01`
- body: 35 bits

body:
```
cb58057605
```
plaintext packet (163 bits):
```
909501ce03000000011015006e328f8065ac02bb06
```
UDP frame (35 bytes):
```
000000001800000000094d483aaa06b1c0347c2dd121e26ee1f4654a202d13a1338903
```

#### A09_controller_pawn_h16

- rep_plain (25 bits): `40280000`
- rep_encoded (25 bits, seed 4): `24c25d01`
- body: 35 bits

body:
```
cb90087705
```
plaintext packet (163 bits):
```
909509ce03000000011015206e328f80654884bb06
```
UDP frame (35 bytes):
```
0000000018000000c07672dc1caa06b1c0347c2dcf21e26ee1f4e55860666ba7338903
```

#### A10_playerstate_playerinfo_h30

- rep_plain (25 bits): `78200000`
- rep_encoded (25 bits, seed 6): `264bce00`
- body: 35 bits

body:
```
cb982c3903
```
plaintext packet (163 bits):
```
909511ce03000000011019006e328f80654c969c05
```
UDP frame (35 bytes):
```
0000000018000000408c697d2faa06b1c0347c6de921e26ee1f4658ab27c05bcf38903
```

#### A11a_playerstate_spawned_character_h194

- rep_plain (33 bits): `0a05280000`
- rep_encoded (33 bits, seed 6): `d614a86300`
- body: 43 bits

body:
```
0b5953a08e01
```
plaintext packet (171 bits):
```
909519ce03000000011019206e32af8085ac2950c704
```
UDP frame (36 bytes):
```
000000001800000000edceee00aa06b1c0347cedfc21e2eedef4e598b21f573d14d5a703
```

#### A11b_playerstate_possessed_character_h195

- rep_plain (33 bits): `0e05280000`
- rep_encoded (33 bits, seed 6): `d6146c4800`
- body: 43 bits

body:
```
0b5953b02101
```
plaintext packet (171 bits):
```
90951dce03000000011019406e32af8085ac29d89004
```
UDP frame (36 bytes):
```
00000000180000004062466c0daa06b1c0347cadd321e2eedef4e59a7200573d94d3a703
```

#### A12_playerstate_desired_class_h201_204

- rep_plain (201 bits): `26057488ac5b2b0528c87b832e051acb5bed33052a9dfe4c0000`
- rep_encoded (201 bits, seed 6): `e793b10eb188f4d67a4bcf8b8b566f3312ab47c00d46ce139f00`
- body: 219 bits

body:
```
4f0a9c4fc63ac422d25beb2d3d2f2e5abdcd48ac1e013718394f7c02
```
plaintext packet (347 bits):
```
909525ce03000000011019606e326f832705ce27631d6211e9adf5969e1717adde6624568f801b8c9c273e05
```
UDP frame (58 bytes):
```
0000000018000000004889e52baa06b1c0347cadd6e108c4d5f4e5873215aec631c7a7c56804bab5197ff740c04f7d8af2ee34842140c73bb503
```

#### A13_playerinfo_controller_h15

- rep_plain (25 bits): `3c100000`
- rep_encoded (25 bits, seed 8): `d140db01`
- body: 35 bits

body:
```
cb44036d07
```
plaintext packet (163 bits):
```
90952dce0300000001101d006e328f8065a281b607
```
UDP frame (35 bytes):
```
000000001800000040b179e405aa06b1c0347cede821e26ee1f465800a2a499c738903
```

#### A14_pawn_playerstate_h15

- rep_plain (25 bits): `3c180000`
- rep_encoded (25 bits, seed 10): `e62ff500`
- body: 35 bits

body:
```
cb98bfd403
```
plaintext packet (163 bits):
```
909535ce03000000011021006e328f8065cc5fea05
```
UDP frame (35 bytes):
```
0000000018000000400d9ef618aa06b1c0347caddc21e26ee1f4650a907f27b6f38903
```

#### A15_pawn_controller_h16

- rep_plain (25 bits): `40100000`
- rep_encoded (25 bits, seed 10): `947af500`
- body: 35 bits

body:
```
cb50ead503
```
plaintext packet (163 bits):
```
90953dce03000000011021206e328f806528f5ea05
```
UDP frame (35 bytes):
```
0000000018000000408a106212aa06b1c0347cade721e26ee1f4e5189017f192f38903
```

#### A16_pawn_remote_role_h4

- rep_plain (20 bits): `100400`
- rep_encoded (20 bits, seed 10): `10e307`
- body: 30 bits

body:
```
a3408c1f
```
plaintext packet (158 bits):
```
909545ce03000000011021406e327b805120c62f
```
UDP frame (34 bytes):
```
000000001800000000e1a7bd26aa06b1c0347c2dff21e2eeccf4e51a90507cd9c01c
```

#### A17_inventory_open_guid60

- rep_plain (9 bits): `0000`
- rep_encoded (9 bits, seed 10): `ca00`
- body: 309 bits

body:
```
02000000f00a28002600000092dceccadce8dee4f286dedae0dedccadce80000000000c22ba10c
```
plaintext packet (437 bits):
```
90954dce03000000011021626e32d704010000007805140013000000496e76656e746f7279436f6d706f6e656e740000000000e1955016
```
UDP frame (69 bytes):
```
000000001800000080566b9b11aa06b1c0347cedce21629dc1f4e5375015ce196afa24b13075585386342fd8f0736c366ac3dda2aea8a12a59113e452188503d
dde2b1580e
```

#### A18_item_slot0_create_guid62

- rep_encoded (9 bits, seed 10): `ca00`
- body: 479 bits

body:
```
02000000f40afc0a00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000001200000092e8cadaa6d8dee80000000000e2430f9e8432
```
plaintext packet (607 bits):
```
909555ce03000000011021826e327f07010000007a057e0500140000002f5363726970742f53686f6f74657247616d650000000000090000004974656d536c6f
740000000000f1a1074f4259
```
UDP frame (90 bytes):
```
0000000018000000c0619c3420aa06b1c0347c6df321e20ac5f4e51350d53d7c4cfa24b1b078b4e549170558f879d267719b974810efc68c1e584bfbbe32ec0c
2a09c2dd02b0398712b1c7a559a95b3f87a33dd5249f68d7bb39
```

#### A19_item_slot0_link_h3

- rep_plain (25 bits): `0cf80000`
- rep_encoded (25 bits, seed 10): `130df500`
- body: 44 bits

body:
```
e1959968a807
```
plaintext packet (172 bits):
```
90955dce03000000011021a06e32b380f0ca4c34d40b
```
UDP frame (36 bytes):
```
000000001800000040d3e38a13aa06b1c0347cadcae1c8aed0f4a50290c302752d350c07
```

#### A20_scale_slot1_create_guid64

- body: 479 bits

body:
```
02000000f40afc0a00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000001200000092e8cadaa6d8dee8000000000002440f9e8432
```
plaintext packet (607 bits):
```
909565ce03000000011021c26e327f07010000007a057e0500140000002f5363726970742f53686f6f74657247616d650000000000090000004974656d536c6f
74000000000001a2074f4259
```
UDP frame (90 bytes):
```
0000000018000000004401ba37aa06b1c0347c6dd021fe0ac5f4251150d53d7c4cfa24b1b078b4e549170558f879d267719b974810efc68c1e584bfbbe32ec0c
2a09c2dd02b0398712b1c7a559a95b4d80a33dd5249f68d7bb39
```

#### A20_scale_slot1_link_h4

- rep_plain (25 bits): `10000100`
- rep_encoded (25 bits, seed 10): `98b00f00`
- body: 44 bits

body:
```
e195c1847d00
```
plaintext packet (172 bits):
```
909569ce03000000011021e06e32b380f0ca60c23e08
```
UDP frame (36 bytes):
```
0000000018000000003244d80caa06b1c0347c2dec61f7aed0f4a50e509a1b756df30c07
```

#### A20_scale_slot15_create_guid92

- body: 479 bits

body:
```
02000000f40afc0a00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000001200000092e8cadaa6d8dee80000000000c2450f9e8432
```
plaintext packet (607 bits):
```
909571ce030000000110214272327f07010000007a057e0500140000002f5363726970742f53686f6f74657247616d650000000000090000004974656d536c6f
740000000000e1a2074f4259
```
UDP frame (90 bytes):
```
0000000018000000804ee0431aaa06b1c0347c2dc821e50ac5f4253750d53d7c4cfa24b1b078b4e549170558f879d267719b974810efc68c1e584bfbbe32ec0c
2a09c2dd02b0398712b1c7a559a95b0d90a33dd5249f68d7bb39
```

#### A20_scale_slot15_link_h18

- rep_plain (25 bits): `48700100`
- rep_encoded (25 bits, seed 10): `2f510f00`
- body: 44 bits

body:
```
e19579897a00
```
plaintext packet (172 bits):
```
909575ce03000000011021607232b380f0cabc443d08
```
UDP frame (36 bytes):
```
0000000018000000006b62090eaa06b1c0347cedc421e5aed0f4e50790c921756dfb0c07
```

#### A21_base_pistol_open_plain

- body: 837 bits

body:
```
020000007c0b840b006c0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6d2c8cac2e4dae65e84c2e6caa0d2e6e8ded85e84c2e6caa0d2e6e8
ded800000000002c00000088caccc2ead8e8bebe84c2e6caa0d2e6e8ded8be860000000000787d1500
```
plaintext packet (965 bits):
```
90957dce03000000013025e26d32170d01000000be05c20500360000002f47616d652f45717569707061626c65732f47756e732f5369646561726d732f426173
65506973746f6c2f42617365506973746f6c00000000001600000044656661756c745f5f42617365506973746f6c5f430000000000bcbe0a10
```
UDP frame (135 bytes):
```
000000001800000080d407d11caa06b1c0347cade22122f371d4656751952a7c41fa24b1b078b46eea0205587869c6122657925963f053eb970b3ef0b951912a
c4ab87ee850bd7955c10555e703f81d8321f13c097d9e3e59a9d8b5f034e3b12fdb46616b4e2e99f8224be55fdc144fe23e84d62343d3b54bb7629d3ceae463c
a819d288ce640e
```

#### A21_base_pistol_open_cosmetic

- body: 5104 bits

body:
```
080000007c0b840b006c0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6d2c8cac2e4dae65e84c2e6caa0d2e6e8ded85e84c2e6caa0d2e6e8
ded800000000002c00000088caccc2ead8e8bebe84c2e6caa0d2e6e8ded8be86000000000096050a9e050a009e0000005e8ec2daca5e8ae2ead2e0e0c2c4d8ca
e65e8eeadce65ea6d2c8cac2e4dae65e84c2e6caa0d2e6e8ded85e86deeccadc5e84c2e6caa0d2e6e8ded8be86deeccadcbea0e4d2dac2e4f282e6e6cae80000
0000005200000088caccc2ead8e8bebe84c2e6caa0d2e6e8ded8be86deeccadcbea0e4d2dac2e4f282e6e6cae8be860000000000a6050aae050a00b40000005e
8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6d2c8cac2e4dae65e84c2e6caa0d2e6e8ded85e86deeccadc5e98caeccad8e65e84c2e6caa0d2e6e8de
d8be86deeccadcbe98ec68bea0e4d2dac2e4f282e6e6cae800000000005a00000088caccc2ead8e8bebe84c2e6caa0d2e6e8ded8be86deeccadcbe98ec68bea0
e4d2dac2e4f282e6e6cae8be860000000000b6050abe050a00ba0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6d2c8cac2e4dae65e84c2e6
caa0d2e6e8ded85e86deeccadc5e86d0e4dedac2e65eec645e84c2e6caa0d2e6e8ded8be86deeccadcbeac64bea0e4d2dac2e4f282e6e6cae800000000005800
000088caccc2ead8e8bebe84c2e6caa0d2e6e8ded8be86deeccadcbeac64bea0e4d2dac2e4f282e6e6cae8be860000000000787d15e03184dc3e6ab79bbe9bd8
5c794f081c5db6f836450058a4fdac9fa5ea9980d295117e444d9dbcecc571ea8c7bdcfa53d99ffd6b91b49332ed747af44a2ec5ec9345eedbf23bcfc787
```
plaintext packet (5232 bits):
```
90957dce03000000013025e26d32c34f04000000be05c20500360000002f47616d652f45717569707061626c65732f47756e732f5369646561726d732f426173
65506973746f6c2f42617365506973746f6c00000000001600000044656661756c745f5f42617365506973746f6c5f430000000000cb0205cf0205004f000000
2f47616d652f45717569707061626c65732f47756e732f5369646561726d732f42617365506973746f6c2f436f76656e2f42617365506973746f6c5f436f7665
6e5f5072696d617279417373657400000000002900000044656661756c745f5f42617365506973746f6c5f436f76656e5f5072696d61727941737365745f4300
00000000d30205d70205005a0000002f47616d652f45717569707061626c65732f47756e732f5369646561726d732f42617365506973746f6c2f436f76656e2f
4c6576656c732f42617365506973746f6c5f436f76656e5f4c76345f5072696d617279417373657400000000002d00000044656661756c745f5f426173655069
73746f6c5f436f76656e5f4c76345f5072696d61727941737365745f430000000000db0205df0205005d0000002f47616d652f45717569707061626c65732f47
756e732f5369646561726d732f42617365506973746f6c2f436f76656e2f4368726f6d61732f76322f42617365506973746f6c5f436f76656e5f56325f507269
6d617279417373657400000000002c00000044656661756c745f5f42617365506973746f6c5f436f76656e5f56325f5072696d61727941737365745f43000000
0000bcbe0af018426e1fb5db4ddf4d6caebc27048e2e5b7c9b22002cd27ed6cf52f54c40e9ca083fa2a64e5ef6e23875c63d6efda9eccffeb548da4999763a3d
7a259762f6c922f76df99de7e3c3
```
UDP frame (668 bytes):
```
00000000180000008026290d20aa06b1c0347cade221a2a44bd4656751952a7c417a0bb1b078b46eea0205587869c6122657925963f053eb970b3ef0b951912a
c4ab87ee850bd7955c10555e703f81d8321f13c097d9e3e59a9d8b5f034e3b12fdb46616b4e2e99f8224be55fdc144fe23e84d62343d3b54bb7629d34ea0c608
8a19d288ceb73dd11b042ff5ff7e4af03c004ad7d4613e5a9a4781f1e98258c7afbff988599799500bb847aa3e1a0dc07af93d055dc1d6941dad823aee46d22a
379ae46a083c64ef820bc9444a56c03bcc096eb893abf508d33fe19dc2169df183bec899437189457e0fc1ccf030d52c042f5da4a6eda4443faf377082114c55
b36eaaf7e9f0e880c7795884e51ee5be81b79fb13953271c111f50c3514f3e90cfe8d1b66b579aab9b38b60bac906a08fd432bd1948ca08862bccd8c06b87604
a0aa8d4db9b7dcaaba3c35a048dda27b358c22be9acce4b222b4b97231eac26dcaf53ceddf5b2345b34e411d0bd1b8417f9b7e908122ec286097bc2fd5fc10c9
9fd8eba0b9126a2e2270fb3f231d4223fb1f56108a8d4b19827dcd4ac1a96b3242b44a53edc83db5d266f235c608fed9ca6bf70692df416ac3e578d23967060f
92f61ce315dea8b78819feb32e7b240cb95baf83e243ce1cfdd51f9e54cb2090c9af53bd25822743cecf9f34b7df79aeec79685cdf6c841d736c6778cf1cb4f7
09879a7ec2b046a811386cb73eca0588aeca8cd5b48948b637561cd497bc33656d018abc401de0d880398a5bbe9f07b3b0a82f9c70ac39009425540f748ca75c
3b266351931617bd3a01b350684fdb049ee24524fa368a28c3b7e44196a1061e33396bf94ec2edfec1e102d306ee44d27783e7b303052a32b6677ebafb65fc77
58c399aa9dbe11925ce715b04316bc544273ca2d76831d597d483e73
```

#### A22_slot1_contents_pistol_h1

- rep_plain (25 bits): `04780100`
- rep_encoded (25 bits, seed 10): `e4770f00`
- body: 62 bits

body:
```
01a2074f86fcee01
```
plaintext packet (190 bits):
```
909585ce03000000011021807232fb8000d18327437ef720
```
UDP frame (38 bytes):
```
000000001800000080f801f60faa06b1c0347caddea1c96ec1f425231055ebb905f65c8fe71c
```

#### A23_pistol_owner_h12

- rep_plain (25 bits): `30280000`
- rep_encoded (25 bits, seed 94): `5840a200`
- body: 35 bits

body:
```
cb60018902
```
plaintext packet (163 bits):
```
90958dce03000000011025006e328f8065b0804405
```
UDP frame (35 bytes):
```
000000001800000000ab7b2500aa06b1c0347c2de621e26ee1f4658a6eb35296f38903
```

#### A24_pistol_remote_role_h4

- rep_plain (20 bits): `100400`
- rep_encoded (20 bits, seed 94): `3e3509`
- body: 30 bits

body:
```
a3f8d424
```
plaintext packet (158 bits):
```
909595ce03000000011025206e327b80517c6a32
```
UDP frame (34 bytes):
```
0000000018000000c01815522caa06b1c0347cadee21e2eeccf4e598ae50f4a7c71c
```

#### A25_pistol_state_machine_open_guid96

- rep_encoded (9 bits, seed 94): `e401`
- body: 341 bits

body:
```
02000000800b78012e0000008ae2ead2e0e0c2c4d8caa6e8c2e8ca9ac2c6d0d2dcca0000000000022e411e
```
plaintext packet (469 bits):
```
90959dce03000000011025426e32570501000000c005bc001700000045717569707061626c6553746174654d616368696e6500000000000197201f
```
UDP frame (73 bytes):
```
0000000018000000c039bb2504aa06b1c0347c6dece1485fd0f425b76e95f9594dfa24b170481c6f99343cd8f03dee6dde85526416e48c7094a65ef4be0bec2b
ea9efb5d6ec5b5450e
```

#### A26_inactive_state_link_guid98

- body: 285 bits

body:
```
02000000880b78011c00000092dcc2c6e8d2eccaa6e8c2e8ca0000000000022ec3210705
```
plaintext packet (413 bits):
```
9095a5ce03000000011025626e32770401000000c405bc000e000000496e616374697665537461746500000000000197e1908312
```
UDP frame (66 bytes):
```
00000000180000000004bd900faa06b1c0347caddf21629ddaf4e5b76ed5cd594dfa24b130759825708234d8b087ecad67dbb9119d346115be476301a165dde1
7d0e
```

#### A27_auth_correct_idx1

- rep_plain (65 bits): `647869020000000000`
- rep_encoded (65 bits, seed 10): `616161087759cef401`
- body: 84 bits

body:
```
e1150c0b0b43b8cb72a60f
```
plaintext packet (212 bits):
```
9095adce03000000011021a072325381f00a86858521dc6539d30f
```
UDP frame (41 bytes):
```
00000000180000008013700a22aa06b1c0347cadf621e590e1f4a50250f8833f8820dfa4947aeb3307
```

#### A27_auth_correct_idx2

- rep_plain (65 bits): `647869040000000000`
- rep_encoded (65 bits, seed 10): `616461087759b9f401`
- body: 84 bits

body:
```
e1150c230b43b8cbcaa50f
```
plaintext packet (212 bits):
```
9095b5ce03000000011021c072325381f00a86918521dc65e5d20f
```
UDP frame (41 bytes):
```
0000000018000000c0c9bf761aaa06b1c0347cedc3a1e590e1f4e52e50f8833f8820dfacd4f5e93307
```

#### A27_auth_correct_idx3

- rep_plain (65 bits): `647869060000000000`
- rep_encoded (65 bits, seed 10): `61646108775949f401`
- body: 84 bits

body:
```
e1150c230b43b8cb4aa20f
```
plaintext packet (212 bits):
```
9095bdce03000000011021e072325381f00a86918521dc6525d10f
```
UDP frame (41 bytes):
```
000000001800000040a28b2714aa06b1c0347c6dd7a1e590e1f4a50e50f8833f8820dfacd452e23307
```

#### A27_auth_correct_idx4

- rep_plain (65 bits): `647869080000000000`
- rep_encoded (65 bits, seed 10): `616461087759d6f401`
- body: 84 bits

body:
```
e1150c230b43b8cbb2a60f
```
plaintext packet (212 bits):
```
9095c5ce030000000110210073325381f00a86918521dc6559d30f
```
UDP frame (41 bytes):
```
0000000018000000404f810533aa06b1c0347c6dc4a1e590e1f4650a50f8833f8820dfac5458eb3307
```

#### A27_auth_correct_idx5

- rep_plain (65 bits): `6478690a0000000000`
- rep_encoded (65 bits, seed 10): `61646108775950f401`
- body: 84 bits

body:
```
e1150c230b43b8cb82a20f
```
plaintext packet (212 bits):
```
9095cdce030000000110212073325381f00a86918521dc6541d10f
```
UDP frame (41 bytes):
```
0000000018000000c0c89ba907aa06b1c0347c2dfca1e590e1f4e51850f8833f8820dfacd467e23307
```

#### M_correction_move100

- rpc_plain (924 bits): `f9c1613b0364a01a2fdd24da5f0542355eba49cd3b063e894160e5c4a1040200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000`
- rpc_encoded (seed 10): `c13ee6c15c96f455ce8d153ee40c864e74966738252e5693d35612c9857c0deacbe62ca55f560e3e3dd46c1384d7deee8d33c9dfd8d3b93367ee6b77be72a22410f97c41d73650f1b94e3c058efa9ba9a5e98a284c1e0c82b4ed7fb068fc8824500690a369463c998c5555357e58dcdc4da8e401`
- body: 942 bits

body:
```
e63804fb98077359d257393756f89033183ad1599ee194b8584d4e5b492417f235a82f9bb3947e5939f8f450b34d105e7bbb37ce247f634fe7ce9cb9afddf9ca
899240e4f3055ddb40c5e73af11438ea6fa696a62ba230793008d2b6ffc1a2f123924019408ea619f164325655d5f861717337a19207
```
plaintext packet (1047 bits):
```
9095d5ce03000000010008701de63804fb98077359d257393756f89033183ad1599ee194b8584d4e5b492417f235a82f9bb3947e5939f8f450b34d105e7bbb37
ce247f634fe7ce9cb9afddf9ca899240e4f3055ddb40c5e73af11438ea6fa696a62ba230793008d2b6ffc1a2f123924019408ea619f164325655d5f861717337
a19247
```
UDP frame (145 bytes):
```
0000000018000000c022e0fb2baa06b1c0347cad0fa5775d59e4e5801b335196e161a5ac1bb8c35c7ab10c4ae6778de3c45e3221149e53a424429cb41a84c8f1
faecd3344b5df1c2d0b7415868a45f7a94a100d7f2f6d9edc86b8f453d5ab5488f92d5a3311947bbfe986590bae1af1ba3777ae14b9e4368d1a73005fc718c5d
65368c2eeb49f5021f28710b53526bac39
```

#### M_input_ack_move101

- rpc_plain (80 bits): `29e1ffffffff65000000`
- rpc_encoded (seed 10): `f5e061094d69e8f5ac03`
- body: 90 bits

body:
```
82d683872534a5a1d7b30e00
```
plaintext packet (195 bits):
```
9095ddce03000000010008d00282d683872534a5a1d7b30e04
```
UDP frame (39 bytes):
```
0000000018000000c0b7ddb30eaa06b1c0347cededc8078479e4a5acdbf2336d210307ce169a03
```

### V.4 Item-slot scale bodies, all 15 iterations

Create body = 479 bits for every slot (only the packed slot GUID varies, all < 128 so one byte). Link rep payload = `build_object_property_payload(3+i, 62+2i)`, encoded with seed 10.

| i | slot GUID | wire handle | create off | link off | create body (479 bits) | link body bits | link body | link rep_plain | link rep_encoded |
|---|---|---|---|---|---|---|---|---|---|
| 1 | 64 | 4 | +7 | +8 | `02000000f40afc0a00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000001200000092e8cadaa6d8dee8000000000002440f9e8432` | 44 | `e195c1847d00` | `10000100` | `98b00f00` |
| 2 | 66 | 5 | +9 | +10 | `02000000f40afc0a00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000001200000092e8cadaa6d8dee8000000000022440f9e8432` | 44 | `e195212e7f00` | `14080100` | `c4e50f00` |
| 3 | 68 | 6 | +11 | +12 | `02000000f40afc0a00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000001200000092e8cadaa6d8dee8000000000042440f9e8432` | 44 | `e19511d27b00` | `18100100` | `427a0f00` |
| 4 | 70 | 7 | +13 | +14 | `02000000f40afc0a00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000001200000092e8cadaa6d8dee8000000000062440f9e8432` | 44 | `e195597b7900` | `1c180100` | `6b2f0f00` |
| 5 | 72 | 8 | +15 | +16 | `02000000f40afc0a00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000001200000092e8cadaa6d8dee8000000000082440f9e8432` | 44 | `e19569f77b00` | `20200100` | `ed7e0f00` |
| 6 | 74 | 9 | +17 | +18 | `02000000f40afc0a00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000001200000092e8cadaa6d8dee80000000000a2440f9e8432` | 44 | `e19591707900` | `24280100` | `122e0f00` |
| 7 | 76 | 10 | +19 | +20 | `02000000f40afc0a00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000001200000092e8cadaa6d8dee80000000000c2440f9e8432` | 44 | `e19571577f00` | `28300100` | `eeea0f00` |
| 8 | 78 | 11 | +21 | +22 | `02000000f40afc0a00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000001200000092e8cadaa6d8dee80000000000e2440f9e8432` | 44 | `e195a9b47a00` | `2c380100` | `95560f00` |
| 9 | 80 | 12 | +23 | +24 | `02000000f40afc0a00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000001200000092e8cadaa6d8dee8000000000002450f9e8432` | 44 | `e195e17b7e00` | `30400100` | `7ccf0f00` |
| 10 | 82 | 13 | +25 | +26 | `02000000f40afc0a00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000001200000092e8cadaa6d8dee8000000000022450f9e8432` | 44 | `e19581f17d00` | `34480100` | `30be0f00` |
| 11 | 84 | 14 | +27 | +28 | `02000000f40afc0a00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000001200000092e8cadaa6d8dee8000000000042450f9e8432` | 44 | `e19539397d00` | `38500100` | `27a70f00` |
| 12 | 86 | 15 | +29 | +30 | `02000000f40afc0a00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000001200000092e8cadaa6d8dee8000000000062450f9e8432` | 44 | `e19531777e00` | `3c580100` | `e6ce0f00` |
| 13 | 88 | 16 | +31 | +32 | `02000000f40afc0a00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000001200000092e8cadaa6d8dee8000000000082450f9e8432` | 44 | `e195a13c7a00` | `40600100` | `94470f00` |
| 14 | 90 | 17 | +33 | +34 | `02000000f40afc0a00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000001200000092e8cadaa6d8dee80000000000a2450f9e8432` | 44 | `e195b9ca7a00` | `44680100` | `57590f00` |
| 15 | 92 | 18 | +35 | +36 | `02000000f40afc0a00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000001200000092e8cadaa6d8dee80000000000c2450f9e8432` | 44 | `e19579897a00` | `48700100` | `2f510f00` |

### V.5 Spawn-location quantized vector

`append_quantized_vector10((3581.634, -12019.322, 1294.153))` -> 61 bits `52f445fe54749401` (header 7 bits = 18|0x40 = 0x52, then 35816, -120193, 12942 as 18-bit two's complement).

### V.6 Cosmetic selection used for `A21_base_pistol_open_cosmetic`

Generated from scratch copies of `local_backend/skin_catalog.json` and `offline_skin_state.json` as of 2026-10-14 (player 7c9f41a4-65e2-4d81-a8d5-f879c3e5ab61). This vector is data-dependent: it changes whenever the saved Classic loadout changes.

```json
{
 "applied": true,
 "skin_id": "051f5a50-4b95-9452-f18a-a1add8266757",
 "level_id": "48d4142e-4538-0bb6-9fa1-57b36dd87b47",
 "chroma_id": "b36bfcf3-45a0-64c5-fa75-61bd9b20c107",
 "name": "BasePistol_Coven",
 "paths": {
  "skin": [
   "/Game/Equippables/Guns/Sidearms/BasePistol/Coven/BasePistol_Coven_PrimaryAsset",
   "Default__BasePistol_Coven_PrimaryAsset_C",
   true
  ],
  "level": [
   "/Game/Equippables/Guns/Sidearms/BasePistol/Coven/Levels/BasePistol_Coven_Lv4_PrimaryAsset",
   "Default__BasePistol_Coven_Lv4_PrimaryAsset_C",
   true
  ],
  "chroma": [
   "/Game/Equippables/Guns/Sidearms/BasePistol/Coven/Chromas/v2/BasePistol_Coven_V2_PrimaryAsset",
   "Default__BasePistol_Coven_V2_PrimaryAsset_C",
   true
  ]
 }
}
```

## Verifier corrections

**Method.** The verifier re-ran this area independently and offline. No responder, socket or live process was used, and no Python file was edited.

- **Dispatch re-run.** The source text of `ruh:2107-3352` was executed verbatim: dedented and wrapped in a one-shot loop so that `continue` stays legal. The run used the production flag values from section 1.1, a fake monotonic clock, a fake socket and a stubbed decoder. `inspect_live_packet_notify` was replaced by a stub that raises `OSError`. The client sent a 77-bit packet every 1/128 s after a 536-bit login and a 336-bit join. The skin JSON came from scratch copies; the state file hash was unchanged, so no migration ran.
- **Dispatch results.** With B = 879, the observed firing order, channels, open and exports flags, reliable offsets and body bit counts match the section 6 table exactly, for both the cosmetic and the plain A21. `take_ch9_sequence` was never called.
- **Observed times after the join ack** (s): A01 0.10, A02 2.13, A03 4.16, A04 6.20, A05 8.23, A06 10.26, A07 13.31, A08 16.35, A09 19.40, A10 22.45, A11 22.95, A12 23.46, A13 23.97, A14 24.48, A15 27.52, A16 28.54, A17 30.06, A18 32.09, A19 33.62, A20 34.63..48.85 (about 1.016 s apart), A21 50.88, A22 52.91, A23 53.93, A24 54.95, A25 56.47, A26 57.48, A27 59.52, 61.55, 63.58, 65.61, 67.64.
- **Vectors.** Every appendix V vector was regenerated through the real builders, capturing the pre-transform plaintext at `transform_short_packet`, and compared byte for byte with no mismatch. That covers V.1, the four V.2 frames, all 38 V.3 rows (body, rep_plain/rep_encoded, plaintext, frame, bit counts, frame length), the 15 V.4 rows and V.5.
- **Library equivalents.** The section 9.5 equivalents are confirmed, and A02 and A06 also equal their `build_dynamic_*` helpers.
- **Unit tests.** `python -B -m unittest test_range_udp_handshake test_range_server_bunch`: 85 tests, OK.

**Changes made to this document** (no wire value in sections 6-9 or appendix V was wrong):

1. **Section 1.2:** `inspect_packet_notify` (`ruh:2116-2126`) was listed as a dead path. It runs in production, inside the challenge arm, as a live ReadProcessMemory diagnostic with no wire effect. It now has its own entry, and its event keys are added to section 4.1.
2. **Section 1.2:** added the missing dead or unset switches:
   - `--move-ack-uniform-transform`, which would change every rep-payload encoding globally;
   - `VALORANT_RANGE_DROP_FLAGS`;
   - the non-cosmetic plain-open `else` branch `ruh:3008-3014`.
3. **Section 2.1:**
   - Fixed the dangling "see table 4" reference.
   - Documented that `hello_packets` is overwritten by every 232-bit packet, and that the welcome copies the latest one.
   - Added `latest_authenticated_packets` (heartbeat-only).
   - Added the per-datagram reset of `ack_history_mask` to 0 (`ruh:1488`).
4. **Section 3:** added the key-acquisition gate (`handler_state: "not_ready"` drop, `ruh:1465-1481`) and the silent drop of non-loopback sources. Also noted that `ruh:2135` is the only loop-level `continue`.
5. **Sections 4.1 and 4.3:** "crashes the handler" was imprecise. The ValueError escapes `run_probe` and ends the process. Added the join-ack `client_bit_count >= 128` check (`ruh:601`), and the facts that the challenge's ch0 sequence equals B and the welcome's equals B+1.
6. **Section 8.1:** made the `packaged == False` rule precise: only a level whose asset is the catalog's `level_asset` with `level_asset_packaged: false`. Expanded the failure behaviour: no output JSON is written, and keepalive's restart conditions apply. The restarted responder has an empty `pending`, so the existing connection gets no reply until a new stateless handshake.
7. **Section 9.4:** the stable-subobject shape implied that the content-block target is the exported GUID. For A26 the export is 98 and the target is the state machine 96.
8. **Section 10:** the outward part-B edges now list their extra prerequisites: `game_state_phase` for team_state_open, `player_unique_id` for pawn_owner, and the purchase-replacement and starting-equippable gates.
9. **Section 11:**
   - Tick-quantization shifts accumulate across stages; the measured drift is given.
   - Non-loopback datagrams and the `continue` path produce no JSON line.
   - Added the non-JSON stdout lines and the meaning of `elapsed_seconds`.
10. **Section 12, question 9:** the claim that the `channel_first_dynamic_offset` writes share the ch8/ch9 sequence space was wrong. Those writes only touch newly allocated channels (purchases 20+, bots 400+, projectiles 10000+). Part A meets the table only through the ch8 (+51) and ch9 (+5) counters, which start past part A's offsets.
11. **V.1:** marked the synthetic 232-bit login and join as builder inputs only. The live hello arm swallows every 232-bit packet.
