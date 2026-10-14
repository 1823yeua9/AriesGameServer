# 08 - Gameplay: abilities, gameplay tags, bots, projectiles, teams

Port spec for the Rust Range server. Sources (all in `local_backend/`, line numbers as of 2026-10-14): `range_abilities.py` (`ra`), `gameplay_tag_net_index.py` (`gtni`), `range_bots.py` (`rb`), `range_projectiles.py` (`rp`), `range_bomb_team_open.py` (`rbt`), the team builders in `range_server_bunch.py` (`rsb`), shared helpers in `range_equippables.py` (`re`), `range_gun_loader.py` (`rgl`), `range_weapon_cosmetics.py` (`rwc`), `read_equippable_state.py` (`res`), and the call sites in `range_udp_handshake.py` (`ruh`). Every number below is copied from code with its file:line. Test vectors (appendix V) were produced by importing the builders offline; `test_range_abilities`, `test_range_bots`, `test_range_projectiles`, `test_range_bomb_team_open` (24 tests) pass on the current tree.

Related docs: `03_bringup_a.md` (transport, framing, part A), `04_bringup_b.md` (stage chain B1-B22, including the team stages B3-B8 and the starting-equippable stage B18 that carries the ability tuning). This file specifies the builders and the per-packet systems ("part C" in 04) for abilities, bots and Cloudburst, plus the tag net index.

## 0. Production configuration for this area

| item | value | source |
|---|---|---|
| `ability_activation` | on | `launch_range.py:40` |
| `range_bots`, `range_bot_count` | on, 5 (argparse default, never passed) | `launch_range.py:40`; `ruh:4684` |
| `ability_projectiles`, `ability_projectile_stage` | on, "A" (default) | `launch_range.py:41`; `ruh:4687-4688` |
| `range_streaming_levels` | on (gates the bots) | `launch_range.py:40` |
| `pistol_state_mirror` | on (the mirror poll drives Cloudburst and shots) | `launch_range.py:39` |
| `command_queue` | on, but inert unless a JSON file is dropped into `runtime/responder_queue` (debug) | `launch_range.py:40`; `ruh:1951-1982` |
| `starting_equippables_probe` | on (B18; ability opens with tuning) | `launch.ps1:180-182` via `launch_range.py:279-283` |
| `team_state_open/array/link/player_probe`, `bomb_team_open/number_probe` | on; `bomb_team_number_value` 1, `bomb_team_number_bits` 8 | `launch.ps1:169-171`; `ruh` main defaults |
| `live_client_pid`, `live_client_image_base` | set | needed: every system here is inside the per-packet block gated on them (`ruh:1566-1568`) |

### 0.1 Dead paths in this area (skip in the port)

- Cloudburst stage B (`ability_projectile_stage == "B"`): `projectile_bending_body` (`rp:225-231`), `projectile_teleport_body` (`rp:234-241`), `projectile_stop_rpc_body` (`rp:244-250`), `ignore_pawn=True` open (`rp:206-211`), `forward=0.0` throw (`ruh:1036`), `stage_b_close_delay` 1.5 s (`rp:66`). Not passed by any launcher.
- `range_bomb_team_open.py`: draft helper, imported only by its test. Production writes the same bodies inline (`ruh:3548-3634`); frames are byte-identical (asserted in V.5).
- `build_ability_tuning_body(export=True)` and the `"bool"` tuning kind (`ra:131-132, 119-121`): no production row uses them.
- `build_bot_alive_body(bot, export=False)`: never called; death always sends the export form (`ruh:1010`).
- `range_settings` `bot_amount_index` (`range_client_rpc.py:220`): decoded, never applied; the bot count stays 5.
- `gtni.main()` (`gtni:109-119`): CLI diagnostic.
- `spawn_bot` queue command (`ruh:1958-1971`): debug only; spec in 4.7 in case the port keeps the queue.

## 1. Where these systems run

### 1.1 Per-packet block ("part C")

All per-packet systems below run inside the block at `ruh:1566-2078`, executed for every authenticated client datagram when `receive_key_override is not None and live_client_pid and live_client_image_base and source in client_restart_sent_sources`. `now = time.monotonic()` is taken once per datagram (`ruh:1569`). Order inside the block (matters for server packet sequence and for per-channel reliable order):

| # | system | lines | gate | sends on |
|---|---|---|---|---|
| 1 | equippable mirror poll; per ready equippable in list order: (a) `cloudburst_state` for `Ability_Wushu_4_Smoke_C`, (b) shot detection -> `bot_shots`, (c) resources, (d) extra machines, (e) auth state | 1600-1711 | `pistol_state_mirror`, reader ok, ready list non-empty, `now - equippable_mirror_polled >= 0.05` | projectile ch, bot/dummy ch, item ch |
| 2-10 | input correction, HUD, streaming, ult points, purchases, settings, command queue, hud refresh | 1713-2023 | (other docs) | |
| 11 | Cloudburst flight tick `tick_projectiles` | 2025-2030 | `ability_projectiles and source in projectile_fields and now - projectile_ticks >= 0.05` | projectile/zone ch |
| 12 | bot tick (dummy, retire, spawn) | 2032-2062 | `range_bots and source in range_streaming_sent_sources and now - bot_ticks >= 0.1` | bot/dummy ch |
| 13 | ability jobs | 2064-2078 | `ability_activation and source in melee_inactive_state_sent_sources and starting_equippable_index >= 7` | ch14-17 |

`*_ticks.get(source, 0)` defaults to 0, so the first eligible datagram always passes a time gate.

Mirror ready-list order (`mirror_ready_equippables`, `ruh:1316-1331`), each with its own gate: BasePistol (`src in inactive_state_link_sent_sources and src not in starting_pistol_replaced and auth_correct_rep_index[src] >= 5`), Melee (`src in melee_inactive_state_sent_sources`), then, only once `starting_equippable_index[src] >= 7`, for `starting_ability_resources()` order (smoke idx 2, Q idx 4, dash idx 3) the registered ability entry if present else the resource-only entry, then registered abilities not in that list (X idx 5), then purchases (`purchased_ready_equippables[src]`).

### 1.2 `send_reliable` (`ruh:1571-1585`)

```
send_reliable(channel, body, bits, has_exports, is_open=False, is_close=False):
  frame = build_server_bunch_frame(body, bits,
      BunchHeader(channel_index=channel, channel_name_index=102,
                  channel_sequence=take_channel_sequence(source, channel),
                  is_reliable=True, is_open, is_close, has_exports),
      server_sequence=next_server_sequences[source],
      acked_client_sequence=event["client_packet_sequence"],
      ack_history_mask=<this datagram's mask>, stateless_header=event["stateless_header"],
      receive_key=receive_key_override)
  sendto; next_server_sequences[source] = (+1) & 0x3FFF
```

- One bunch per datagram. Every send here is reliable. Close = `send_reliable(ch, b"", 0, False, is_close=True)`: empty body, bClose 1, then `SerializeInt(close_reason=0 Destroyed, 15)` = 4 zero bits (`rsb:1576-1577`).
- `take_channel_sequence` (`ruh:789-793`): `offset = channel_next_offset.get((source, ch), channel_first_dynamic_offset[ch])`; store `offset + 1`; return `(initial_actor_channel_sequences[source] + offset) & 0xFFF`. One counter per (connection, channel), shared by every sender on that channel.
- `channel_first_dynamic_offset` is a process-global dict (`ruh:786`): `{6: 11, 8: 51, 9: 5, 12: 2, 13: 5, 14: 2, 15: 2, 16: 2, 17: 2, 18: 2}`. Bots, the dummy and projectile/zone channels set `channel_first_dynamic_offset[ch] = 0` right before their open (`ruh:1038, 1085, 1965, 2042, 2056`). A missing key raises `KeyError`.
- The base B is the same for every channel of a connection, so a freshly opened dynamic channel's open bunch carries sequence `B`.

### 1.3 Cross-cutting state

Per connection (key = client `(ip, port)`):

| state | type | written | read |
|---|---|---|---|
| `channel_next_offset[(src, ch)]` | int | every `send_reliable` | same |
| `bot_fields[src]` | `BotField` (count, next_channel 400, next_guid 300100, bots[], rng `Random(1306)`, dummy) | bot tick / spawn_bot (setdefault) | bot tick, `bot_shots` |
| `bot_ticks[src]`, `bot_spawned_at[src]` | monotonic | bot tick | bot tick (0.1 s, 0.3 s gates) |
| `firing_seeds[(src, equippable.key)]` | u32 | mirror poll | shot detection |
| `firing_rng_components[(state_machine_addr, key)]` | address | `read_firing_seed` | cache (not per src) |
| `projectile_fields[src]` | `ProjectileField` (next_channel 10000, next_guid 610100, flights[], previous_smoke_state) | `cloudburst_state` (setdefault on every smoke poll) | flight tick |
| `projectile_ticks[src]` | monotonic | flight tick | flight tick (0.05 s) |
| `ability_allocators[src]` | `PurchaseAllocator(next_channel=0, next_guid=100000)` | job creation | job registration |
| `ability_jobs[src]` | list of 4 dicts `{load, stage, next, registered_tries}` | job creation | each packet |
| `ability_ready_equippables[src][class]` | `EquippableActor` | registration | mirror ready list |
| `equippable_mirror_states[(src, key)]`, `equippable_resource_states[(src, key, res)]` | mirror state | mirror, `send_extra_machines` | mirror |

Process-wide: `tag_net_index_cache["indices"]` (`ruh:935, 4051-4060`; set only on success, so a failed read is retried by the next ability index), `channel_first_dynamic_offset`, the module-global `random` used for the smoke-zone yaw (`ruh:1049, 1087`), and ONE `EquippableStateReader` in `pistol_mirror_reader["reader"]` (`ruh:885, 1591-1598`; created lazily on the first part-C datagram, shared by every connection; the B18 tag lookup opens and closes its own separate reader). That reader's caches `_names` (state-machine address -> {state name: address}, SM+0x160 order) and `_class_names` (object address -> class/object name) are filled on first read and never invalidated (`res:110-111, 147-157, 189-194`): a state machine first read while its state array is empty or invalid stays `{}` for the life of the process, which gives an ability registration with only `InactiveState` (`ability_ready` `[class, 1, ...]`). The Rust oracle should keep the same cache-once semantics for parity, or document the difference.

### 1.4 Channel and NetGUID map (this area)

| range | object | allocator | source |
|---|---|---|---|
| ch10, ch11; GUID 100, 102 (101/103 archetype/package) | BaseTeamState actors | fixed | `ruh:3420-3426`, `rsb:206-218` |
| GUID 104, 106 | "BlueTeam", "RedTeam" components (outer GameState 2) | fixed | `ruh:3482-3484` |
| GUID 108 | PlayerState "BombTeam" (outer 6) | fixed | `ruh:3548` |
| ch14-17; actors 208, 212, 216, 220 | abilities idx 2-5 (opened by B18) | fixed `12+i`, `200+4i` | `rsb:692-706` |
| GUID 190004, 190006, 190008 | "AbilityTuning" component of ability 2, 3, 4 | `190000 + 2i` | `ra:158-160` |
| GUID 100000+ (even) | ability state machines, InactiveStates, then registration state/extra-machine GUIDs | `ability_allocator()` per connection | `ra:25, 39-40` |
| GUID 310, 312, 314 | EquipmentCharge (smoke), EquipmentCharge (Q), SignatureAbilityResource (dash) | fixed | `re:272-274` |
| GUID 300001 / 300003 | `Default__TrainingBot_PC_C` / its package | fixed | `rb:41-43` |
| GUID 300005 / 300007 | `Default__TargetPracticeTrainingBot_C` / its package | fixed | `rb:87-89` |
| ch400+ (+1 per entity, never reused); GUID 300100+ stride 8 | dummy first, then bots: actor `g`, HealthDamageSection `g+2`, DamageHandlerComponent `g+4`, BombTeam `g+6` | `BotField` per connection | `rb:44-47, 136-146` |
| GUID 610001/610003, 610005/610007 | projectile / smoke-zone archetype and package | fixed | `rp:45-46` |
| ch10000+ (max 32766); GUID 610100+ step 2 | projectile actor, its FiniteSpeedMovement, zone actor | `ProjectileField` per connection | `rp:60-61, 89-107` |

Purchases use ch20+ / GUID 520+ (`rgl:38-41`). Nothing guards the ranges against each other; they only stay apart in practice (purchases would need 380 buys to reach ch400, bots about 38,750 spawns to reach GUID 610100).

## 2. Abilities

### 2.1 Constants (`ra`)

- `ability_indices = (2, 3, 4, 5)` (`ra:23`): Cloudburst C, Tailwind E, Updraft Q, Blade Storm X. Index = position in `wushu_starting_equippables` (`rsb:680-691`).
- `plan_ability_load(index, allocator)` (`ra:43-50`): `class_name = archetype.removeprefix("Default__")`, `channel = 12 + index`, `actor_guid = 200 + 4*index`, `slot` from the table, then `state_machine_guid = allocator.dynamic_guid()`, `inactive_guid = allocator.dynamic_guid()`. Index 0/1/6 raise `ValueError`.
- `PurchaseAllocator.dynamic_guid()` (`rgl:56-62`): bump to even, return, `+2`.

| index | class | ch | actor | slot | SM GUID | Inactive GUID |
|---|---|---|---|---|---|---|
| 2 | Ability_Wushu_4_Smoke_C | 14 | 208 | 3 | 100000 | 100002 |
| 3 | Ability_Wushu_E_Dash_C | 15 | 212 | 5 | 100004 | 100006 |
| 4 | Ability_Wushu_Q_CycloneBoost_C | 16 | 216 | 4 | 100008 | 100010 |
| 5 | Ability_Wushu_X_Dagger_Production_C | 17 | 220 | 9 | 100012 | 100014 |

All four loads are planned in one list comprehension at job creation, so these GUIDs are fixed. The allocator then continues at 100016 for registration.

### 2.2 Open with AbilityTuning (B18 hook, `ruh:4045-4068`)

At B18 (see `04_bringup_b.md` B18) for `equip_i in starting_ability_tunings` (= {2, 3, 4}) and `ability_activation`:

1. If `tag_net_index_cache` has no `"indices"`: open a new `EquippableStateReader`, `predict_net_indices(handle, image_base, key)` (section 3), close the reader, cache the `TagNetIndex` for the process.
2. If `first_bit_segment < true_bit_num`: raise (segmented encoding unsupported).
3. `open_body = build_ability_open_with_tuning(equip_i, indices, true_bit_num)`. Event `ability_open_tuning = [equip_i, true_bit_num, constructed]` (live: `[2|3|4, 10, true]`).
4. Any exception: keep the default open (`build_starting_equippable_open`), event `ability_open_tuning_error`. Tailwind then never readies (no PrimeDuration).

Rows (`ra:148-155`; must be in the open bunch because the Blueprint reads them at BeginPlay):

| i | tag | kind | value |
|---|---|---|---|
| 2 | `AbilityTuning.Wushu.4.SmokeDuration` | float | 2.5 |
| 2 | `AbilityTuning.Wushu.4.SmokeMaxCharges` | int | 2 |
| 3 | `AbilityTuning.Wushu.E.DashVelocityMultiplier` | float | 1.0 |
| 3 | `AbilityTuning.Wushu.E.PrimeDuration` | float | 7.5 |
| 3 | `AbilityTuning.Wushu.E.MaxCharges` | int | 1 |
| 4 | `AbilityTuning.Wushu.Q.MaxCharges` | int | 1 |

`build_ability_open_with_tuning(index, tag_indices, tag_bits)` (`ra:163-201`), body (`[w: v]` = v in w bits LSB-first, `[p: v]` = packed uint, `FS(s)` = u32 len+1, ASCII, NUL):

```
[1: 0][32: 2]                                           NetGUID exports, count 2
[p: actor+1][8: 5][p: actor+3][8: 5][p: 0] FS(package) [32: 0] FS(archetype) [32: 0]   rwc:133-144
[p: 190000+2i][8: 5][p: actor][8: 0] FS("AbilityTuning") [32: 0]     outer = actor by GUID only
[p: actor][p: actor+1][p: 5][4: 0]                      dynamic actor header, level 5, default transform
tuning block (2.3, export=False)
guard: body bits < 8192 else ValueError
```

### 2.3 AbilityTuning block (`build_ability_tuning_body`, `ra:96-140`)

UAbilityTuningComponent ("AbilityTuning") FRepLayout: 1 bReplicates, 2 bIsActive, 3 ReplicatedAbilityTunings (TArray<FAbilityTuningRuntimeEntry>, 0x28 bytes). Leaves per element: 1 Tag (net index), 2 Type (EAbilityTuningValueType, 3 bits: bool 0, int 1, float 2, class 3), 3 BoolValue, 4 IntValue, 5 FloatValue, 6 ClassValue (`ra:77-86`).

```
rep (plain, seed = actor GUID):
  [1: 0][p: 3][16: n]                            n = 1..32
  per element e (0-based), base = 6e:
    [p: base+1][tag_bits: tag net index]         0 <= index < 2^tag_bits
    [p: base+2][3: type]
    bool : [p: base+3][1: value]
    int  : [p: base+4][32: value & 0xFFFFFFFF]
    float: [p: base+5][32: IEEE f32 bits]        struct.pack("<f")
  [p: 0]   end of element handles
  [p: 0]   end of properties
block:
  (export=True only: [1: 0][32: 1][p: comp][8: 5][p: actor][8: 0] FS(name) [32: 0])
  [1: 1 bHasRepLayout][1: 0 bIsActor][p: comp GUID][1: 1 bStablyNamed][p: rep bits][enc(actor)]
```

Validation: `0 < n <= 32`, `1 <= tag_bits <= 16`, kind in bool/int/float, index in range; else `ValueError`. Element handles are 8-bit packed while `6e+6 < 128` (n <= 21). Bits per float/int element with 10-bit tags: 8+10+8+3+8+32 = 69. Production rep sizes: i=2 179, i=3 248, i=4 110 bits; bodies 1443 / 1496 / 1478 bits (V.1).

### 2.4 Activation job (`ruh:2064-2078`, `advance_ability_job` `ruh:1130-1170`)

Creation (first packet where the gate in 1.1 #13 holds): `allocator = ability_allocators.setdefault(src, ability_allocator())`; `jobs = [{load: plan_ability_load(i, allocator), stage: 0, next: now + 0.25*order, registered_tries: 0} for order, i in enumerate((2, 3, 4, 5))]`. Then, every packet, for each job in list order with `stage < 7 and now >= next`:

| stage | action | next |
|---|---|---|
| 0 | `send_reliable(ch, *ability_role_body(load), False)` | now + 0.4 |
| 1 | `send_reliable(ch, *ability_state_machine_open_body(load), True)` | now + 0.4 |
| 2 | `send_reliable(ch, *ability_initial_state_body(load), True)` | now + 0.4 |
| 3 | register (below) | done (7) or now + 0.3 |

Job 0 fires its stage 0 in the creation packet. Registration: `machine = read_slot_machines(reader, [slot])[slot]` (oracle). If missing or `machine.class_name != load.class_name`: `registered_tries += 1`; if > 20: stage 7, event `ability_register_gave_up`; else retry in 0.3 s. Otherwise, in this exact order:

1. `extra = plan_extra_machines(reader, machine, allocator)` (`ruh:1102-1118`): for each machine in `read_item_machines(item)` (item+0xE30 array, count +0xE38, 1..8) whose address != the main machine and which has a starting state (machine+0x1E8): `ExtraStateMachine(name, guid=allocator.dynamic_guid(), starting_state, state_guids={s: allocator.dynamic_guid() for s in machine state names})` (machine GUID before its state GUIDs; states in SM+0x160 array order).
2. `names = [n for n in state names of the main SM (SM+0x160 order) if n != "InactiveState"]`; state GUIDs `{n: allocator.dynamic_guid()}` in that order.
3. `resources` = the matching `starting_ability_resources()` entry's resources (smoke/Q/dash; X has none).
4. `equippable = ability_equippable(load, state_guids, resources, extra)` (`ra:62-70`): key = class name (same key as the resource-only entry, so charge mirror state carries over), `state_guids` = `{"InactiveState": inactive_guid} + names`, `mirror_states=True`.
5. `send_extra_machines` (`ruh:1120-1128`): per extra machine, `send_reliable(ch, build_machine_open_body(actor, extra.guid, extra.name), True)` then `send_reliable(ch, build_machine_state_body(equippable, extra, extra.starting_state, True), True)`; mirror state `(src, "<key>/<name>")` = `MirrorState(auth=starting, exported={starting}, active=True)`.
6. `ability_ready_equippables[src][key] = equippable`; event `ability_ready += [class, len(names)+1, [extra names]]`; stage 7.

Live (`responder_24252` log): smoke 11 states / no extra; dash 15 / `StateMachine_Priming`; Q 12 / `BoostStateMachine`; X 35 / none. The registration GUIDs therefore depend on oracle data (state-name order) and on the order in which jobs reach stage 3; they cannot be precomputed. From then on the generic mirror (other doc) confirms the ability's predicted states.

### 2.5 Setup body layouts

```
role (ra:53-55 -> rgl:194-203), 30 bits:
  [1: 1][1: 1][p: 20][enc(actor) of [1: 0][p: 4][3: 2][p: 0]]        RemoteRole = AutonomousProxy (3-bit ENetRole)
state machine open (ra:58-59 -> re:119-134), 381 bits:
  [1: 0][32: 1][p: SM][8: 5][p: actor][8: 0] FS("EquippableStateMachine") [32: 0]
  [1: 1][1: 0][p: SM][1: 1][p: 9][enc(actor) of [1: 0][p: 0]]
InactiveState (ra:73-74 -> re:88-116), 341 bits:
  [1: 0][32: 1][p: Inactive][8: 5][p: actor][8: 0] FS("InactiveState") [32: 0]
  [1: 1][1: 0][p: SM][1: 1][p: 41][enc(actor) of [1: 0][p: 3][p: Inactive][p: 0]]
```

Channel sequences: ability channels start their dynamic counter at +2 (`ruh:786`). The resource mirror (EquipmentCharge seeds on ch14/16, SignatureAbilityResource on ch15) becomes active as soon as `starting_equippable_index >= 7`, which is before the knife chain finishes, so in practice the first resource send takes +2 and the role body takes a later offset. Never hard-code +2/+3/+4: always draw from the shared per-(connection, channel) counter. ch17 (X) has no resources, so its role normally takes +2. The charge resources themselves (GUIDs 310/312/314, seeded from MaxCharges at component +0x188, `refill_after = 4.0` s back to full; `re:252-275`) are specified in `05_authority_loop.md` and `07_gameplay_guns.md` 5.3, not here; they also draw from these same ch14-16 counters.

## 3. Gameplay-tag net index (`gtni`)

### 3.1 What goes on the wire

FastReplication is on, so an FGameplayTag is its net index (IDA `sub_7FF7389AA370` -> `SerializeTagNetIndexPacked` `sub_7FF7389A7EE0`, `gtni:1-12`). With `FirstBitSegment >= TrueBitNum` it is a plain `TrueBitNum`-bit field, LSB-first (the only case the port supports; `ruh:4061-4062` raises otherwise).

- `InvalidTagNetIndex = N + 1`, `TrueBitNum = ceil(log2(N + 1))` (`gtni:97`), `FirstBitSegment = min(TrueBitNum, settings.NetIndexFirstBitSegment)` (`gtni:98`).
- Live 13.06: N = 898 tags, NetIndexFirstBitSegment = 16, so TrueBitNum = 10 and FirstBitSegment = 10 (session notes; live event `ability_open_tuning = [i, 10, true]`).

### 3.2 Index construction (client `ConstructNetIndex` `sub_7FF7389B0590`)

1. Take every value of `GameplayTagNodeMap` (manager+0x130).
2. Sort by complete tag name: FName compare = same comparison index -> compare number; else case-insensitive text. Python: `sort(key=(name.lower(), number))` (`gtni:87`). Lowercase ASCII compare, so `_` (0x5F) sorts before letters and `.` (0x2E) before both.
3. For each `CommonlyReplicatedTags` entry k (manager+0x180, 12-byte FGameplayTag, FName comparison index at +0) in order: find it in the list and swap it with position k (`gtni:89-96`). Missing entries are skipped.
4. Index = position.

If the client has already built its index (first sorted node's `NetIndex` at node+0x50 != 0xFFFF, `gtni:99-100`), Python ignores the computed order and reads each node's u16 `NetIndex` at +0x50 (`gtni:101-105`). Live: `constructed = true`, so production uses the client's own values.

### 3.3 Memory reads (oracle requirement)

```
GObjects scan indices 0xEA40..0xFAFF (gtni:48): object name FName at +0x18, class at +0x10 -> class FName at +0x18
  GameplayTagsSettings, name starts "Default__"  -> settings
  GameplayTagsManager, name not "Default__"      -> manager
manager+0x130: TMap data (u64), count (u32 +8), capacity (u32 +12); element stride 0x28:
  +0x00 FName comparison index (u32), +0x04 number (u32), +0x10 node pointer (u64; 0 = free slot)
  require (non-null nodes decoded) == count
manager+0x180: CommonlyReplicatedTags data (u64), count (u32 +8); stride 12, comparison index at +0
settings+0xBC: NetIndexFirstBitSegment (i32)
node+0x50: NetIndex (u16), 0xFFFF until the client constructs its index
manager+0x28C: name-index-invalidated flag (declared, unused by the code)
```

FName text comes from `read_fname_pool.decode_name` with the derived name key (oracle doc).

### 3.4 What the Rust server needs

Only six indices (section 2.2 tags) and `TrueBitNum`. The full 898-tag list exists only in client memory (node map). The count 898 and NetIndexFirstBitSegment 16 come from session notes (live reads), not from any file in the repo. Candidate on-disk sources listed in `game req/VALORANT_GAME/VALORANT/live/Manifest_UFSFiles_Win64.txt` are `ShooterGame/Config/DefaultGameplayTags.ini` (line 4368), `ShooterGame/Config/Tags/CosmeticGameplayTags.ini` (4380, the only file under `Config/Tags/`) and 25 `*_GameplayTags.uasset` DataTables (e.g. `Characters/Wushu/Wushu_GameplayTags`, 10138); that these produce exactly the client's 898 node-map entries is unverified, and they are inside the paks, not extracted. No copy of the list is in the repo and no log records the six values (04 open question 1). Options:

1. Oracle message: read node+0x50 for the six names (plus N and settings+0xBC). Same result as Python's constructed path. Recommended.
2. Pinned table in `aries-server.toml`, captured once by the user with `python gameplay_tag_net_index.py <pid> <image_base> AbilityTuning.Wushu` (prints `tags=N true_bits first_segment constructed` and each matching index). The client is pinned at 13.06, so the values should be stable, but this is unverified.

Keep the Python fallback order: on any failure, send the default open and log `ability_open_tuning_error`.

## 4. Bots (`rb`)

### 4.1 Constants

| name | value | line |
|---|---|---|
| bot archetype | `("Default__TrainingBot_PC_C", "/Game/Characters/TrainingBot/TrainingBot_PC")`, GUID 300001 / package 300003 | 41-43 |
| first channel / GUID / stride | 400 / 300100 / 8 | 44-47 |
| BombTeam component | name `"BombTeam"`, Team handle 3, 8 bits (FEnumProperty EAresTeam, IDA `sub_7FF7368FA3B0`) | 48-52 |
| teams | `player_team = 1`, `red_team = 0`; bot team = `red_team if player_team != red_team else 1` = 0 | 53-54, 146 |
| `bot_max_life` | 100.0 | 55 |
| `manager_origin` | (4825.0, -13600.0) | 59 |
| `bot_center_z` | 1253.8 + 98.0 = 1351.8 | 60 |
| head | sphere centre z+75.0, radius 22.0 | 66 |
| body | cylinder radius 28.0, z in [-22.0, +53.0] | 67 |
| legs | cylinder radius 22.0, z in [-98.0, -22.0] | 68 |
| dummy archetype | `("Default__TargetPracticeTrainingBot_C", "/Game/Heroes/Decoy/TargetPracticeTrainingBot")`, GUID 300005 / package 300007 | 87-89 |
| dummy transform | (6190.0, -11640.0, 1270.0), yaw 180.0 | 90-91 |
| `dummy_max_life` / `dummy_mesh_lift` | 1000.0 / 4.0 | 92-93 |
| UpdateDamageText | field 57, `SerializeInt(57, 58 + 1)` | 96-97 |
| damage sections | HealthDamageSection handles 3 bAlive (1 bit), 4 Life (f32), 5 MaximumLife (f32); DamageHandlerComponent handle 3 bAlive | `re:156`, `rb:280` |

Damage table `weapon_damage` (head, body, legs), `rb:71-83`; default `(100, 35, 30)` for any other class (`rb:83`, `shot_damage` `rb:341-343` uses `class_name or ""`):

| class | head | body | legs |
|---|---|---|---|
| BasePistol_C | 78 | 26 | 22 |
| SawedOffShotgun_C | 36 | 12 | 10 |
| AutomaticPistol_C | 78 | 26 | 22 |
| LugerPistol_C | 105 | 30 | 25 |
| CompactPistol_C | 160 | 40 | 34 |
| RevolverPistol_C | 159 | 55 | 46 |
| Vector_C | 67 | 27 | 22 |
| SubMachineGun_MP5_C | 78 | 26 | 22 |
| PumpShotgun_C | 40 | 20 | 17 |
| AutomaticShotgun_C | 34 | 17 | 14 |
| AssaultRifle_Burst_C | 116 | 35 | 29 |
| DMR_C | 195 | 65 | 48 |
| AssaultRifle_ACR_C | 156 | 39 | 33 |
| AssaultRifle_AK_C | 160 | 40 | 34 |
| BattleRifle_C | 176 | 44 | 37 |
| LeverSniperRifle_C | 202 | 101 | 85 |
| DS_Gun_C | 238 | 140 | 119 |
| BoltSniper_C | 255 | 150 | 127 |
| LightMachineGun_C | 72 | 30 | 25 |
| HeavyMachineGun_C | 95 | 38 | 32 |

### 4.2 Allocation and placement (`BotField`, `rb:117-157`)

`Bot` (`rb:100-114`) defaults: `life 100.0` (f64), `alive True`, `opened False`, `died_at None`, `team 0`. `BotField` defaults: `count 5`, `next_channel 400`, `next_guid 300100`, `bots []`, `rng Random(1306)`, `dummy None`.

```
allocate(index):
  channel = next_channel; next_channel += 1
  g = next_guid; actor, health, handler, team = g, g+2, g+4, g+6; next_guid += 8
  location = place()
  yaw = degrees(atan2(-13600.0 - y, 4825.0 - x))          face the manager
  Bot(index, channel, actor, health, location, handler, yaw, team_guid=team, team=0)
allocate_dummy():
  dummy = allocate(-1)          consumes a channel, 8 GUIDs AND place()'s RNG draws
  dummy.location, dummy.yaw, dummy.life = (6190.0, -11640.0, 1270.0), 180.0, 1000.0
  self.dummy = dummy            never in bots[]
place():
  taken = [b.location for b in bots if b.alive]           dead-but-not-retired bots do not block
  repeat 64 times:
    angle = radians(rng.uniform(-30.0, 30.0))
    distance = rng.uniform(700.0, 1800.0)
    x = 4825.0 + distance * sin(angle)
    y = -13600.0 - distance * cos(angle)
    if all(hypot(x - ox, y - oy) >= 200.0 for taken): return (x, y, 1351.8)
  return (4825.0, -14600.0, 1351.8)
```

RNG decision: **reproduce it.** `rng` is `random.Random(1306)` per `BotField` (per connection). Live logs match the offline sequence exactly (dummy then bots on ch401-405 at `(4463, -14408)`, `(4573, -14792)`, `(4795, -14629)`, `(5187, -14270)`, `(4754, -14342)`, `responder_20928` log; V.3). Bot positions feed the open bunch bits, so a different RNG breaks byte parity and shadow diffs. Implement:

- MT19937, seeded by `init_by_array([1306])` (CPython `random_seed`: the absolute value of an int seed split into 32-bit little-endian words; 1306 -> key `[1306]`).
- `random() = ((u32 >> 5) * 67108864.0 + (u32' >> 6)) * (1.0 / 9007199254740992.0)` using two consecutive outputs.
- `uniform(a, b) = a + (b - a) * random()`.
- `radians(x) = x * (PI / 180.0)`, `degrees(x) = x * (180.0 / PI)` (CPython and Rust std use the same single multiply).
- `sin`, `cos`, `atan2`: CPython calls the MSVC CRT; Rust `f64::sin/cos/atan2` on `x86_64-pc-windows-msvc` call the same CRT functions. The V.3 `repr` values are the check.
- `hypot`: CPython 3.12 has its own correctly-rounded-ish algorithm; Rust's calls the CRT. They can differ by 1 ulp, which only matters at exactly 200.0. Use `(dx*dx + dy*dy).sqrt()` or the CRT; document the choice.

Each placement attempt draws 2 values; a rejected attempt draws 2 more (bot 2 in V.3 took 4). The draw sequence depends on which bots are alive at each spawn, so respawn positions depend on kill order.

The smoke-zone yaw (section 5) uses the process-global unseeded `random` and cannot be reproduced; that one may be replaced.

### 4.3 Bot tick (`ruh:2032-2062`, every >= 0.1 s once Range streaming was sent)

```
field = bot_fields.setdefault(src, BotField(count=5))
if field.dummy is None:
    dummy = field.allocate_dummy(); first_dynamic_offset[dummy.ch] = 0
    send_reliable(dummy.ch, build_dummy_open(dummy), exports=True, is_open=True)      +0
    send_reliable(dummy.ch, build_dummy_health_body(dummy), exports=True)            +1
    dummy.opened = True; event dummy_spawned = [ch, actor]
for bot in list(field.bots):
    if not bot.alive and bot.died_at is not None and now - bot.died_at >= 1.5:
        send_reliable(bot.ch, b"", 0, False, is_close=True); field.bots.remove(bot); event bot_retired += [actor]
if count(alive bots) < field.count and now - bot_spawned_at.get(src, 0) >= 0.3:
    bot = field.allocate(len(field.bots)); field.bots.append(bot); first_dynamic_offset[bot.ch] = 0
    send_reliable(bot.ch, build_bot_open(bot), True, is_open=True)                     +0
    send_reliable(bot.ch, build_bot_health_body(bot, export=True), True)               +1
    bot.opened = True; bot_spawned_at[src] = now; event bot_spawned += [[ch, actor, rounded location]]
```

At most one bot spawns per tick, so the field fills at one bot per >= 0.3 s. The first bot spawns in the same tick as the dummy. A dying bot is replaced while its corpse waits the 1.5 s for retirement (live: ch406 opened before the first `bot_retired`).

### 4.4 Shots (`ruh:1622-1632`, `bot_shots` `ruh:969-1012`)

Trigger, per mirror poll (>= 0.05 s), per ready equippable with `range_bots and machine.is_current and equippable.resources`:

- `seed = read_firing_seed(...)` (`ruh:946-964`): the item machine's state named `"FiringState"` (else the first state whose name starts with `"Firing"`), u64 at +0x4A0 = RNG component (cached per (machine, key)), u32 at +0xEC = predicted networked random seed. No firing state or read failure -> None.
- If `seed is not None`: store it; if a previous seed exists and differs, call `bot_shots(src, class_name, shots=1)`. One shot per change per poll, however many rounds were fired in between.

`bot_shots` (returns at once if `bot_fields` has no entry yet, i.e. before the first bot tick, or the reader is missing; a `None` camera returns silently, a read exception logs `bot_camera_error`):

```
camera = read_camera(): PlayerCameraManager = controller+0x528; 6 doubles at PCM+0x17C0:
         location (x, y, z), rotation (pitch, yaw, roll) degrees            res:66-74
direction = view_ray(pitch, yaw) = (cos p cos y, cos p sin y, sin p)        rb:289-291
hit = first_hit(bots, origin, direction)
on_dummy = dummy_hit(dummy, origin, direction)      None unless dummy.opened; centre = dummy.location + (0, 0, 4.0)
if on_dummy and (hit is None or on_dummy.distance < hit.distance):          strict <
    damage = shot_damage(class, region)
    send_reliable(dummy.ch, build_update_damage_text_body(dummy, float(damage)), False)
    event dummy_hits += [region, damage, round(distance)]
elif hit is None: event bot_shot_missed += 1
else:
    bot.life -= damage                                   python float
    if bot.life <= 0: bot.life, bot.alive, bot.died_at = 0.0, False, now
    send_reliable(bot.ch, build_bot_health_body(bot, export=False), False)
    if not bot.alive: send_reliable(bot.ch, build_bot_alive_body(bot, export=True), True)
    event bot_hits += [actor, region, damage, round(life), round(distance)]
```

`round()` in events is Python half-even.

Hit geometry (`rb:294-355`), all f64, direction assumed unit:

- Sphere: `oc = o - c`, `b = oc.d`, `c' = |oc|^2 - r^2`, `disc = b^2 - c'`; `disc < 0` -> miss; `t = -b - sqrt(disc)`; hit only if `t > 0` (an origin inside the sphere misses).
  - Parity trap: `b` and `|oc|^2` are built with Python's builtin `sum(...)` (`rb:296-297`). The responder runs on CPython 3.12 (`python --version` = 3.12.3), where `sum()` over floats uses Neumaier compensated summation, not plain left-to-right addition (offline check: `sum([0.1, 0.2, 0.3]) = 0.6` but `0.1 + 0.2 + 0.3 = 0.6000000000000001`). Rust must reproduce it: `s = x0` (the int start 0 plus the first float), `comp = 0.0`; for each next `x`: `t = s + x; comp += if s.abs() >= x.abs() { (s - t) + x } else { (x - t) + s }; s = t`; at the end `if comp != 0.0 && comp.is_finite() { s += comp }`. Then `c' = sum - r*r`. The cylinder terms below are plain `+` (no `sum`).
- Vertical cylinder about the centre axis: `a = dx*dx + dy*dy`; `a < 1e-9` -> miss (vertical rays never hit body or legs); `b = ox*dx + oy*dy`, `c' = ox*ox + oy*oy - r*r`, `disc = b*b - a*c'`; `disc < 0` -> miss; for `t` in `((-b - sqrt(disc))/a, (-b + sqrt(disc))/a)` in that order: skip `t <= 0`; return the first `t` with `cz + low <= oz + dz*t <= cz + high` (`rb:305-323`). No end caps.
- `hit_region`: evaluate head, body, legs; return the smallest t (`min` keeps the first on a tie: head, then body, then legs).
- `first_hit`: bots in list order, skip `not alive or not opened`, replace only on strictly smaller distance.

### 4.5 Body layouts

Bot open (`build_bot_open`, `rb:160-195`); requires even `team_guid != 0` and `team != player_team`:

```
[1: 0][32: 2]
[p: 300001][8: 5][p: 300003][8: 5][p: 0] FS("/Game/Characters/TrainingBot/TrainingBot_PC") [32: 0] FS("Default__TrainingBot_PC_C") [32: 0]
[p: team_guid][8: 5][p: actor][8: 0] FS("BombTeam") [32: 0]
append_dynamic_actor_header(actor, 300001, 5, location, yaw)            rsb:171-195:
  [p: actor][p: 300001][p: 5][1: 1 position][1: 1 quantized][quantized10(location)]
  yaw_short = round(yaw * 65536 / 360) & 0xFFFF       Python round = half-even
  yaw_short != 0: [1: 1][1: 0 pitch][1: 1][16: yaw_short][1: 0 roll][2: 0 scale, velocity]
  yaw_short == 0: [3: 0]
[1: 1][1: 0][p: team_guid][1: 1][p: 25][enc(actor) of [1: 0][p: 3][8: team][p: 0]]
guard < 8192 bits
```

`quantized10` (`rsb:78-96`): components `floor(v*10 + 0.5)` if `v >= 0` else `ceil(v*10 - 0.5)`; width = smallest w in 1..63 with every component in `[-2^(w-1), 2^(w-1))`; `[7: w | 0x40]` then each component as w-bit two's complement. Bot open is 1197 bits for every bot whose quantized coordinates need width 19 and whose yaw short is non-zero (all V.3 bots); yaw short 0 gives 1178 bits (`test_range_bots.py:83-89`, the `[3: 0]` branch).

Damage section (`build_damage_section_body`, `rb:237-261`; `0 <= life <= maximum <= 1000` else `ValueError`):

```
(export: [1: 0][32: 1][p: section][8: 5][p: actor][8: 0] FS("HealthDamageSection") [32: 0])
[1: 1][1: 0][p: section][1: 1][p: 98][enc(actor) of
  [1: 0][p: 3][1: alive][p: 4][32: f32(life)][p: 5][32: f32(maximum)][p: 0]]
bot:   build_bot_health_body(bot, export) = section(actor, actor+2, max(0, life), 100.0, alive)   rb:264-266
dummy: build_dummy_health_body = section(actor, actor+2, 1000.0, 1000.0, True, export=True)       rb:205-207
```

Same shape as the pawn's ShieldDamageSection (`re:160-184`; equality asserted by `test_range_bots.py:62-66`).

Alive (`build_bot_alive_body`, `rb:269-286`):

```
(export: [1: 0][32: 1][p: actor+4][8: 5][p: actor][8: 0] FS("DamageHandlerComponent") [32: 0])
[1: 1][1: 0][p: actor+4][1: 1][p: 18][enc(actor) of [1: 0][p: 3][1: alive][p: 0]]
```

Dummy open (`rb:198-202`): `build_dynamic_actor_reference(dummy archetype, actor, 300005, 300007, level 5, (6190.0, -11640.0, 1270.0), 180.0)`, 1030 bits.

UpdateDamageText (`rb:210-226`), multicast RPC on the dummy channel, sent reliable:

```
payload (79 bits): [SerializeInt(57, 59) = 6 bits: 1,0,0,1,1,1][p: 65][1: 1 presence][64: f64 bits of damage]
body (89 bits):    [1: 0 bHasRepLayout][1: 1 bIsActor][p: 79][enc(dummy actor)]
```

The client adds `floor(Damage)` to its tally, shows `"{tally}\r\n({hits})"` and clears it 2.0 s after the last hit (`rb:23-28`).

### 4.6 Bot channel sequence per bot

`+0` open, `+1` health (export), `+2..` one health update per hit, on death `health` then `alive` (export), then `close` at retirement. The dummy: `+0` open, `+1` health, then one UpdateDamageText per hit.

### 4.7 Debug `spawn_bot` (`ruh:1958-1971`)

A queue file `{"spawn_bot": [x, y, z]}`: `field.allocate(len(bots))` (consumes RNG draws and a channel/GUIDs), overwrite `location` with the given floats (yaw stays the one computed from the random location), append, `first_dynamic_offset[ch] = 0`, send open (+0, open, exports) and health (+1, exports), `opened = True`, event `bot_spawned`. It does not touch `bot_spawned_at`. Optional for the port.

## 5. Cloudburst projectile, stage A (`rp`)

Live status: not yet run live. The `responder_20928` log (the bots/dummy session) has no `cloudburst_*` event. The bit layouts below are offline-tested only (`test_range_projectiles.py`).

### 5.1 Trigger (`cloudburst_state`, `ruh:1016-1044`)

Called from the mirror poll for every ready equippable with class `Ability_Wushu_4_Smoke_C` whose slot machine read matches (resource-only entry or registered ability):

```
pf = projectile_fields.setdefault(src, ProjectileField())
previous, pf.previous_smoke_state = pf.previous_smoke_state, machine.predicted
if not (predicted in {"ProjectileThrowState", "CommitTime", "AirBendingState"}
        and previous not in that set): return                                   rp:39, 110-112
camera = read_camera() (None or error -> return; event cloudburst_camera_error)
location, pitch, yaw, velocity = throw_transform(view_location, view_pitch, view_yaw, forward=60.0)
channel = pf.channel(); actor = pf.guid(); movement = pf.guid()
first_dynamic_offset[channel] = 0
send_reliable(channel, smoke_projectile_open_body(actor, location, pitch, yaw, velocity,
              movement_guid=movement, ignore_pawn=False), True, is_open=True)    seq +0
pf.flights.append(Flight(channel, actor, movement, now, location, velocity, pitch, yaw))
event cloudburst_open = [channel, actor, rounded location, rounded velocity, "A"]
```

`throw_transform` (`rp:267-278`), defaults `upward_arc 8.0`, `upward_shift 8.0`, `speed 2900.0`:

```
vp = ((view_pitch + 180) % 360) - 180          Python float modulo (result has the divisor's sign)
scale = 1 - (0.25 * min(|vp|, 90)) / 90         keep this evaluation order
pitch = min(vp + scale * 8.0, 89.9)
d = (cos p cos y, cos p sin y, sin p)            p = radians(pitch), y = radians(view_yaw)
location = (vx + d.x*60, vy + d.y*60, vz - scale*8.0 + d.z*60)
return location, pitch, view_yaw, d * 2900.0
```

Rust: Python `%` on floats is floored modulo, not `fmod`; use `rem_euclid` for a positive divisor.

### 5.2 Flight tick (`tick_projectiles`, `ruh:1046-1100`, every >= 0.05 s once `projectile_fields[src]` exists)

```
for flight in list(flights):
  if flight.state == "flying":
    snapshot = read_projectile(reader, flight.channel)     (exception -> None, event cloudburst_read_error)
    stop = flight_stop_point(flight, snapshot, now)
    if stop is None: continue
    smoke_location, reason = stop; flight.state = "stopped"; flight.close_at = now
    flight.zone_channel = pf.channel(); zone = pf.guid(); first_dynamic_offset[zone_channel] = 0
    send_reliable(zone_channel, smoke_zone_open_body(zone, smoke_location, random.uniform(0, 360)), True, is_open=True)
    flight.zone_close_at = now + 3.2; event cloudburst_zone = [zone_channel, zone, rounded location, reason]
  if flight.close_at is not None and now >= flight.close_at:
    send_reliable(flight.channel, close); close_at = None; state = "closed"            projectile ch +1
  if flight.zone_close_at is not None and now >= flight.zone_close_at:
    send_reliable(zone_channel, close); zone_close_at = None; event cloudburst_zone_closed += [ch]   zone ch +1
  if state == "closed" and zone_close_at is None: remove flight
```

So in stage A the zone open and the projectile close go out in the same tick (zone open first). `flight_stop_point` (`rp:346-360`):

```
if snapshot and snapshot.class_name == "Projectile_Wushu_4_Smoke_C" and snapshot.location:
    seen = True; last_location = snapshot.location
    if snapshot.collisions >= 1: return (hit_location + hit_normal * 65.0, "hit")
    if now - opened >= 2.0:     return (snapshot.location, "timeout")
    return None
if seen: return (last_location, "lost")
if now - opened >= 0.3: return (ballistic_fallback(flight.location, flight.velocity), "fallback")
return None
```

`ballistic_fallback` (`rp:285-295`), `floor_z 1253.8`, `gravity_z -2500.0 * 0.45 = -1125.0`, `timeout 2.0`, `step 0.005`:

```
elapsed = 0.0; point = location
while elapsed < 2.0:
    elapsed += 0.005                                  accumulated f64; replicate exactly, do not use i * 0.005
    point = (lx + vx*e, ly + vy*e, lz + vz*e + 0.5 * gz * e * e)      e = elapsed, left-to-right evaluation
    if point.z <= 1253.8: return (point.x, point.y, 1253.8 + 65.0)
return point
```

`read_projectile` (oracle, `rp:308-343`): world (`res:47-57`) +0x40 NetDriver, +0xC0 ServerConnection, Channels data +0x13F8 / count +0x1400; `Channels[channel]` +0x78 actor; class FName via actor+0x10 -> +0x18; RootComponent actor+0x290, location 3 doubles at +0x320; FiniteSpeedMovement actor+0x610: NumCollisions u32 +0x198, last hit Location 3 doubles at +0x228, Normal 3 doubles at +0x258.

### 5.3 Body layouts

Projectile open, production form (`smoke_projectile_open_body` with `movement_guid`, `rp:182-222`), 1689 bits in V.4:

```
[1: 0][32: 2]
[p: 610001][8: 5][p: 610003][8: 5][p: 0] FS("/Game/Characters/Wushu/S0/Ability_4/Projectile_Wushu_4_Smoke") [32: 0]
     FS("Default__Projectile_Wushu_4_Smoke_C") [32: 0]
[p: movement][8: 5][p: actor][8: 0] FS("FiniteSpeedMovement") [32: 0]
spawn header (rp:164-179):
  [p: actor][p: 610001][p: 5][1: 1][1: 1][quantized10(location)]
  shorts = (round(pitch*65536/360) & 0xFFFF, round(yaw*65536/360) & 0xFFFF, 0)     round = half-even
  [1: any(shorts)] then if any: per axis [1: s != 0][16: s if != 0]
  [2: 0]
actor block (rp:145-161): [1: 1][1: 1][p: rep bits][enc(actor) of
  [1: 0][p: 11][FRepMovement][p: 12][p: 10][p: 14][p: 10][p: 0]]        ReplicatedMovement, Owner = pawn 10, Instigator = pawn 10
FiniteSpeedMovement block: [1: 1][1: 0][p: movement][1: 1][p: 9][enc(actor) of [1: 0][p: 0]]
guard < 8192 bits
```

FRepMovement (`rp:137-142`, IDA `sub_7FF739016C70`, quantization levels 0/0/0):

```
[4: 0]  bSimulatedPhysicSleep, bRepPhysics, bRepServerFrame, bRepServerHandle
quantized(location, scale 1)
rotator bytes (rp:128-134, FRotator::SerializeCompressed sub_7FF7366CF940):
  per axis pitch, yaw, roll: b = floor(deg * 256/360 + 0.5) & 0xFF; [1: b != 0][8: b if != 0]
quantized(velocity, scale 1)
```

`append_quantized_vector(v, scale)` (`rp:115-125`, reader `sub_7FF737520200`): components rounded half away from zero (`floor(x*s + 0.5)` / `ceil(x*s - 0.5)`); `width = max over components of max(1, bitlen(c >= 0 ? c : ~c) + 1)`; `> 63` raises; `[7: width | 0x40]`, then each component as width-bit two's complement. Scale 10 equals `append_quantized_vector10` (`test_range_projectiles.py:35-42`, 500 random vectors).

Smoke zone open (`smoke_zone_open_body`, `rp:253-264`): `build_dynamic_actor_reference(("Default__GameObject_Wushu_4_SmokeZone_C", "/Game/Characters/Wushu/S0/Ability_4/GameObject_Wushu_4_SmokeZone"), zone, 610005, 610007, 5, location, yaw)` + actor block `[1: 1][1: 1][p: 41][enc(zone) of [1: 0][p: 12][p: 10][p: 14][p: 10][p: 0]]` (no ReplicatedMovement: the zone has bReplicateMovement 0).

### 5.4 Allocation

`ProjectileField` (`rp:89-107`): `channel()` returns next (from 10000), raises `RuntimeError` above 32766, `+1`; `guid()` returns next (from 610100), `+2`. Per throw: projectile channel, actor GUID, movement GUID at the throw; zone channel and zone GUID at the stop. Never reused.

## 6. Teams

### 6.1 Team-state builders (B3-B8; scheduling and framed vectors in `04_bringup_b.md`)

| stage | channel / seq | body | bits |
|---|---|---|---|
| B3 x2 | ch10, ch11 / B+0, open, exports | `build_dynamic_base_team_state_reference(100 + 2i)` (`rsb:206-218`) = dynamic actor reference `("Default__BaseTeamState", "/Script/ShooterGame")`, actor 100/102, archetype 101, package 103, level 5, no transform | 573 |
| B4 | ch4 / B+3 | rep block on GameState, seed 2: `build_object_array_property_payload(34, (100, 102))` (`rsb:1300-1322`): `[1: 0][p: 34][16: 2][p: 1][p: 100][p: 2][p: 102][p: 0][p: 0]` (73 bits) | 83 |
| B5 x2 | ch10, ch11 / B+1, exports | `build_team_state_component_update(100+2i, 104+2i, "BlueTeam"/"RedTeam", 1+i)` (`rsb:1351-1392`): `[1: 0][32: 1][p: comp][8: 5][p: 2][8: 0] FS(name) [32: 0]` + rep block on the team actor (seed 100+2i): `[1: 0][p: 16][p: comp][p: 17][3: role][p: 18][3: role][p: 0]` (47 bits; TeamComponent 16, InitialRole 17, TeamRole 18; EAresTeamRole 3 bits) | 258 / 250 |
| B6 | ch6 / B+5 | rep block on PlayerState, seed 6: `build_object_property_payload(27, 100)` = `[1: 0][p: 27][p: 100][p: 0]` | 35 |
| B7 | ch6 / B+6, exports | inline `ruh:3548-3568`: `[1: 0][32: 1][p: 108][8: 5][p: 6][8: 0] FS("BombTeam") [32: 0][1: 1][1: 0][p: 108][1: 1][p: 9][enc(6) of [1: 0][p: 0]]` | 229 |
| B8 | ch6 / B+7 | inline `ruh:3601-3615`: `[1: 1][1: 0][p: 108][1: 1][p: 25][enc(6) of [1: 0][p: 3][8: 1][p: 0]]` (TeamNumber handle 3 = UNumberedTeamComponent parent 2 at +0xE8, BlueTeam = 1) | 44 |

Gates (copied from 04): B3 `game_state_phase_sent and base_pistol_open_sent and index < 2 and now - max(pistol open, last B3) >= 1.0`; B4 index == 2 and >= 2.0 s after the last B3; B5 >= 1.0 s after max(B4, last B5), index < 2; B6 link index == 2 and >= 2.0 s after the last B5; B7 >= 1.0 s after B6; B8 >= 1.0 s after B7. All are arms of the session-ack `elif` chain, one per tick.

### 6.2 `range_bomb_team_open.py`

Draft helper (`rbt:1-14` says so). `build_bomb_team_open_bunch` and `build_bomb_team_number_update` return complete frames. `subobj_guid` is a required argument (the module constant `DEFAULT_BOMB_TEAM_GUID = 108`, `rbt:22`, is not a default); fixed outer 6, channel 6, `channel_name_index` 102; the helper's own defaults (`server_sequence 0`, `ack_history_mask 7`, `stateless_header 0`) are not production values, so always pass every frame parameter. Called with production parameters (open `channel_sequence = B+6`, number `team_number=1`, `team_number_wire_handle=3`, `channel_sequence = B+7`), they produce frames byte-identical to the inline bodies (V.5). Port one builder pair; do not port the helper's live-handle-lookup framing comments.

### 6.3 Bot teams

Each bot carries its own BombTeam component (bot+0x1800, UBombTeamComponent; the `Pawn_TrainingBot` template Team is 1 = the player's team). The bot open sets Team = 0 in its first bunch because content blocks apply before PostNetInit -> BeginPlay -> UpdateTeam, which is what sets the outline and minimap; a later Team-only update does not refresh them (`rb:16-21`). The player's own team is the PlayerState BombTeam = 1 (B8). Red minimap icons (ClientUpdateVisibility, field 123/178 per session notes, not in the code) are not sent by production; do not add them to the parity port.

## 7. Port notes

### 7.1 128 tps loop

The Python has no tick: everything here runs while one received datagram is handled (the socket loop blocks in `recvfrom` with a timeout of at most 0.25 s, `ruh:1410-1442`). The Rust server runs at 128 tps (128 Hz fixed loop, 7.8125 ms, `DESIGN.md`):

- Each tick: drain the socket (receive until it would block) and, for each authenticated datagram in arrival order, run the section 1.1 pipeline once, with one `now` per datagram (its receive timestamp), building frames with that datagram's `acked_client_sequence`, `stateless_header` and ack mask. This is the parity mode: the 128 Hz loop is the scheduler, the systems stay datagram-driven.
- Consequence to keep in parity mode: with no client datagram nothing in this file is sent, including bot retire/respawn, the Cloudburst zone close and ability job steps; they fire on the next datagram after their deadline.
- Keep every gate as a monotonic-time comparison (mirror/shots 0.05 s, projectile tick 0.05 s, bot tick 0.1 s, spawn spacing 0.3 s, retire 1.5 s, flight timeouts 0.3 s / 2.0 s, zone lifetime 3.2 s, ability job steps 0.25 / 0.4 / 0.3 s, 20 registration retries). Do not convert them to tick counts (0.05 s = 6.4 ticks).
- Later, a tick-driven mode (run the bot and projectile ticks from the 128 Hz loop with the last packet's ack fields) changes only server sequences and timing, never bodies or per-channel sequences. Make it a deliberate switch after parity is shown.
- Oracle reads must not block the tick: use the snapshot model from `DESIGN.md`. The Python reads synchronously per packet; with a snapshot, the camera/seed/state values may be up to one oracle period old. Shot detection compares consecutive seed reads, so a slower oracle merges shots (Python already merges shots inside one 50 ms poll).

### 7.2 Console and logging

The server must always show a console with live logging, and also log to `logs/aries_server_<pid>.log` (`DESIGN.md`). The Python responder is launched hidden (`launch_range.py`: `subprocess.Popen(..., creationflags=CREATE_NO_WINDOW)`, stdout redirected to `runtime/responder_<pid>_*.out`), so the Rust binary cannot rely on an inherited console: at startup, if `GetConsoleWindow()` is null, call `FreeConsole()` (ignore failure) then `AllocConsole()`, and reopen stdout/stderr on `CONOUT$` before the first log line. Keep the Python event keys so the shadow diff against `runtime/responder_<pid>_phase4_match_state.out` works:

| system | keys |
|---|---|
| abilities | `ability_open_tuning` `[i, bits, constructed]`, `ability_open_tuning_error`, `ability_ready` `[[class, states, [extra]]]`, `ability_register_error`, `ability_register_gave_up`, `extra_machines_error` |
| bots | `dummy_spawned` `[ch, actor]`, `bot_spawned` `[[ch, actor, [x, y, z]]]`, `bot_retired` `[actor]`, `bot_hits` `[[actor, region, damage, life, distance]]`, `dummy_hits` `[[region, damage, distance]]`, `bot_shot_missed` (count), `bot_camera_error` |
| Cloudburst | `cloudburst_open` `[ch, actor, [x,y,z], [vx,vy,vz], "A"]`, `cloudburst_zone` `[ch, zone, [x,y,z], reason]`, `cloudburst_zone_closed` `[ch]`, `cloudburst_camera_error`, `cloudburst_read_error` |
| teams | as in 04 (`sent_team_state_*`, `sent_bomb_team_*`, `bomb_team_subobj_guid`, `bomb_team_number_value/bits`) |

All logged coordinates and distances use Python `round()` (half-even) to int. Shot and spawn lines are the useful ones on a quiet console; keep everything in the file log.

### 7.3 Oracle inputs used here

| input | Python | used by |
|---|---|---|
| camera POV (6 doubles) | `read_camera`, controller+0x528 -> +0x17C0 | shots, Cloudburst throw |
| firing seed (u32) | `read_firing_seed`, FiringState +0x4A0 -> +0xEC | shot detection |
| slot machine (item, class, predicted, is_current) | `read_slot_machines` (`res:132-164`) | mirror gate, Cloudburst trigger, ability registration |
| main SM state names (SM+0x160 order), item machines (item+0xE30) with starting state (+0x1E8) | `state_names`, `read_item_machines` | ability registration GUID allocation |
| projectile snapshot | `read_projectile` | Cloudburst stop |
| six tag net indices, N, NetIndexFirstBitSegment | `predict_net_indices` | ability opens (B18) |

### 7.4 Float and rounding parity checklist

- `round()` = half-even: yaw/pitch shorts in actor headers, event values. Rust `f64::round_ties_even`.
- `floor(x + 0.5)`: FRotator bytes. Half away from zero: quantized vectors.
- f32 packing (`struct.pack("<f")`): round to nearest f32 (`as f32`).
- `&0xFFFF` / `&0xFF` on negative Python ints = two's-complement low bits (`(v as i64) & 0xFFFF`).
- Python float `%` is floored (`rem_euclid` for positive divisors).
- CPython 3.12 `sum()` over floats is Neumaier-compensated (used by `_ray_sphere`, section 4.4); `min(..., key=)` keeps the first of equal keys.

## 8. Open questions

1. The six AbilityTuning tag net indices are not in any file or log. The port needs the oracle (or a one-time capture by the user) before B18 i = 2, 3, 4 can be byte-identical. V.1 uses sample indices.
2. Cloudburst stage A has never run live: channel indices >= 10000, the projectile open and the zone open are unverified on the client. Test on a live session (with the user's go-ahead) before relying on it.
3. Smoke-zone yaw comes from the unseeded global `random`; Python output is not reproducible. Any yaw is acceptable for parity; tests must inject it.
4. Bot channels are never reused and have no upper guard (`rb:136-146`): every respawn takes a new channel. Keep that for parity, but add a guard (the projectile allocator stops at 32766).
5. Ability registration GUIDs depend on oracle state-name order and job timing; a shadow diff needs the same oracle data on both sides.
6. Shot detection is one shot per seed change per 50 ms poll and ignores spread/recoil. Improving it changes behaviour, not bytes, and should be a separate step.
7. `place()` uses `hypot` and the CRT trig functions; the Rust build must be checked against the V.3 `repr` values on the target machine.

## Appendix V. Test vectors

Generated by `gen08.py` (scratchpad, not committed), which imports `range_abilities`, `range_bots`, `range_projectiles`, `range_bomb_team_open`, `range_server_bunch`, `range_equippables` and `range_gun_loader` read-only. It wraps `range_server_bunch.transform_short_packet` and `encode_short_actor_rep_payload` in memory to capture `decoded` and every `plain`/`encoded` pair.

Fixed frame parameters (same as `04_bringup_b.md` section 11): B = 1000, server_sequence 8770, acked_client_sequence 4817, ack_history_mask 0xFFFFFFFF, stateless_header 16, receive_key bytes 00..1f, channel_name_index 102. Field legend as in 04: `plain` before the content transform, `encoded` after it (same bit count, seed shown), `body` = bunch body, `decoded` = packet before `transform_short_packet`, `frame` = UDP datagram. Hex is LSB-first `BitWriter.to_bytes()` order; long lines wrap at 128 hex chars (concatenate continuation lines).

### V.1 Abilities

`plan_ability_load` with one fresh `ability_allocator()`, indices in `ability_indices` order:

| index | class | channel | actor GUID | slot | state machine GUID | InactiveState GUID |
|---|---|---|---|---|---|---|
| 2 | Ability_Wushu_4_Smoke_C | 14 | 208 | 3 | 100000 | 100002 |
| 3 | Ability_Wushu_E_Dash_C | 15 | 212 | 5 | 100004 | 100006 |
| 4 | Ability_Wushu_Q_CycloneBoost_C | 16 | 216 | 4 | 100008 | 100010 |
| 5 | Ability_Wushu_X_Dagger_Production_C | 17 | 220 | 9 | 100012 | 100014 |

Allocator after the four plans: next_guid = 100016 (the first GUID handed out at registration: an extra-machine GUID if the first job to register has an extra machine, else that job's first state GUID), next_channel = 0 (never used).

#### `ability2_role`

Frame shown with the offsets the job takes when no resource-mirror send has used the channel first (+2, +3, +4); see section 2.5.

```
ch14 reliable seq=1002 (B+2) open=0 exports=0
plain        20b 100400   (seed 208)
encoded      20b e01e02
body         30b a3807b08
decoded     158b 102d0989ffffffff011039407d327b8051c03d24
frame        34B 000000001000000040729ee0c623f47889322f2bcfe1f5eeccf4e51aa4506e25c21c
```

#### `ability2_state_machine_open`

```
ch14 reliable seq=1003 (B+3) open=0 exports=1
plain         9b 0000   (seed 208)
encoded       9b 1100
body        381b 020000008236180a4205002e0000008ae2ead2e0e0c2c4d8caa6e8c2e8ca9ac2c6d0d2dcca00000000000ada60281101
decoded     509b 102d0989ffffffff011039627d32f70501000000411b0c05a102001700000045717569707061626c6553746174654d616368696e650000000000056d30948810
frame        78B 0000000010000000809b7c2ed923f47889322f2bcfe175dfcff4e53724be133c5afa24b170363dbc6b7338c1308057e4fff2f054d8920af16145232fa04bef0c
                 6ac78a52cb232a90c3d9d9c3570e
```

#### `ability2_inactive_state`

```
ch14 reliable seq=1004 (B+4) open=0 exports=1
plain        41b 0c8a36180000   (seed 208)
encoded      41b 9e038d1f3001
body        341b 020000008a36180a4205001c00000092dcc2c6e8d2eccaa6e8c2e8ca00000000000ada6028e539d0f80113
decoded     469b 102d0989ffffffff011039827d32570501000000451b0c05a102000e000000496e61637469766553746174650000000000056d3094f21c68fc8019
frame        73B 0000000010000000804b8938c623f47889322f2bcfe1755fd0f4e51324be3a3c5afa24b1707674ad6b73b8e370768f53dadedd9a9d34a12a994b0dd0decda841
                 851142dab51827440e
```

#### `ability3_role`

```
plain        20b 100400   (seed 212)
encoded      20b 58910b
body         30b a360452e
```

#### `ability3_state_machine_open`

```
plain         9b 0000   (seed 212)
encoded       9b 0701
body        381b 020000009236180a5205002e0000008ae2ead2e0e0c2c4d8caa6e8c2e8ca9ac2c6d0d2dcca00000000004ada60287110
```

#### `ability3_inactive_state`

```
plain        41b 0c9a36180000   (seed 212)
encoded      41b 9e02b4e78c01
body        341b 020000009a36180a5205001c00000092dcc2c6e8d2eccaa6e8c2e8ca00000000004ada6028e529407bce18
```

#### `ability4_role`

```
plain        20b 100400   (seed 216)
encoded      20b ec0f0f
body         30b a3b03f3c
```

#### `ability4_state_machine_open`

```
plain         9b 0000   (seed 216)
encoded       9b cd01
body        381b 02000000a236180a6205002e0000008ae2ead2e0e0c2c4d8caa6e8c2e8ca9ac2c6d0d2dcca00000000008ada6028d11c
```

#### `ability4_inactive_state`

```
plain        41b 0caa36180000   (seed 216)
encoded      41b 9e0bc94d4200
body        341b 02000000aa36180a6205001c00000092dcc2c6e8d2eccaa6e8c2e8ca00000000008ada6028e5b990dc2404
```

#### `ability5_role`

```
plain        20b 100400   (seed 220)
encoded      20b 493c0e
body         30b a324f138
```

#### `ability5_state_machine_open`

```
plain         9b 0000   (seed 220)
encoded       9b 0501
body        381b 02000000b236180a7205002e0000008ae2ead2e0e0c2c4d8caa6e8c2e8ca9ac2c6d0d2dcca0000000000cada60285110
```

#### `ability5_inactive_state`

```
plain        41b 0cba36180000   (seed 220)
encoded      41b 9e0aad03c500
body        341b 02000000ba36180a7205001c00000092dcc2c6e8d2eccaa6e8c2e8ca0000000000cada6028e5a9d03a500c
```

AbilityTuning body (`build_ability_tuning_body`), test inputs from `test_range_abilities.py:57-59`: `tuning_one_float` = `build_ability_tuning_body(212, 190006, [TuningEntry(245, "float", 7.5)], 10)`; `tuning_float_int` = the same plus `TuningEntry(244, "int", 1)`.

#### `tuning_one_float`

```
plain       110b 0c020004ea21900200003c100000   (seed 212)
encoded     110b d9c37e467efb5f61a01879be6330
body        145b b5655ae4ce1ef633f2dbff0a03c5c8f31d8301
```

#### `tuning_float_int`

```
plain       179b 0c040004ea21900200003c90033d10a108000000000000   (seed 212)
encoded     179b dbc37e3375f6b961c6c90d968a90f465c6597be3a64c01
body        222b b5655a3c13d81ef69ba9b3cf0d334e6eb05484a42f33ceda1b37650a
```

#### `tuning_bool_export`

Not used by production (`export=False` there; kinds are float/int only); shows the bool leaf and the export prefix. Exact call: `build_ability_tuning_body(212, 190006, [TuningEntry(7, "bool", True)], 10, export=True)` (tag index 7 is arbitrary).

```
plain        79b 0c0200040e2080410000   (seed 212)
encoded      79b 58d7ad6e7e5ab961f36d
body        379b 02000000da322d0a5205001c00000082c4d2d8d2e8f2a8eadcd2dcce00000000006acbb4e88975ddeae6a7951b36df06
```

Ability opens with tuning (`build_ability_open_with_tuning(i, tags, 10)`). **The tag indices are SAMPLE values**, not live ones: E tags 243/244/245 are the values used by `test_range_abilities.py:51-53`; the others are made up. Every tag occupies exactly 10 bits at the position shown in section 2.3, so a Rust test can swap indices without changing anything else:

```
AbilityTuning.Wushu.4.SmokeDuration            240
AbilityTuning.Wushu.4.SmokeMaxCharges          241
AbilityTuning.Wushu.E.DashVelocityMultiplier   243
AbilityTuning.Wushu.E.MaxCharges               244
AbilityTuning.Wushu.E.PrimeDuration            245
AbilityTuning.Wushu.Q.MaxCharges               250
```

#### `ability2_open_with_tuning`

```
plain       179b 0c040004e021900200000890433c10a110000000000000   (seed 208)
encoded     179b db427e37756bb961470c285bfd5f6a270575c6d8bee900
body       1443b 0400000046050a4e050a00740000005e8ec2daca5e86d0c2e4c2c6e8cae4e65eaeeae6d0ea5ea6605e82c4d2d8d2e8f2be685e82c4d2d8d2e8f2beaeeae6d0ea
                 be68bea6daded6ca00000000004200000088caccc2ead8e8bebe82c4d2d8d2e8f2beaeeae6d0eabe68bea6daded6cabe860000000000d2322d0a4205001c0000
                 0082c4d2d8d2e8f2a8eadcd2dcce00000000004205460514a0b44c8b6702db427e37756bb961470c285bfd5f6a270575c6d8bee900
```

#### `ability3_open_with_tuning`

```
ch15 reliable seq=1000 (B+0) open=1 exports=1
plain       248b 0c060004e62190020000e08f433d10b200008007d2a0872320010000000000   (seed 212)
encoded     248b dba77e5a436575616495acba84008a7fd6ec4cebbe8ebdb8d46b1f3cba4abf
body       1496b 0400000056050a5e050a00720000005e8ec2daca5e86d0c2e4c2c6e8cae4e65eaeeae6d0ea5ea6605e82c4d2d8d2e8f2be8a5e82c4d2d8d2e8f2beaeeae6d0ea
                 be8abe88c2e6d000000000004000000088caccc2ead8e8bebe82c4d2d8d2e8f2beaeeae6d0eabe8abe88c2e6d0be860000000000da322d0a5205001c00000082
                 c4d2d8d2e8f2a8eadcd2dcce00000000005205560514a0b64c8bf102dba77e5a436575616495acba84008a7fd6ec4cebbe8ebdb8d46b1f3cba4abf
decoded    1624b 102d0989ffffffff01303d027d32631702000000ab0205af020500390000002f47616d652f436861726163746572732f57757368752f53302f4162696c697479
                 5f452f4162696c6974795f57757368755f455f4461736800000000002000000044656661756c745f5f4162696c6974795f57757368755f455f446173685f4300
                 000000006d991605a902000e0000004162696c69747954756e696e670000000000a902ab020a505ba6c57881ed533fada1b2ba30b24a565d4200c53f6b76a675
                 5fc75e5ceab50f1e5da5df
frame       217B 0000000010000000c0b0dc7bcf23f47889322f2bcfe1b54853d4e5e6e071c2c457ba10b17036fdbb2b27040fd573526ab1bc3707651c76e45d48d64677052377
                 8873f3f645084618db6448c20f7c43ccf2ff59605807064b9312fa28d2caa045f0c79864348f072d99247e6ffd4fdd247bee75f5827214b59b9297e5622305a4
                 f7b56c1f200b209173e0d8f039108c6be231035a39a253f4cfea272f488fc1172495c557c4d0749e4fcd61836f11865dbabf9bd2c38e04d770381f7f5763ba52
                 e254487fd907fdbb4896761727e48424a5eba8659c741c1673
```

#### `ability4_open_with_tuning`

```
plain       110b 0c020004f4210842000000000000   (seed 216)
encoded     110b d97073f57e675f61644715775616
body       1478b 0400000066050a6e050a00820000005e8ec2daca5e86d0c2e4c2c6e8cae4e65eaeeae6d0ea5ea6605e82c4d2d8d2e8f2bea25e82c4d2d8d2e8f2beaeeae6d0ea
                 bea2be86f2c6d8dedcca84dedee6e800000000005000000088caccc2ead8e8bebe82c4d2d8d2e8f2beaeeae6d0eabea2be86f2c6d8dedcca84dedee6e8be8600
                 00000000e2322d0a6205001c00000082c4d2d8d2e8f2a8eadcd2dcce00000000006205660514a0b84c8bdcd97073f57e675f61644715775616
```

Default (no tuning) open sizes for comparison: i=2 989 bits, i=3 973 bits, i=4 1101 bits.

### V.2 Gameplay-tag net index (pure algorithm)

Copy of `gameplay_tag_net_index.py:87-101` applied to a synthetic list (node map order `b.Tag, A.tag, a.Tag2, C, a_b, a.b, A`, CommonlyReplicatedTags `[C, a_b]`):

```
0  C
1  a_b
2  A.tag
3  a.Tag2
4  a.b
5  b.Tag
6  A
TrueBitNum = ceil(log2(7+1)) = 3
```

Sorted order before the swaps is `A, a.b, A.tag, a.Tag2, a_b, b.Tag, C`. The common-tag step swaps (it does not insert): `C` moves to 0 and `A` moves to `C`'s old slot 6; then `a_b` moves to 1 and `a.b` to slot 4.

| tag count N | TrueBitNum = ceil(log2(N+1)) |
|---|---|
| 255 | 8 |
| 256 | 9 |
| 511 | 9 |
| 512 | 10 |
| 898 | 10 |
| 1022 | 10 |
| 1023 | 10 |
| 1024 | 11 |

### V.3 Bots

#### MT19937 / `random.Random(1306)`

```
first 12 genrand_uint32 outputs after seed(1306):
  0xc9121a55 0xcbbd14f6 0x7aedd71e 0x8dbbd8ca
  0x19099857 0xf838f274 0x2b0f1caa 0xe7d74b9e
  0x4cfe782d 0xeab9251c 0x78b73e22 0x8af83541
first 6 random() = ((u0>>5)*67108864 + (u1>>6)) / 2^53:
  0.785432478754624  (3fe922434b2ef453)
  0.480191652131435  (3fdebb75c46ddec6)
  0.09780266113295355  (3fb909985f071e48)
  0.16819934476635523  (3fc5878e5e7d74b8)
  0.30075789926117347  (3fd33f9e0f55c928)
  0.47154606538154076  (3fde2dcf8c57c1aa)
```

#### Production allocation order (first bot tick: dummy, then one bot per >= 0.3 s; no deaths)

| entity | Bot.index | channel | actor | health | handler | BombTeam | location (repr) | yaw (repr) | team | uniform() draws |
|---|---|---|---|---|---|---|---|---|---|---|
| dummy | -1 | 400 | 300100 | 300102 | 300104 | 300106 | (6190.0, -11640.0, 1270.0) | 180.0 | 0 | 2 (location discarded) |
| bot | 0 | 401 | 300108 | 300110 | 300112 | 300114 | (4463.170774427629, -14407.674895086804, 1351.8) | 65.86815966797721 | 0 | 2 |
| bot | 1 | 402 | 300116 | 300118 | 300120 | 300122 | (4572.564072216778, -14792.2698646287, 1351.8) | 78.04547395567045 | 0 | 2 |
| bot | 2 | 403 | 300124 | 300126 | 300128 | 300130 | (4795.468318650681, -14628.70432286504, 1351.8) | 88.35562457025777 | 0 | 4 |
| bot | 3 | 404 | 300132 | 300134 | 300136 | 300138 | (5186.609028363901, -14270.230718226876, 1351.8) | 118.34815636375865 | 0 | 4 |
| bot | 4 | 405 | 300140 | 300142 | 300144 | 300146 | (4754.492781286616, -14341.763633110617, 1351.8) | 84.5701504353565 | 0 | 4 |

Live cross-check (`local_backend/runtime/responder_20928_phase4_match_state.out`, read-only grep): `"dummy_spawned": [400, 300100]`, then `"bot_spawned"` `[401, 300108, [4463, -14408, 1352]]`, `[402, 300116, [4573, -14792, 1352]]`, `[403, 300124, [4795, -14629, 1352]]`, `[404, 300132, [5187, -14270, 1352]]`, `[405, 300140, [4754, -14342, 1352]]`. All match the table. Note that `responder_20928_phase4_match_state.out` (1.3 GB) holds several appended sessions: 14 start with the older stride-6/no-dummy code (`bot_spawned [[400, 300100, [5187, -14774, 1352]]]`, then 401/300106, ...) and only 2 (first at line 1370905) with the current code (`dummy_spawned [400, 300100]` in the same event as `bot_spawned [[401, 300108, ...]]`). Compare against the current-code sessions only. In those, ch406 (300148, `(4972, -15364)`) opens at line 1374633, before the first `bot_retired [300140]` at 1374815 (section 4.3). The older code in `responder_24252` (stride 6, no dummy) put its first bot at `(5187, -14774)`, which is exactly the placement the dummy now consumes and discards.

Spawn transform as written by `append_dynamic_actor_header` (quantized x10, half away from zero; yaw short = Python `round()` half-even):

```
ch401: quantized [44632, -144077, 13518]  yaw_short 0x2ed7 (11991)
ch402: quantized [45726, -147923, 13518]  yaw_short 0x3780 (14208)
ch403: quantized [47955, -146287, 13518]  yaw_short 0x3ed5 (16085)
ch404: quantized [51866, -142702, 13518]  yaw_short 0x5429 (21545)
ch405: quantized [47545, -143418, 13518]  yaw_short 0x3c24 (15396)
ch400 dummy: quantized [61900, -116400, 12700]  yaw_short 0x8000
```

#### `bot0_open`

```
ch401 reliable seq=1000 (B+0) open=1 exports=1
plain        25b 0c000000   (seed 300108)
encoded      25b 1771fb00
body       1197b 04000000869f480a8e9f480a00580000005e8ec2daca5e86d0c2e4c2c6e8cae4e65ea8e4c2d2dcd2dcce84dee85ea8e4c2d2dcd2dcce84dee8bea08600000000
                 003400000088caccc2ead8e8bebea8e4c2d2dcd2dcce84dee8bea086be8600000000004aa3480a32a348001200000084dedac4a8cac2da000000000032a34886
                 9f48149e62b962a6b9ce34e8b50b2a8d22297311b70f
decoded    1333b 102d0989ffffffff0130470c027d32b71202000000c34f2405c74f2405002c0000002f47616d652f436861726163746572732f547261696e696e67426f742f54
                 7261696e696e67426f745f504300000000001a00000044656661756c745f5f547261696e696e67426f745f50435f430000000000a55124059951240009000000
                 426f6d625465616d0000000000995124c34f240a4fb15c31d35c671af4da0595469194b988db17
frame       181B 00000000100000008002eae2d723f47889322f2bcf850f5b73d4658df6b8520facd02ab170b63c35b7528fa7275c2a7f1a7c292b15e4cc0cc5386cbb0e509b0f
                 70cbe7185e0e37d489c5ff2d07aaa78f6cd8b7d1b903f139de438b1f124efba35b835d16d7731de825eba7740bd9d7c16c5e6a84465477fc2f0abd95a716efc8
                 9d19d288ceb71ad1db9231354816f1492e3a2bd02d82faea883a273808b74f8c3e1f88390dffe746aa5df2b35a7daf1124e226440e
```

#### `bot0_health_export`

```
ch401 reliable seq=1001 (B+1) open=0 exports=1
plain        98b 0c220000200b290000200b0100   (seed 300108)
encoded      98b 025c61ac0d653a64f760037d00
body        454b 020000003aa3480a32a348002800000090cac2d8e8d088c2dac2cecaa6cac6e8d2dedc0000000000ea8c22492cc015c6da50a643760f36d007
decoded     590b 102d0989ffffffff0110470c227d321b07010000009d51240599512400140000004865616c746844616d61676553656374696f6e00000000007546912416e00a
                 636d28d321bb071be823
frame        88B 0000000010000000406ff6a4d723f47889322f2bcf450e74f3f465cdd5aa520ff5391eb170ae01b5ab508fe7a0c76557714f2942f2d6137a3f90dedfb90bec0c
                 6a5b8952dc8543007f9c47d98bfec21897148885be2dd61c
```

#### `bot1_open`

```
ch402 reliable seq=1000 (B+0) open=1 exports=1
plain        25b 0c000000   (seed 300116)
encoded      25b cd2d7401
body       1197b 04000000869f480a8e9f480a00580000005e8ec2daca5e86d0c2e4c2c6e8cae4e65ea8e4c2d2dcd2dcce84dee85ea8e4c2d2dcd2dcce84dee8bea08600000000
                 003400000088caccc2ead8e8bebea8e4c2d2dcd2dcce84dee8bea086be8600000000006aa3480a52a348001200000084dedac4a8cac2da000000000052a34886
                 9f48149e7acaa2c5b7ce3428e00daa8d2229d3dc4217
decoded    1333b 102d0989ffffffff01304b0c027d32b71202000000c34f2405c74f2405002c0000002f47616d652f436861726163746572732f547261696e696e67426f742f54
                 7261696e696e67426f745f504300000000001a00000044656661756c745f5f547261696e696e67426f745f50435f430000000000b5512405a951240009000000
                 426f6d625465616d0000000000a95124c34f240a4f3d65d1e25b671a14f006d5469194696ea11b
frame       181B 000000001000000000a080d0d223f47889322f2bcf850f5b73d465cdefb8520facd02ab170b63c35b7528fa7275c2a7f1a7c292b15e4cc0cc5386cbb0e509b0f
                 70cbe7185e0e37d489c5ff2d07aaa78f6cd8b7d1b903f139de438b1f124efba35b835d16d7731de825eba7740bd9d7c16c5e6a84465477fc2f0abd95a756f3c8
                 9d19d288ceb71ad1dbd204354816f1492e3a2bd0ada8faea883a27388897cf4c071f88394db3d767ac66d6337574af1124fb3e470e
```

#### `bot1_health_export`

```
ch402 reliable seq=1001 (B+1) open=0 exports=1
plain        98b 0c220000200b290000200b0100   (seed 300116)
encoded      98b 025c619c0da53a640a276b2b02
body        454b 020000005aa3480a52a348002800000090cac2d8e8d088c2dac2cecaa6cac6e8d2dedc00000000006a8d22492cc015c6d950aa43a670b2b622
decoded     590b 102d0989ffffffff01104b0c227d321b0701000000ad512405a9512400140000004865616c746844616d61676553656374696f6e0000000000b546912416e00a
                 e36c28d5215338595b31
frame        88B 0000000010000000c0b49db8e723f47889322f2bcf450e74f3f4650d35b0520ff5391eb170ae01b52b668fe7a0c76557714f2942f2d6137a3f90dedfb90bec0c
                 6a5b8952dc8543003fbc47d9080b45eabddc90b4feaed21c
```

#### `bot2_open`

```
plain        25b 0c000000   (seed 300124)
encoded      25b b1715600
body       1197b 04000000869f480a8e9f480a00580000005e8ec2daca5e86d0c2e4c2c6e8cae4e65ea8e4c2d2dcd2dcce84dee85ea8e4c2d2dcd2dcce84dee8bea08600000000
                 003400000088caccc2ead8e8bebea8e4c2d2dcd2dcce84dee8bea086be8600000000008aa3480a72a348001200000084dedac4a8cac2da000000000072a34886
                 9f48149e4eed2292b8ce3468b50f2a8e2229131b6705
```

#### `bot3_open`

```
plain        25b 0c000000   (seed 300132)
encoded      25b 85c55100
body       1197b 04000000869f480a8e9f480a00580000005e8ec2daca5e86d0c2e4c2c6e8cae4e65ea8e4c2d2dcd2dcce84dee85ea8e4c2d2dcd2dcce84dee8bea08600000000
                 003400000088caccc2ead8e8bebea8e4c2d2dcd2dcce84dee8bea086be860000000000aaa3480a92a348001200000084dedac4a8cac2da000000000092a34886
                 9f48149e6a2a4352bace34680a15aa8e222953581c05
```

#### `bot4_open`

```
plain        25b 0c000000   (seed 300140)
encoded      25b 7342c500
body       1197b 04000000869f480a8e9f480a00580000005e8ec2daca5e86d0c2e4c2c6e8cae4e65ea8e4c2d2dcd2dcce84dee85ea8e4c2d2dcd2dcce84dee8bea08600000000
                 003400000088caccc2ead8e8bebea8e4c2d2dcd2dcce84dee8bea086be860000000000caa3480ab2a348001200000084dedac4a8cac2da0000000000b2a34886
                 9f48149ee6e6c2f8b9ce3428090f2a8f22293327540c
```

#### `bot0_health_after_classic_body_74`

```
ch401 reliable seq=1002 (B+2) open=0 exports=0
plain        98b 0c220000500a290000200b0100   (seed 300108)
encoded      98b 9c8661ac0d653a64f760037d00
body        133b 75469124e6340c636d28d321bb071be803
decoded     269b 102d0989ffffffff0110470c407d3217823aa34812731a86b13694e990dd830df411
frame        48B 000000001000000080cc0ed8ca23f47889322f2bcf859048f3f465cd55b4eb80dcebb8209beda93c15abbac643e47c0e
```

#### `bot0_health_dead`

```
ch401 reliable seq=1003 (B+3) open=0 exports=0
plain        98b 0c2000000000280000200b0100   (seed 300108)
encoded      98b 756761cc23653a64f760037d00
body        133b 75469124ae3b0b631e29d321bb071be803
decoded     269b 102d0989ffffffff0110470c607d3217823aa34812d79d85318f94e990dd830df411
frame        48B 0000000010000000c09ed30ed523f47889322f2bcf059548f3f465cdd5b26b71e8ebb8209beda93c557a9f4667e47c0e
```

#### `bot0_alive_false_export`

```
ch401 reliable seq=1004 (B+4) open=0 exports=1
plain        18b 0c0000   (seed 300108)
encoded      18b 2c5100
body        398b 0200000042a3480a32a348002e00000088c2dac2ceca90c2dcc8d8cae486dedae0dedccadce800000000000a8d2249c21205
decoded     534b 102d0989ffffffff0110470c827d323b0601000000a1512405995124001700000044616d61676548616e646c6572436f6d706f6e656e74000000000085469124
                 618922
frame        81B 0000000010000000c0b3e032cf23f47889322f2bcfc5d864f3f465cd9599520ff5792ab1708401b5ab508fe76050aa765a61e9ce6a1c8f7c31971e91f948ef0c
                 7671875251858907f3b047287739e2ea1c
```

#### `bot0_close_destroyed`

```
ch401 reliable seq=1005 (B+5) close=1 reason=0 exports=0 body 0 bits
decoded     140b 102d0989ffffffff015070c400da27330008
frame        32B 000000001000000000b7e661c523f47889322f2b8f7e0605f0d025b49ba72107
```

#### `bot0_alive_false_noexport`

Not sent by production (death always sends the export form); for builder coverage.

```
plain        18b 0c0000   (seed 300108)
encoded      18b 2c5100
body         53b 85469124618902
```

#### `dummy_open`

```
ch400 reliable seq=1000 (B+0) open=1 exports=1
body       1030b 02000000969f480a9e9f480a005a0000005e8ec2daca5e90cae4decae65e88cac6def25ea8c2e4cecae8a0e4c2c6e8d2c6caa8e4c2d2dcd2dcce84dee8000000
                 00004a00000088caccc2ead8e8bebea8c2e4cecae8a0e4c2c6e8d2c6caa8e4c2d2dcd2dcce84dee8be86000000000012a348969f48149632c7039523670c0500
                 04
decoded    1166b 102d0989ffffffff0130430c027d321b1001000000cb4f2405cf4f2405002d0000002f47616d652f4865726f65732f4465636f792f5461726765745072616374
                 696365547261696e696e67426f7400000000002500000044656661756c745f5f5461726765745072616374696365547261696e696e67426f745f430000000000
                 895124cb4f240a4b99e381ca913386020022
frame       160B 000000001000000080ccade0d123f47889322f2bcf850f7473d4654d4f81520fec3923b170b63c75f5748fe73e5c2a7f1a7c292b55f013cc177f6fb8a34e2ca4
                 f1a2eca9fac90e8d17b5b53fd0f20f557df8356db85fe4391e3cc77a2f4e3b237d831d0f2fd3ddd125f2bde541c6cd70635ef270fa8ebb94c23abf2546da05bb
                 52137155f8b723d19b5a217589a68bb0e4cb2f08855d3dfdd6ee731846b9e71c
```

#### `dummy_health_export`

```
ch400 reliable seq=1001 (B+1) open=0 exports=1
plain        98b 0c220000e811290000e8110100   (seed 300100)
encoded      98b cada618c0db83a640acb832b03
body        454b 020000001aa3480a12a348002800000090cac2d8e8d088c2dac2cecaa6cac6e8d2dedc00000000006a8c2249acac1dc6d880ab43a6b03cb832
decoded     590b 102d0989ffffffff0110430c227d321b07010000008d51240589512400140000004865616c746844616d61676553656374696f6e00000000003546912456d60e
                 636cc0d52153581e5c39
frame        88B 00000000100000008019771cd623f47889322f2bcf450e74f3f4658d92a0520ff5391eb170ae01b56b678fe7bec76557714f2942f2d6137a3f90dedfb90bec0c
                 6a5b89521c9dc365bd9f47d90b8be921cb2f88ccfbbdd01c
```

#### `dummy_update_damage_text_26`

RPC payload before the content transform = `plain` (79 bits).

```
ch400 reliable seq=1002 (B+2) open=0 exports=0
plain        79b b9600000000000001d20   (seed 300100)
encoded      79b 75676148ddc8f3647906
body         89b 7ad69d85217523cf93e51900
decoded     225b 102d0989ffffffff0110430c407d3267013debcec290ba91e7c9f20c01
frame        42B 00000000100000000085d615e323f47889322f2bcf855071f3f4658d52a4e6ed0bcb64945aa5d16ca9f4
```

#### `dummy_update_damage_text_78`

RPC payload before the content transform = `plain` (79 bits).

```
plain        79b b9600000000000c02920   (seed 300100)
encoded      79b 75673548ddc8f3646006
body         89b 7ad69dd5207523cf93811900
```

#### `dummy_update_damage_text_160`

RPC payload before the content transform = `plain` (79 bits).

```
plain        79b b9600000000000003220   (seed 300100)
encoded      79b 75676148ddc8f364fd06
body         89b 7ad69d85217523cf93f51b00
```

Cross-check with `test_range_bots.py:113-116`: `build_update_damage_text_payload(26.0)` = `b9600000000000001d20` / 79 bits; body for actor 300400 = `7ad29d857177a3cf938d5f00` / 89 bits.

#### Hit tests (`hit_region`, `dummy_hit`, `view_ray`)

```
hit_region(origin=(0, 0, 170), dir=(1.0, 0.0, 0.0), center=(1000.0, 0.0, 100.0)) = ('head', 978.5757147143714)
hit_region(origin=(0, 0, 120), dir=(1.0, 0.0, 0.0), center=(1000.0, 0.0, 100.0)) = ('body', 972.0)
hit_region(origin=(0, 0, 40), dir=(1.0, 0.0, 0.0), center=(1000.0, 0.0, 100.0)) = ('legs', 978.0)
hit_region(origin=(0, 0, 176), dir=(1.0, 0.0, 0.0), center=(1000.0, 0.0, 100.0)) = ('head', 978.0227390241641)
hit_region(origin=(0, 0, 3), dir=(1.0, 0.0, 0.0), center=(1000.0, 0.0, 100.0)) = ('legs', 978.0)
hit_region(origin=(0, 0, 1), dir=(1.0, 0.0, 0.0), center=(1000.0, 0.0, 100.0)) = None
hit_region(origin=(0, 100, 120), dir=(1.0, 0.0, 0.0), center=(1000.0, 0.0, 100.0)) = None
hit_region(origin=(0, 0, 120), dir=(-1.0, 0.0, 0.0), center=(1000.0, 0.0, 100.0)) = None
hit_region(origin=(1000, 0, 400), dir=(0.0, 0.0, -1.0), center=(1000.0, 0.0, 100.0)) = ('head', 203.0)
dummy_hit(dummy@(6190.0, -11640.0, 1270.0), origin=(6190.0,-11000.0,1290.0), dir=(0,-1,0)) = ('body', 612.0)
view_ray(0.0, 90.0) = (6.123233995736766e-17, 1.0, 0.0)
view_ray(90.0, 0.0) = (6.123233995736766e-17, 0.0, 1.0)
view_ray(-2.86, -130.79) = (-0.6524747597851398, -0.756166058446629, -0.04989569016070803)
view_ray(10.0, 45.0) = (0.696364240320019, 0.696364240320019, 0.17364817766693033)
```

```
first_hit(bots, origin=(4825.0, -12600.0, 1371.8), view_ray(0.0, -101.31889987841747)):
  -> bot actor 300108, region 'body', distance 1815.531641932226
```

Neumaier-sum detector (verifier addition; none of the vectors above changes with plain summation, this one does):

```
view_ray(1.7288577981699722, -101.31889987841747) = (-0.19618026054883472, -0.9801036147501181, 0.030169682211511784)
hit_region(origin=(4825.0, -12600.0, 1371.8), that dir, center=(4463.170774427629, -14407.674895086804, 1351.8))
  = ('head', 1822.3612052256326)       CPython 3.12 sum() (Neumaier)
  plain x0 + x1 + x2 in _ray_sphere gives 1822.361205225643 (wrong)
```

### V.4 Cloudburst stage A

```
view (test_range_projectiles.py:31) = ((4786.03, -13285.84, 1427.98), -2.86, -130.79)
throw_transform(view, forward=60.0):
  location = (4746.9864419503865, -13331.088360883461, 1425.352643253695)
  pitch    = 5.07644444444443
  yaw      = -130.79
  velocity = (-1887.1053057312877, -2187.004109367309, 256.6059054100816)
  spawn-header shorts (round half-even): pitch 0x039c yaw 0xa2fe
  FRotator bytes (floor(x+0.5)): pitch 4 yaw 163 roll 0
ballistic_fallback(location, velocity) = (3180.6890381934163, -15146.301771658329, 1318.8)
smoke_location_from_hit((100.0, 200.0, 1253.8), (0.0, 0.0, 1.0)) = (100.0, 200.0, 1318.8)
```

Quantized vectors (`append_quantized_vector`, sub_7FF737520200 reader) and FRepMovement:

```
quantized(scale 1) (4746.9864, -13331.0884, 1425.3526) -> 52b cf4549fb32b200
quantized(scale 1) (-1887.1053, -2187.0041, 256.6059) -> 46b cd505c770302
quantized(scale 1) (0.0, 0.0, 0.0) -> 10b 4100
quantized(scale 1) (0.5, -0.5, 1.49) -> 13b c20e
quantized(scale 10) (4746.9864, -13331.0884, 1425.3526) -> 64b 53b75c04ddd7f506
rotator_bytes(pitch, yaw, 0) -> 19b 098e02
rep_movement(location, pitch, yaw, velocity) -> 121b f05c94b42f230b098e6a86e2ba1b1000
```

Allocation for two throws (each: projectile channel + actor + movement GUID at throw, zone channel + GUID at stop):

```
throw 1: projectile ch10000 actor 610100 FiniteSpeedMovement 610102; zone ch10001 actor 610104
throw 2: projectile ch10002 actor 610106 FiniteSpeedMovement 610108; zone ch10003 actor 610110
(assumes throw 2 happens after throw 1 stopped; a second throw while the first is flying takes its channel/GUIDs before throw 1's zone)
```

#### `cloudburst_projectile_open_stageA`

Production call (`range_udp_handshake.py:1039-1041`, stage A: `movement_guid` passed, `ignore_pawn=False`). plain[0] = AActor block (ReplicatedMovement 11, Owner 12, Instigator 14), plain[1] = empty FiniteSpeedMovement block.

```
ch10000 reliable seq=1000 (B+0) open=1 exports=1
plain[0]    170b 2ce0b928695f4616121cd50cc5753720605070500000   (seed 610100)
encoded[0]  170b 13795eeeb0efe344794f321eb4d39bafc06044413401
plain[1]      9b 0000   (seed 610100)
encoded[1]    9b 5601
body       1689b 040000004677940a4e77940a007a0000005e8ec2daca5e86d0c2e4c2c6e8cae4e65eaeeae6d0ea5ea6605e82c4d2d8d2e8f2be685ea0e4ded4cac6e8d2d8cabe
                 aeeae6d0eabe68bea6daded6ca00000000004800000088caccc2ead8e8bebea0e4ded4cac6e8d2d8cabeaeeae6d0eabe68bea6daded6cabe860000000000da7a
                 940ad27a9400280000008cd2dcd2e8caa6e0cacac89adeeccadacadce80000000000d27a94467794149ebae522e8beae379873a0bf28ae1298c8f372877d1f27
                 ca7b92f1a09dde7c0506230aa2a9b61ea5125601
decoded    1825b 102d0989ffffffff01304338037d32671a02000000a33b4a05a73b4a05003d0000002f47616d652f436861726163746572732f57757368752f53302f4162696c
                 6974795f342f50726f6a656374696c655f57757368755f345f536d6f6b6500000000002400000044656661756c745f5f50726f6a656374696c655f5775736875
                 5f345f536d6f6b655f4300000000006d3d4a05693d4a001400000046696e69746553706565644d6f76656d656e740000000000693d4aa33b4a0a4fdd7211745f
                 d71bcc39d05f1457094ce479b9c3be8f13e53dc978d04e6fbe02831105d1545b8f5209ab01
frame       242B 000000001000000080c4c392cb23f47889322f2bcf055a7173d4a577cfa51278ac5017b170363035ba758f383c5c2a7f1a7c292b15e4cc0cc5386cbb4e8f247e
                 67cba7229e11a9045d7b88c08f27ad3ee3d535cf8759bd9f4b7d4b8bf23c76f581e398ab79e9262d59d3c3947b9abc3e27635d7b86b49f40bb587cc725c12077
                 970710481973d1a3966ba06a20e62828e73fafa604a2fafb4f6b1ab8e427133da186f317ab934882d588d544366e38ca46ecfbc3ac70a1d7bbc2656a63634b70
                 789e59395b75da3573668455376d9c80cf9bdf9ed8bf606a7051231de8902081119b56159884cfb8c75cb002ab8c63946aec
```

#### `cloudburst_zone_open`

Zone yaw is `random.uniform(0.0, 360.0)` from the process-global unseeded RNG in production; 123.0 is a fixed test value. Exact call (the inputs of `test_range_projectiles.py:53`, NOT the `smoke_location_from_hit` example above): `smoke_zone_open_body(610104, (5000.0, -14000.0, 1319.0), 123.0)`. With location `(100.0, 200.0, 1318.8)` the body is 1256 bits instead (narrower quantized width).

```
ch10001 reliable seq=1000 (B+0) open=1 exports=1
plain        41b 302838280000   (seed 610104)
encoded      41b f4d3a5746800
body       1268b 020000005677940a5e77940a00820000005e8ec2daca5e86d0c2e4c2c6e8cae4e65eaeeae6d0ea5ea6605e82c4d2d8d2e8f2be685e8ec2daca9ec4d4cac6e8be
                 aeeae6d0eabe68bea6daded6cab4dedcca00000000005000000088caccc2ead8e8bebe8ec2daca9ec4d4cac6e8beaeeae6d0eabe68bea6daded6cab4dedccabe
                 860000000000e27a94567794149e420d03a4bb8633e8dd1596a29f2ea54303
decoded    1404b 102d0989ffffffff01304738037d32d31301000000ab3b4a05af3b4a0500410000002f47616d652f436861726163746572732f57757368752f53302f4162696c
                 6974795f342f47616d654f626a6563745f57757368755f345f536d6f6b655a6f6e6500000000002800000044656661756c745f5f47616d654f626a6563745f57
                 757368755f345f536d6f6b655a6f6e655f430000000000713d4aab3b4a0a4fa18601d25dc319f4ee0a4bd14f97d2a109
frame       190B 00000000100000004001fc28c423f47889322f2bcf051a4073d4a5b7b6941278ec393eb1703630b5237e8fb80f5c2a7f1a7c292b15e4cc0cc5386cbb4e8f247e
                 67cba7229e11a9045d7b88c08f27ed8de6d535cfc725faa54ecc2244d83c76f581e398ab79e926bb72d3c3947bdaa337676ee9cd37458c0edca7c4ae134a47cb
                 4c12680141048d02e7cd9d8dca40102c094b122337f4029acc4a1c840446201ba13305efbb017cb751e4512fc60abd10836ffb63d3225b877d84eaa72707
```

#### `cloudburst_projectile_close`

```
ch10000 reliable seq=1001 (B+1) close=1 reason=0 exports=0 body 0 bits
decoded     140b 102d0989ffffffff0150308413d227330008
frame        32B 0000000010000000008aeca8fe23f47889322f2bcf122005f0d0e5ff8fa72107
```

#### `cloudburst_zone_close`

```
ch10001 reliable seq=1001 (B+1) close=1 reason=0 exports=0 body 0 bits
decoded     140b 102d0989ffffffff0150708413d227330008
frame        32B 000000001000000080113430e423f47889322f2bcf122005f0d0e5bf9ba72107
```

Builder sizes from `test_range_projectiles.py:44-56` (stage A open without movement = 1357, with = 1689, stage B ignore-pawn = 1737, zone = 1268, bending 29, teleport 112, stop RPC 22) are reproduced by this generator: 1357, 1689, 1737, 1268.

### V.5 Team builders (bodies; framed versions are in `04_bringup_b.md` section 11)

#### `team_state_open_100`

```
body        573b 02000000940b9c0b00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000002e00000088caccc2ead8e8bebe84c2e6caa8cac2daa6e8c2e8ca00
                 0000000090951500
```

#### `team_state_open_102`

```
body        573b 02000000940b9c0b00280000005ea6c6e4d2e0e85ea6d0dedee8cae48ec2daca00000000002e00000088caccc2ead8e8bebe84c2e6caa8cac2daa6e8c2e8ca00
                 0000000098951500
```

#### `team_states_array_h34`

```
plain        73b 88040004900998010000   (seed 2)
encoded      73b 412f660475d6b9610e00
body         83b 4b06bd9811d459e7863900
```

#### `team_link_blue`

```
plain        47b 40a045421200   (seed 100)
encoded      47b e6e59da5661a
body        258b 02000000a00b08001200000084d8eacaa8cac2da0000000000f6322fef2c35d300
```

#### `team_link_red`

```
plain        47b 40a845442200   (seed 102)
encoded      47b c6e77f535e65
body        250b 02000000a80b080010000000a4cac8a8cac2da0000000000f6323eff9bf22a03
```

#### `player_state_team_h27`

```
plain        25b 6c900100   (seed 6)
encoded      25b 755fcf00
body         35b cbd47d3d03
```

#### `bomb_team_open_inline`

```
ch6 reliable seq=1006 (B+6) open=0 exports=1
plain         9b 0000   (seed 6)
encoded       9b 5300
body        229b 02000000b00b18001200000084dedac4a8cac2da0000000000c22e3105
decoded     357b 102d0989ffffffff011019c27d32970301000000d8050c0009000000426f6d625465616d000000000061979812
frame        59B 0000000010000000805799fce023f47889322f2bcfa1c001fcf425917295f7195afa24b170a0e9366cc23dd8b09bce31fcf6eb4e552389c3e9410e
```

#### `bomb_team_number_inline`

```
ch6 reliable seq=1007 (B+7) open=0 exports=0
plain        25b 0c020000   (seed 6)
encoded      25b 60dbce00
body         44b 619701db7606
decoded     172b 102d0989ffffffff011019e07d32b380b0cb806d3b0b
frame        36B 0000000010000000c0870c19df23f47889322f2bcf21dfaed0f4a58e7294d29107000c07
```

`range_bomb_team_open.build_bomb_team_open_bunch(108, ...)` and `build_bomb_team_number_update(108, 1, 3, ...)` with the same frame parameters produce frames byte-identical to the inline production bodies above (asserted).

Bot BombTeam rep plain (Team = 0): 25 bits `0c000000` (matches `test_range_bots.py:96`).

Self-checks passed: 23 frames decrypted (AES-GCM nonce u32 0, u32 0x7908CBDD, 4-byte tag) and inverse-transformed back to `decoded`; 42 encoded blocks decoded back to `plain` with `range_client_input.decode_content_payload`.

Independent re-generation (verifier, 2026-10-14): every `plain`/`encoded`/`body`/`decoded`/`frame` hex and bit count in this appendix (47 named vectors), the MT19937 outputs, the allocation table, the spawn transforms, the hit/view-ray `repr` values, the throw/fallback values, the quantized/rotator/FRepMovement vectors and the builder sizes were rebuilt from the Python builders with a separate script and match byte for byte, after supplying the zone-open input given in V.4. The `range_bomb_team_open` helper frames equal the inline frames. The 24 module unit tests pass (`python -m unittest test_range_abilities test_range_bots test_range_projectiles test_range_bomb_team_open`).

## Verifier corrections

1. V.4 `cloudburst_zone_open`: the input location was missing, and the nearby `smoke_location_from_hit` example `(100.0, 200.0, 1318.8)` gives 1256 bits, not the vector's 1268. The exact call is now given: `smoke_zone_open_body(610104, (5000.0, -14000.0, 1319.0), 123.0)` (`test_range_projectiles.py:53`).
2. V.1: added the exact calls for `tuning_one_float` and `tuning_float_int`, and for `tuning_bool_export` (tag 7, `True`, `export=True`, actor 212, component 190006), whose inputs were not stated.
3. Section 4.4 hit geometry: `_ray_sphere` builds `b` and `|oc|^2` with Python `sum()`. On CPython 3.12 (the responder's interpreter) that is Neumaier-compensated summation, and plain addition differs in about 45% of realistic head-sphere hits (offline sample). Added the exact algorithm, a 7.4 checklist entry and a V.3 detector vector (`1822.3612052256326`; plain addition gives `...643`). Also stated the cylinder's `disc < 0` miss and its root order, which were missing.
4. Fixed wrong cross-references: the V.1 ability frames pointed to "section 4.4" (should be 2.5) and the tag position to "section 4.2" (should be 2.3).
5. V.1 allocator note: GUID 100016 is the first GUID handed out at registration. It goes to an extra machine if the first job to register has one, not always to a state.
6. Section 1.1: the mirror ready list now states each entry's gate (BasePistol, Melee, abilities only once `starting_equippable_index >= 7`, purchases).
7. Section 1.3: added the single process-wide `EquippableStateReader` (`pistol_mirror_reader`) and its never-invalidated `_names` / `_class_names` caches. An empty state list cached on the first read makes an ability register with only `InactiveState`.
8. Section 3.4: the claim that the 898 tags are "built from" specific ini/DataTable files was unsourced. It is now marked as unverified, with the candidate files cited from the client manifest. 898 and NetIndexFirstBitSegment 16 are marked as coming from session notes.
9. Section 6.2: `subobj_guid` is a required parameter of the draft helper, not a default of 108. The helper's other defaults (`ack_history_mask 7`, sequences 0) are not production values.
10. Section 4.5: the 1197-bit bot-open size also needs a non-zero yaw short (yaw 0 gives 1178 bits, per `test_range_bots.py:83-89`).
11. V.3 live cross-check: `responder_20928` holds several appended sessions, and only 2 use the current dummy/stride-8 code. Added the line numbers that confirm ch406 opens before the first retirement.
12. Section 6.3: the ClientUpdateVisibility field 123/178 comes from session notes, not code. It is not sent in production.
13. Section 7.1 (128 tps): spelled out the 128 Hz loop. Each tick drains the socket and runs the pipeline once per datagram. With no datagram, nothing in this file is sent. Replaced the unsourced "about every 31 ms" with the real `recvfrom` timeout (0.25 s, `ruh:1410-1442`).
14. Section 7.2 (console): the console is now unconditional. The Python responder runs with `CREATE_NO_WINDOW` and stdout goes to a file. The binary does `GetConsoleWindow` -> `FreeConsole` + `AllocConsole`, reopens `CONOUT$`, and also writes `logs/aries_server_<pid>.log`.
15. Section 2.5: added a pointer to where the ability charge resources are specified (GUIDs 310/312/314, MaxCharges +0x188, 4.0 s refill to full, 05/07). They share the ch14-16 counters.
