# 07 - Gameplay: equippables, guns, shop, cosmetics

Source of truth (all under `local_backend/`). Abbreviations used in references:

| abbr | file | role |
|---|---|---|
| `req` | `range_equippables.py` | equippable model, AuthCurrentState / AuthResourceAmount / shield builders, mirror policy |
| `rgl` | `range_gun_loader.py` | PurchaseAllocator, GunLoad/ItemLoad, gun setup bodies, skin selection for purchases |
| `rst` | `range_store.py` | store table, class-path helpers |
| `rat` | `range_attachments.py` | forced attachment table, array property writers, attachment pre-export |
| `rwc` | `range_weapon_cosmetics.py` | cosmetic actor-open builders, FGuid words, static export entry |
| `ec` | `equipped_cosmetic.py` | saved selection -> catalog CDO paths |
| `si` | `skin_inventory.py` | catalog + `offline_skin_state.json` (shared with the platform backend) |
| `rme` | `range_melee_equippable.py` | knife role / state machine / state bodies and state GUID table |
| `rmc` | `range_melee_cosmetics.py` | knife cosmetic open and saved-knife selection |
| `ruh` | `range_udp_handshake.py` | the consumers: purchase job, mirror block, registration helpers |
| `rsb` | `range_server_bunch.py` | shared primitives (BitWriter, content transform, owner / slot / correction blocks) |
| `res` | `read_equippable_state.py` | live-memory reads the mirror and purchases depend on |
| `rcr` | `range_client_rpc.py` | ServerPurchase decode (detail in `06_client_decode.md`) |
| `rab` | `range_abilities.py` | reuses the PurchaseAllocator class and the mirror (detail in the abilities doc) |

Every number below is copied from code, with its file:line. The test vectors in appendix X were produced by importing the builders offline (`python -B`, `PYTHONDONTWRITEBYTECODE=1`, no socket, no live process). Skin data was read from scratch copies of `offline_skin_state.json` and `skin_catalog.json` (taken 2026-10-14), so nothing in `local_backend` was written. The existing unit tests for this area pass on the current tree: `test_range_equippables`, `test_range_gun_loader`, `test_range_attachments`, `test_range_weapon_cosmetics`, `test_range_melee_equippable`, `test_range_melee_cosmetics`, `test_equipped_cosmetic`, `test_cosmetic_cdo_paths` (61 tests, OK). `test_skin_inventory` was not run, because it constructs a `backend.Backend` (TLS listener).

Independent check of the builders against live data: the bit counts of nine skinned gun opens logged by the live responder (`runtime/responder_20928_phase4_match_state.out`, event `purchase_opened`) are reproduced exactly by the offline builder with the saved level and chroma (appendix X.9, 9 of 9 match).

## 0. Scope and neighbouring documents

- `03_bringup_a.md`: the fixed ch9 BasePistol chain A21-A27 (open, Slot1.Contents, Owner, RemoteRole, state machine, InactiveState, five inventory corrections) and the initial-cosmetic decision at A21 (its section 8.1). Section 7 below repeats only what this area needs.
- `04_bringup_b.md`: B17 shop switches, B18 starting equippables (including the knife cosmetic open), B19-B21 knife role / state machine / InactiveState. It calls the per-packet authority work "part C".
- `05_authority_loop.md`: the per-datagram authority block ("part C") as a loop: trigger, order, `send_reliable`, and each feature including the mirror (its 6.3) and purchases (its 6.9). Its purchase vectors (Vandal open / owner / role / state machine / InactiveState / slot / correction, EquipState 562, MagazineAmmo 25) are byte-identical to the ones in appendix X here.
- `06_client_decode.md`: ServerPurchase decode (`rcr`) and `resolve_class_path` (its section 6).
- This document: the gameplay modules themselves (data tables, builders, GUID rules, mirror policy, skin selection) and the two part-C consumers that drive them: the equippable mirror (`ruh:1600-1711`) and the purchase pipeline (`ruh:1847-1907` + `advance_purchase_job` `ruh:1172-1314`).

## 1. Production path and dead paths

Production flags that reach this area (`launch_range.py:29-43`, `launch.ps1:166-192`): `pistol_state_mirror`, `store_purchases`, `range_shop_probe`, `starting_equippables_probe`, `inactive_state_link_probe`, `auth_correct_rep_probe`, `input_correction_responder` (reads `purchased_class_guids`), `ability_activation` (reuses the allocator class and the mirror), `range_bots` (shot hook inside the mirror), `ability_projectiles` (Cloudburst hook inside the mirror), and the BasePistol initial-cosmetic trial.

The initial-cosmetic trial reaches `run_probe` through an environment variable: `launch.ps1:189-192` appends `--base-pistol-initial-cosmetic-trial` unless `-skip_saved_pistol_cosmetic` or `-base_pistol_skin_trial` is given. `launch_range.py:286-287` forwards it. `ruh main()` sets `os.environ["RANGE_BASE_PISTOL_INITIAL_COSMETIC_TRIAL"] = "1"` (`ruh:4908-4909`). `run_probe` reads it at `ruh:737-739`. `ruh:745-758` then fixes catalog `skin_catalog.json`, state `offline_skin_state.json` and player `7c9f41a4-65e2-4d81-a8d5-f879c3e5ab61` (`backend.py:36`). Port it as an always-on config switch.

Dead in this area (do not port):

- `rwc.build_base_pistol_cosmetic_update` (`rwc:91-130`): the late skin trial, env `RANGE_BASE_PISTOL_SKIN_TRIAL` (`ruh:736, 3260-3289`). Off.
- `rwc.primary_asset_object_path` (`rwc:65-88`): used only by the late trial and by `build_base_pistol_initial_cosmetic_open` when `cdo_paths is None`. Production always passes catalog paths (`ruh:2989-3000`).
- `rgl.gun_setup_bodies` (`rgl:308-316`), `rme.build_melee_initialization_bodies` (`rme:142-152`), `rsb.build_pistol_auth_state_body` (`rsb:771-800`): test-only helpers. The responder calls the individual builders. The generic `req.build_auth_state_body` is byte-identical to the pistol helper (X.11).
- `RANGE_EQUIP_STATE_TRIAL` (`ruh:865-866, 4335-4363`): legacy EquipState push on ch9. Off.
- `ruh:881-891` `pistol_mirror_polled/_machine/_auth/_exported/_active`, `melee_mirror_*`: declared, never used. Only `pistol_mirror_reader` (the shared live-memory reader holder) is used.
- `acr_layout_trial` (`ruh:30-93, 725-734, 2984-2985`) and `RANGE_ACR_ACTOR_INSPECTION_PROBE` (`range_acr_actor.py`, `ruh:4192-4220`): off.
- `ServerRemove` / `ServerSell` (fields 157 / 161): decoded by `rcr` but ignored, because only field 156 is acted on (`ruh:1871`).

## 2. Data tables (become data files in Rust)

| table | source | contents | proposed file |
|---|---|---|---|
| Store items | `rst:25-49` | 23 rows: display, class, cost, weapon UUID, EAresItemSlot, is_gun. Slots: primary 0, secondary 1, armor 11 (`rst:12`). | `data/store_items.json` |
| Forced attachments | `rat:38-97` | 13 gun classes -> ordered list of (package path, CDO name, 4 x u32 UUID words). Order = live TSet order and must be kept. | `data/attachments.json` |
| Armor shield values | `req:157` | LightArmorItem_C 25.0, HeavyArmorItem_C 50.0, PlasmaArmorItem_C 25.0; fallback 25.0 (`ruh:1203`) | `data/armor.json` |
| BasePistol state GUIDs | `rsb:755-767` | 9 names -> GUIDs (98, 99, 240..252 even) | `data/equippables.json` |
| Knife state GUIDs | `rme:23-56` | 25 names, fixed order -> 256, 258, 260 + 2i | `data/equippables.json` |
| Resource subobjects | `req:319-327, 252-275`, `rgl:238-239` | name, NetGUID, item offset, default offset, refill delay | `data/equippables.json` |
| Rep handles | section 3.1 | every FRepLayout handle and width this area writes | `data/rep_handles.json` |
| Cosmetic GUID pairs | `rwc:40-44`, `rmc:34` | pistol 229-239, knife 241-251 | `data/equippables.json` |
| Gun damage (bots) | `range_bots.py:71-83` | (head, body, leg) per gun class, default (100, 35, 30). Used by the mirror's shot hook. Owned by the bots doc. | `data/weapon_damage.json` |
| Skin catalog | `skin_catalog.json` (3.2 MB, 1415 skins) | shared with the platform backend; read-only for the game server | keep the JSON |
| Saved skins | `offline_skin_state.json` | shared with the platform backend (writer); read-only for the game server | keep the JSON |
| Live-memory offsets | `res:1-10, 132-236` | offsets the oracle reads (section 8.4) | oracle config |

### 2.1 Store (`rst:25-49`)

| display | class | cost | weapon_id | slot | gun |
|---|---|---|---|---|---|
| Classic | BasePistol_C | 0 | 29a0cfab-485b-f5d5-779a-b59f85e204a8 | 1 | 1 |
| Shorty | SawedOffShotgun_C | 300 | 42da8ccc-40d5-affc-beec-15aa47b42eda | 1 | 1 |
| Frenzy | AutomaticPistol_C | 450 | 44d4e95c-4157-0037-81b2-17841bf2e8e3 | 1 | 1 |
| Ghost | LugerPistol_C | 500 | 1baa85b4-4c70-1284-64bb-6481dfc3bb4e | 1 | 1 |
| Bandit | CompactPistol_C | 600 | 410b2e0b-4ceb-1321-1727-20858f7f3477 | 1 | 1 |
| Sheriff | RevolverPistol_C | 800 | e336c6b8-418d-9340-d77f-7a9e4cfe0702 | 1 | 1 |
| Stinger | Vector_C | 1100 | f7e1b454-4ad4-1063-ec0a-159e56b58941 | 0 | 1 |
| Spectre | SubMachineGun_MP5_C | 1600 | 462080d1-4035-2937-7c09-27aa2a5c27a7 | 0 | 1 |
| Bucky | PumpShotgun_C | 850 | 910be174-449b-c412-ab22-d0873436b21b | 0 | 1 |
| Judge | AutomaticShotgun_C | 1850 | ec845bf4-4f79-ddda-a3da-0db3774b2794 | 0 | 1 |
| Bulldog | AssaultRifle_Burst_C | 2050 | ae3de142-4d85-2547-dd26-4e90bed35cf7 | 0 | 1 |
| Guardian | DMR_C | 2250 | 4ade7faa-4cf1-8376-95ef-39884480959b | 0 | 1 |
| Phantom | AssaultRifle_ACR_C | 2900 | ee8e8d15-496b-07ac-e5f6-8fae5d4c7b1a | 0 | 1 |
| Vandal | AssaultRifle_AK_C | 2900 | 9c82e19d-4575-0200-1a81-3eacf00cf872 | 0 | 1 |
| Warden | BattleRifle_C | 2900 | 8db0a1bf-4a50-832a-4566-faaaa6d250ca | 0 | 1 |
| Marshal | LeverSniperRifle_C | 950 | c4883e50-4494-202c-3ec3-6b8a9284f00b | 0 | 1 |
| Outlaw | DS_Gun_C | 2400 | 5f0aaf7a-4289-3998-d5ff-eb9a5cf7ef5c | 0 | 1 |
| Operator | BoltSniper_C | 4700 | a03b24d3-4319-996d-0f8c-94bbfba1dfc7 | 0 | 1 |
| Ares | LightMachineGun_C | 1600 | 55d8a0f4-4274-ca67-fe2c-06ab45efdf58 | 0 | 1 |
| Odin | HeavyMachineGun_C | 3200 | 63e6c2b6-4a8e-869c-3d4c-e38355226584 | 0 | 1 |
| Light Shields | LightArmorItem_C | 400 | 4dec83d5-4902-9ab3-bed6-a7a390761157 | 11 | 0 |
| Heavy Shields | HeavyArmorItem_C | 1000 | 822bcab2-40a2-324e-c137-e09195ad7692 | 11 | 0 |
| Regen Shield | PlasmaArmorItem_C | 650 | b1b9086d-41bd-a516-5d29-e3b34a6f1644 | 11 | 0 |

`store_items` is keyed by class name (`rst:50`). Cost is never read by the server (infinite money is a GameState switch, B17 in 04). The class paths the live client actually sent are listed in X.1.

### 2.2 Forced attachments (`rat:38-97`)

Each row: package path under `/Game/Equippables/Guns/`, CDO name, UUID words (FGuid A, B, C, D as u32).

| class | # | package (relative to `/Game/Equippables/Guns/`) | CDO | words |
|---|---|---|---|---|
| AssaultRifle_ACR_C | 1 | `Rifles/Carbine/Attachments/Reflex/RedDot/Attachment_ACR_Reflex_RedDotPrimaryAsset` | `Default__Attachment_ACR_Reflex_RedDotPrimaryAsset_C` | c119da22 419d5a7d 2e83ce9c 814719e5 |
| AssaultRifle_ACR_C | 2 | `Rifles/Carbine/Attachments/Muzzle/Silencer/Attachment_ACR_Silencer_PrimaryAsset` | `Default__Attachment_ACR_Silencer_PrimaryAsset_C` | f1278ba5 45c7e3bc b2393d9e e61b989d |
| SubMachineGun_MP5_C | 1 | `SubMachineGuns/MP5/Attachments/Reflex/RedDot/Attachment_MP5_Reflex_RedDotPrimaryAsset` | `Default__Attachment_MP5_Reflex_RedDotPrimaryAsset_C` | c59421d8 d7dc4e75 8fb99071 0386599e |
| SubMachineGun_MP5_C | 2 | `SubMachineGuns/MP5/Attachments/Muzzle/Silencer/Attachment_MP5_Silencer_PrimaryAsset` | `Default__Attachment_MP5_Silencer_PrimaryAsset_C` | a711565a e57c43c8 9395e7d0 9229ff26 |
| AssaultRifle_AK_C | 1 | `Rifles/AK/Attachments/Reflex/RedDot/Attachment_AK_ReflexPrimaryAsset` | `Default__Attachment_AK_ReflexPrimaryAsset_C` | d90ff390 4d4dee48 8d4cbc9f 7b123692 |
| AssaultRifle_Burst_C | 1 | `Rifles/Burst/Attachments/Rfx/RedDot/Attachment_Burst_Reflex_RedDotPrimaryAsset` | `Default__Attachment_Burst_Reflex_RedDotPrimaryAsset_C` | 8c505c5f 4b9e7338 a54f24ab a13b2b6b |
| DMR_C | 1 | `SniperRifles/DMR/Attachments/Reflex/ACOG/Attachment_DMR_Reflex_ReddotPrimaryAsset` | `Default__Attachment_DMR_Reflex_ReddotPrimaryAsset_C` | 2cabd857 439b33b4 cef703a8 137ed1e4 |
| BattleRifle_C | 1 | `Rifles/BattleRifle/Attachments/Reflex/Acog/Attachment_BattleRifle_ACOGScope_PrimaryAsset` | `Default__Attachment_BattleRifle_ACOGScope_PrimaryAsset_C` | 8ff81891 4c1acfbc 82b201ba 37250b8f |
| Vector_C | 1 | `SubMachineGuns/Vector/Attachments/Reflex/RedDot/Attachment_Vector_Reflex_RedDotPrimaryAsset` | `Default__Attachment_Vector_Reflex_RedDotPrimaryAsset_C` | d1d44263 4192cebc 0974d48e f2f15e1d |
| LightMachineGun_C | 1 | `HvyMachineGuns/LMG/Attachments/Reflex/RedDot/Attachment_LMG_Reflex_RedDotPrimaryAsset` | `Default__Attachment_LMG_Reflex_RedDotPrimaryAsset_C` | db9e14fd 4c573fcc 38170bac fa8b518c |
| HeavyMachineGun_C | 1 | `HvyMachineGuns/HMG/Attachments/Reflex/Attachment_HMG_ReflexPrimaryAsset` | `Default__Attachment_HMG_ReflexPrimaryAsset_C` | c1ca73bb 4dcff8c0 e6aacf8a 51eafc97 |
| LeverSniperRifle_C | 1 | `SniperRifles/Leversniper/Attachments/Reflex/Default/Scope_Lever_DataAsset` | `Default__Scope_Lever_DataAsset_C` | 03e545a4 4e1998be 421cedba 218fda66 |
| DS_Gun_C | 1 | `SniperRifles/Doublesniper/Attachments/Scope/Scope_DS_DataAsset` | `Default__Scope_DS_DataAsset_C` | 73dd60ef 4b02a29d 2c33659d 3da022e4 |
| BoltSniper_C | 1 | `SniperRifles/Boltsniper/Attachments/2XScope/Attachment_BoltSniper_2XScope_PrimaryAsset` | `Default__Attachment_BoltSniper_2XScope_PrimaryAsset_C` | e27e029d 47e6ef77 a21da4ba 46737070 |
| LugerPistol_C | 1 | `Sidearms/Luger/Attachments/Silencer/Attachment_Luger_Silencer_PrimaryAsset` | `Default__Attachment_Luger_Silencer_PrimaryAsset_C` | 106e210f 7f8f408b 801081c1 5ab50032 |

No forced attachments: BasePistol_C, SawedOffShotgun_C, AutomaticPistol_C, CompactPistol_C, RevolverPistol_C, PumpShotgun_C, AutomaticShotgun_C (`rat:98-99`). Why it matters: the client builds sights and silencers in AAresEquippable BeginPlay (IDA `sub_7FF73B7D85A0`) from handles 20 and 40. A null or unresolved element crashes the client (no null check at `0x7ff73b7d8b24`), so every attachment CDO is exported (on ch8) before the open that references it (`rat:1-19`).

### 2.3 State GUID tables

BasePistol (`rsb:755-767`), outer = actor 94, machine 96:

| state | GUID |
|---|---|
| InactiveState | 98 |
| EquipState | 99 (odd; the proven legacy export) |
| IdleWaitingForEventState | 240 |
| IdleState | 242 |
| FiringState | 244 |
| ReloadState | 246 |
| ReadyingState | 248 |
| JammedState | 250 |
| FiringStateBurst | 252 |

Knife (`rme:26-56`), outer = actor 204, machine 206 (`rme:23`, = actor + 2): InactiveState 256, EquipState 258, then `260 + 2*i` for i over this fixed order: ReadyingState, IdleWaitingForEventState, RespondToEventState, FirstAttackIdle, AnimTriggeredState1, MeleeAttackState1, WaitForComboInput1, MeleeAttackState2, WaitForComboInput2, AnimTriggeredState2, WaitForComboInput3, AnimTriggeredState3, MeleeAttackState3, AnimTriggeredState4, MeleeAttackState4, WaitForComboInput4, AnimTriggeredState_Alt, MeleeAttackState_Alt, WaitForAltAttack, GhostEquipState, AnimTriggeredState5, MeleeAttackState5, WaitForComboInput5 (ReadyingState 260 ... WaitForComboInput5 304).

Purchased guns and abilities have no table: their state names are read from the live state machine at registration time (section 6.6) and get allocator GUIDs.

### 2.4 Resource subobjects

| equippable | resource (default-subobject name) | NetGUID | item offset | default offset | refill | source |
|---|---|---|---|---|---|---|
| BasePistol (94, ch9) | MagazineAmmo | 308 | 0x1208 | 0x12C | none | `req:326` |
| BasePistol | ReserveAmmo | 306 | 0x1200 | 0x12C | none | `req:327` |
| purchased gun | MagazineAmmo | `load.magazine_guid` | 0x1208 | 0x12C | none | `rgl:238` |
| purchased gun | ReserveAmmo | `load.reserve_guid` | 0x1200 | 0x12C | none | `rgl:239` |
| Ability_Wushu_4_Smoke_C (208, ch14, slot 3) | EquipmentCharge | 310 | 0x1240 | 0x188 | 4.0 s | `req:272` |
| Ability_Wushu_Q_CycloneBoost_C (216, ch16, slot 4) | EquipmentCharge | 312 | 0x12A0 | 0x188 | 4.0 s | `req:273` |
| Ability_Wushu_E_Dash_C (212, ch15, slot 5) | SignatureAbilityResource | 314 | 0x1290 | 0x188 | 4.0 s | `req:274` |

Order inside each equippable is the send order: Magazine before Reserve. The ability order in the ready list is smoke (2), Q (4), E (3) (`req:271-275`). Default offsets: `UAmmoComponent.Ammo` +0x12C, `MaxCharges` +0x188 (`req:36, 255-257`). The default amounts are read from the client (section 8.4). Observed live first-send (export) values, magazine / reserve, from `sent_equippable_auth_states.resources` in `runtime/responder_20928_phase4_match_state.out`: Classic (starting) 12 / 36, Shorty 2 / 6, Frenzy 15 / 45, Ghost 13 / 39, Bandit 8 / 24, Sheriff 6 / 24, Guardian 12 / 36, Phantom 30 / 60, Vandal 25 / 50, Warden 18 / 36, Marshal 5 / 15, Outlaw 2 / 10, Operator 5 / 10; charges Cloudburst 2, Updraft 1, Tailwind 1. Never logged: Stinger, Spectre, Bucky, Judge, Bulldog, Ares, Odin (Odin has a `purchase_opened` event but no `purchase_ready` in that log).

## 3. Shared wire primitives

LSB-first `BitWriter` (`rsb:16-75`): packed uint = 7 bits per byte with the continuation flag in bit 0; FString = u32 (len+1), ASCII, NUL. `[p: x]` = packed uint, `[n: x]` = n raw bits, `enc(seed)` = `encode_short_actor_rep_payload(plain, bits, 0, seed)` (`rsb:1131-1216`, content transform, covered in 03/06). The seed is always the channel actor's GUID, including for subobjects on that channel.

### 3.1 Handles and widths written by this area

| object | handle | field | wire value | source |
|---|---|---|---|---|
| AAresEquippable (actor) | 4 | RemoteRole | 3 bits, value 2 | `rsb:1450-1465`, `rgl:194-203`, `rme:60-74` |
| AAresEquippable | 12 | Owner (pawn) | packed GUID 10 | `rsb:697, 721-730` |
| AAresEquippable | 18 / 19 / 21 | Skin / Level / Chroma data-asset pointers | packed GUID | `rwc:33` |
| AAresEquippable | 20 | AttachmentDataAssets (+0xFC0, TArray<obj>) | array, see 3.3 | `rat:26` |
| AAresEquippable | 24-27 / 28-31 / 41-44 | Skin / Level / Chroma FGuid leaves | 4 x raw u32 each | `rwc:34` |
| AAresEquippable | 40 | AttachmentDataAssetIds (+0x1028, TArray<FGuid>) | array of 4 u32 leaves | `rat:27` |
| UEquippableStateMachineComponent | 3 | AuthCurrentState.CurrentState | packed state GUID | `req:19`, `rme:57` |
| UResourceComponent (ammo / charges) | 3 | AuthResourceAmount (+0x110) | raw 32 bits (int32 >= 0) | `req:40, 22-32` |
| UChildDamageSectionComponent "ShieldDamageSection" | 3 / 4 / 5 | bAlive / Life / MaximumLife | 1 bit / f32 bits / f32 bits | `req:153-156` |
| UItemSlot (dynamic subobject, class 61, outer 60) | 1 | Contents | packed item GUID | `rsb:698, 733-749` |
| UAresInventory 60 (stable) | 25 / 26 | CurrentEquippable / CorrectionIndex | packed GUID / raw u32 | `rsb:813-839` |

Handle order inside one payload must strictly ascend (`rwc:324-325`); array element handles count on across elements (`rat:120-129`).

### 3.2 Export shapes

Stable child export (`req:76-85`; identical copy in `rme:77-86` with the outer fixed to 204):
`[1: 0][32: 1][p: object][8: 5][p: outer][8: 0] FString(name) [32: 0]`

Static export entry (`rwc:133-144`), one per exported CDO; a list of them follows one `[1: 0][32: count]` header:
`[p: object][8: 5][p: package][8: 5][p: 0] FString(package path) [32: 0] FString(object name) [32: 0]`

`rsb.build_dynamic_actor_reference` (`rsb:141-168`) is `[1: 0][32: 1]` + one such entry (archetype, package) + `[p: actor][p: archetype][p: level 5][4: 0]` (no spawn location).

### 3.3 Array properties (`rat:109-129`)

Object array: `[p: handle][16: count]` then per element (1-based i) `[p: i][p: guid]`, then `[p: 0]` (array terminator). The outer `[p: 0]` property terminator is the caller's. `rsb.build_object_array_property_payload` (`rsb:1300-1322`) = `[1: 0]` + this + `[p: 0]` (X.11 check is in `test_range_attachments.py:127-138`).

FGuid array: `[p: handle][16: count]`, then for element i (0-based) and leaf j in 0..3: `[p: 4i+j+1][32: word j]`, then `[p: 0]`.

## 4. `range_equippables` builders

### 4.1 Model (`req:22-73, 278-284`)

- `ResourceSubobject(name, guid, item_offset, default_offset=0x12C, refill_after=None)`.
- `ExtraStateMachine(name, guid, starting_state, state_guids)`: a further machine in the item's machine array (item+0xE30), for example a rifle's ZoomStateMachine for right-click ADS. Without an authoritative state the client resets it to no state (`req:43-51`).
- `EquippableActor(key, class_name, channel, actor_guid, slot, state_machine_guid, state_guids, resources=(), extra_machines=(), mirror_states=True)`. Validation (`req:67-73`): `actor_guid > 0` and even, `0 <= slot < 16`, `"InactiveState" in state_guids`, else ValueError.
- `MirrorState(auth="InactiveState", exported={"InactiveState"}, active=False, kicked=False)` per (connection, key).
- `ResourceMirrorState(auth=None, baseline=None, sent_at=0.0)` per (connection, key, resource name).

### 4.2 AuthCurrentState (`req:88-116`)

`build_auth_state_body(eq, state, export)` = `_auth_state_body(eq.actor_guid, eq.state_machine_guid, eq.state_guids[state], state, export)`:

```
[export ? stable child export(state_guid, outer=actor_guid, state_name) : nothing]
[1: 1 bHasRepLayout][1: 0 bIsActor][p: machine_guid][1: 1 bStablyNamed]
[p: rep_bits][enc(actor_guid) of rep]
rep = [1: 0][p: 3][p: state_guid][p: 0]
```

`build_machine_state_body(eq, extra, state, export)` (`req:137-141`) is the same with `extra.guid` and `extra.state_guids`. The bunch must carry `has_exports` exactly when `export` is true.

### 4.3 State-machine subobject open (`req:119-134`)

`build_machine_open_body(actor_guid, machine_guid, name)`: stable child export of `(machine_guid, outer actor_guid, name)`, `[1: 1][1: 0][p: machine_guid][1: 1][p: 9][enc(actor_guid) of [1: 0][p: 0]]`. It creates the client's FObjectReplicator. It is used for extra machines (name from live memory), abilities (`rab:58-59`) and, byte-identically, by `rgl.gun_state_machine_open_body` and `rme.build_melee_state_machine_open` (X.11).

### 4.4 AuthResourceAmount (`req:187-207`)

```
[export ? stable child export(resource.guid, outer=eq.actor_guid, resource.name) : nothing]
[1: 1][1: 0][p: resource.guid][1: 1][p: 49][enc(eq.actor_guid) of rep]
rep = [1: 0][p: 3][32: amount][p: 0]     (49 bits)
```

`0 <= amount < 2^31`, else ValueError.

### 4.5 ShieldDamageSection (`req:160-184`)

Pawn channel 8, seed 10, section GUID 316 (`req:152`), outer pawn 10:

```
[export ? stable child export(316, outer=10, "ShieldDamageSection") : nothing]
[1: 1][1: 0][p: 316][1: 1][p: rep_bits][enc(10) of rep]
rep = [1: 0][p: 3][1: 1][p: 4][32: f32(life)][p: 5][32: f32(maximum)][p: 0]
```

`0.0 <= life <= maximum <= 1000.0`, else ValueError. Production always sends `life == maximum == armor_shield_values[class]`.

## 5. Mirror policy

The client predicts state-machine transitions locally. A real server confirms each one by replicating `UStateMachineComponent.AuthCurrentState.CurrentState` (handle 3). Without confirmation the client's disagreement timer rolls the prediction back (IDA `sub_7FF73B65C9E0`, `sub_7FF73C3AB9B0`; `req:1-12`). Ammo works the same way: AuthResourceAmount starts at 0 on the client, so an unconfirmed gun has an empty magazine and never enters FiringState (`req:26-29`).

### 5.1 `mirror_target(eq, state, predicted, is_current)` (`req:287-309`)

In order:

1. `predicted is None` or not in `eq.state_guids` -> None.
2. `predicted != "InactiveState"` -> `state.active = True`.
3. `is_current and not state.active and not state.kicked and "EquipState" in eq.state_guids` -> `state.kicked = True`, return `"EquipState"`. A server correction can make an item current while its machine still predicts Inactive; push EquipState once so it equips.
4. `predicted == state.auth` -> None.
5. `predicted == "InactiveState" and not state.active` -> None. A stale Inactive right after the push must not undo it.
6. Return `predicted`.

The caller then sets `export = target not in state.exported`, sends `build_auth_state_body(eq, target, export)` on `eq.channel` (reliable, `has_exports = export`), adds target to `exported`, sets `auth = target` (`ruh:1698-1709`). `kicked` is never reset. Traces: X.12.

### 5.2 `machine_mirror_target(state, predicted, state_guids)` (`req:144-148`)

`None` if predicted is None, unknown, or equal to `state.auth`; else predicted. No kick and no Inactive rule. The MirrorState for an extra machine is created by `send_extra_machines` as `auth = starting_state`, `exported = {starting_state}`, `active=True` (`ruh:1127-1128`). The mirror's `setdefault` fallback (`ruh:1682-1685`) would create it with `active=False`, but registration always runs first and `machine_mirror_target` never reads `active`, so the flag has no effect on the wire.

### 5.3 `resource_target(state, predicted, authoritative, default_amount, now, refill_after)` (`req:217-242`)

1. `state.auth is None` (never sent) -> `default_amount` if `0 <= default_amount < 10000`, else None (keeps retrying every poll).
2. `authoritative != state.auth` -> None. The client has not applied our last value yet; never overwrite it.
3. `state.baseline is None` -> `baseline = predicted`, return None. The prediction seen when our value lands is the baseline.
4. `predicted != baseline and predicted != state.auth and 0 <= predicted < 10000` -> predicted (a shot or a reload).
5. `refill_after is not None and state.auth < default_amount < 10000 and now - state.sent_at >= refill_after` -> `default_amount` (Range ability refill: straight back to full, not +1).
6. None.

`resource_confirmed(state, amount, now)` (`req:245-249`): `auth = amount`, `baseline = None`, `sent_at = now`. First send exports the component (`export = state.auth is None`, `ruh:1660`).

Python behaviours to keep (not bugs to fix silently): a prediction change that happens between our send and the client applying it is absorbed into the baseline and never confirmed (X.12 gun trace, step 9). A prediction that the client never initialised can never overwrite the ammo.

### 5.4 Who is in the mirror (`mirror_ready_equippables`, `ruh:1316-1331`)

In this order:

1. Starting BasePistol (`req:320-327`; key `"BasePistol"`, ch9, actor 94, slot 1, machine 96) if A26 was sent (`inactive_state_link_sent_sources`), it was not replaced by a bought sidearm (`starting_pistol_replaced`), and `auth_correct_rep_index >= 5`.
2. Knife (`req:328-331`; key `"Melee"`, class `Ability_Melee_Base_C`, ch13, actor 204, slot 2, machine 206) if B21 was sent (`melee_inactive_state_sent_sources`).
3. If `starting_equippable_index >= 7`: for each entry of `starting_ability_resources()` (smoke, Q, E), the full ability equippable registered by the ability job if present (same key = class name, so its resource state carries over), else the resource-only entry (`mirror_states=False`, `state_machine_guid=0`, `{"InactiveState": 0}`). Then any other registered ability not in that list (Blade Storm X), in registration order.
4. `purchased_ready_equippables[source]` in registration order.

### 5.5 The mirror pass (`ruh:1600-1711`)

Runs inside the per-datagram authority block (`ruh:1566-2078`). That block runs for every authenticated client datagram when `receive_key_override is not None and live_client_pid and live_client_image_base and source in client_restart_sent_sources` (`ruh:1566-1568`), before the session-ack tick (see 03 section 3). Gate: `pistol_state_mirror and state_reader is not None and ready and now - equippable_mirror_polled[source] >= 0.05` (missing = 0). `equippable_mirror_polled[source] = now` is written before the read, even if the read fails. `now` is one `time.monotonic()` per datagram (`ruh:1569`).

1. `machines = read_slot_machines(reader, [eq.slot for eq in ready])` (live memory, section 8.4). An exception -> `{}` and event `equippable_mirror_error`.
2. For each `eq` in ready order, skip unless `machines[eq.slot]` exists and its `class_name == eq.class_name`. Then:
   1. `ability_projectiles` and class `Ability_Wushu_4_Smoke_C` -> Cloudburst hook (projectiles doc).
   2. `range_bots and machine.is_current and eq.resources` -> firing-seed hook: a change of the FiringState RNG seed counts as one shot at the bots (bots doc).
   3. Resources, if any: `read_resource_amounts(reader, machine.item, eq.resources)`. For each resource in order with a reading: `resource_target(...)`, and if not None send `build_resource_amount_body(eq, res, amount, export)` on `eq.channel`, then `resource_confirmed`. Key `(source, eq.key, res.name)`.
   4. `if not eq.mirror_states: continue`.
   5. Extra machines, if any: `read_item_machines(reader, machine.item)` keyed by machine name; for each `extra` in `eq.extra_machines` order: `machine_mirror_target`, send `build_machine_state_body(eq, extra, target, export)`, update. Key `(source, f"{eq.key}/{extra.name}")`.
   6. Main machine: `mirror_target(eq, mirror, machine.predicted, machine.is_current)`, send, update. Key `(source, eq.key)`.
3. Event `sent_equippable_auth_states`: `{eq.key: [target, predicted], "<key>/<extra>": target, "resources": {"<key>.<name>": amount}}`.

Every send goes through the local `send_reliable` (`ruh:1571-1585`): reliable, `channel_name_index=102`, `channel_sequence = take_channel_sequence(source, channel)`, `is_open` / `is_close` as given, the triggering datagram's `client_packet_sequence`, `ack_history_mask` and `stateless_header`, then `next_server_sequences += 1` (mod 0x4000).

### 5.6 Channel sequence counters used here (`ruh:786-802`)

`take_channel_sequence(source, ch)` returns `(B + offset) & 0xFFF` with B = `initial_actor_channel_sequences[source]` (the same B for every channel of the connection) and offset taken from a per-(source, channel) counter that starts at `channel_first_dynamic_offset[ch]` and increments per send. Starting offsets: ch8 51, ch9 5, ch13 5, ch12 / 14-18 2 (`ruh:786`). A purchased or armor channel is registered with 0 at its first send (`ruh:1197, 1225`). Any on-demand sender on the same channel (mirror, ability setup, purchases, corrections) shares the counter, so offsets follow send order and never leave a gap. An unregistered channel would raise KeyError.

## 6. Purchases

### 6.1 Trigger and decode (`ruh:1847-1907`)

Gate (inside the per-datagram block): `store_purchases and starting_equippable_index[source] >= 7` (all seven starting equippables sent, B18). Purchases decoded before that are dropped for good; the client does not resend an acked reliable RPC.

1. `decode_purchase_requests(decode_client_ares_packet(...))` (06 section 6). An exception -> `[]` + event `purchase_decode_error`.
2. Per request: skip if `channel_sequence in purchase_sequences_seen[source]` (a set of 12-bit sequences that is never cleared), else add it. Event `received_purchases` += `[field, item_path, target_slot, purchasable_type]`.
3. Skip unless `field == 156` and `item_path`. Fields 157 / 161 are still added to the dedup set and logged. `target_slot` and `purchasable_type` are only logged: the slot always comes from the store table.
4. `class_path = resolve_class_path(item_path, purchase_package_guids[source], purchase_seen_paths[source])` (`rst:63-85`). The first purchase of a class carries the full path. Later ones name the package by the NetGUID we exported: `"<guid:525>.AssaultRifle_AK_C"`.
5. `item = store_item_for_path(class_path) if class_path else None` (`rst:88-90`: `split_class_path` then `store_items.get(class_name)`). `resolve_class_path` returning None, a ValueError, or an unknown class -> event `purchase_unsupported` (the raw path), purchase dropped. If resolved differently from the raw path -> event `purchase_resolved_path`.
6. `purchase_seen_paths[source][class] = class_path`; allocator = `purchase_allocators.setdefault(source, PurchaseAllocator())`.
7. Not a gun -> `plan_item_load`; `purchase_package_guids[source][load.package_guid] = load.package`; append job `{"load", "stage": 0, "next": now, "registered_tries": 0, "armor": True}`; event `purchase_planned = [display, channel, actor]`.
8. Gun -> `plan_gun_load`; same bookkeeping without `armor`.
9. Then, in the same datagram, every job with `stage < 7 and now >= next` advances one step in list order (`ruh:1904-1907`). A new job's stage 0 therefore runs in the datagram that carried the purchase. Finished jobs stay in the list.

`split_class_path` (`rst:53-60`): must be a str starting `/Game/` with exactly one `.`; class must end `_C` and the package must end with `/` + class stem; returns `(package, class, "Default__" + class)`. Vectors X.13.

### 6.2 PurchaseAllocator (`rgl:38-70`)

Per connection, created on the first purchase. `first_purchase_channel = 20` (ch 9-18 are starting items and teams; codex reserved ch19, `rgl:37-38`). `first_purchase_guid = 520` (session GUIDs below 520 are taken; the ACR probe holds 512-515, `rgl:39-41`).

```
channel():      c = next_channel; next_channel += 1; return c
dynamic_guid(): if next_guid odd: next_guid += 1; g = next_guid; next_guid += 2; return g      (even: actors, subobjects)
static_pair():  if next_guid even: next_guid += 1; p = (next_guid, next_guid + 2); next_guid += 4; return p   (odd: object + package)
```

The same class with `next_channel=0, next_guid=100000` is the ability allocator (`rab:25, 39-40`). Its channel counter is never used.

Allocation order per load, exactly:

- `plan_gun_load` (`rgl:123-138`): `split_class_path`, check `class_name == item.class_name and item.is_gun`, then channel; actor = dynamic; (archetype, package) = static_pair; state machine = dynamic; magazine = dynamic; reserve = dynamic; 3 cosmetic static pairs (skin, level, chroma) always, even if no skin is used; one static pair per forced attachment. "Fresh pairs every purchase: the pre-export and the open must agree" (`rgl:136`).
- `plan_item_load` (`rgl:106-113`): check not a gun; channel; actor = dynamic; (archetype, package) = static_pair.
- Stage 3 (`gun_initial_state_body` -> `gun_equippable(load, allocator)`, `rgl:224-245`): InactiveState = dynamic. `load.state_guids` is a mutable dict inside the frozen GunLoad (`compare=False`).
- Stage 5 (`ruh:1301-1303`): first `plan_extra_machines` (`ruh:1102-1118`): for each live machine of the item other than the main one, with a non-empty starting state, in item machine-array order: machine GUID = dynamic, then one dynamic per state name in that machine's live TMap order. Then `gun_equippable(load, allocator, names, extra)`: one dynamic per main-machine state name in live TMap order, skipping names already present (InactiveState).

Jobs of one connection share the allocator and run interleaved, so GUIDs depend on how purchases overlap in time. Example: X.3 (an armor bought before the Vandal reaches stage 3 takes the GUID the Vandal's InactiveState would have had; the live log shows the same with the older, attachment-free allocation: Vandal 520, Heavy Shields 548).

Fresh-allocator numbers for every item: X.2. A gun with no attachments: ch 20, actor 520, archetype/package 523/525, SM 528, magazine 530, reserve 532, cosmetics 535/537, 539/541, 543/545, next 547, InactiveState 548. The live client echoed package 525 back for the Vandal (`test_range_gun_loader.py:32`).

`GunLoad.key = f"{class_name}@{channel}"` (`rgl:89-91`), e.g. `AssaultRifle_AK_C@20`.

### 6.3 Gun job stages (`advance_purchase_job`, `ruh:1172-1314`)

All gun-channel bunches are reliable on `load.channel` with the counter started at 0. "next" is the earliest `now` at which the following stage may run; it runs on the first authenticated datagram at or after that time.

| stage | action | channel / seq offset | open | exports | next |
|---|---|---|---|---|---|
| 0a | only if the class has forced attachments and not yet exported: `attachment_pre_export_body(assets, load.attachment_guid_pairs)`; `job["attachments_exported"]=True`; event `purchase_attachments_exported = [cdo names..., bits]`; stage stays 0 | ch8, next ch8 dynamic | 0 | 1 | now + 0.2 |
| 0b | register `channel_first_dynamic_offset[ch] = 0`; choose the cosmetic (6.4); send `gun_open_body(load, cdo)`; then `gun_owner_body(load)`. Event `purchase_opened = [display, ch, actor, skin asset id or null, open bits, n attachments]` | gun ch +0, +1 | 1, 0 | 1, 0 | now + 0.5 |
| 1 | `gun_role_body(load)` | gun ch +2 | 0 | 0 | now + 0.5 |
| 2 | `gun_state_machine_open_body(load)` | gun ch +3 | 0 | 1 | now + 0.5 |
| 3 | `gun_initial_state_body(load, allocator)` (allocates InactiveState, exports it) | gun ch +4 | 0 | 1 | now + 0.5 |
| 4 | slot contents + correction + replacement (6.5) | ch8 x2, old ch | 0 | 0 | now + 0.5 |
| 5 | registration (6.6); on failure retry | gun ch +5.. for extra machines | 0 | 1 | now + 0.3 per retry |
| 7 | done | | | | |

Builders (`rgl:150-245`):

- `gun_open_body(load, cdo)`: with `cdo is None`: `build_dynamic_actor_reference((archetype, package), actor, archetype_guid, package_guid, level 5)`; if the class has attachments, append `[1: 1][1: 1][p: rep_bits][enc(actor) of rep]` with `rep = [1: 0] objarray(20, attachment object GUIDs) guidarray(40, attachment UUID words) [p: 0]`. With `cdo`: `build_catalog_cosmetic_actor_open(cdo, layout, attachments)` (section 7.3) with `layout = CosmeticActorLayout(weapon_id, channel, actor, archetype, package, archetype_guid, package_guid, 5, (18, 19, 21), (24, 28, 41), load.cosmetic_guid_pairs)`. Both raise ValueError above 7600 bits (`rat:101-102`; the client's real limit is 8064 bits, the frame field is 13 bits). `gun_attachments` raises if the load has a different number of attachment pairs than the table (`rgl:141-147`).
- `gun_owner_body` = `rsb.build_channel_actor_owner_block(actor, 10)`: `[1: 1][1: 1][p: 25][enc(actor) of [1: 0][p: 12][p: 10][p: 0]]`.
- `gun_role_body`: `[1: 1][1: 1][p: 20][enc(actor) of [1: 0][p: 4][3: 2][p: 0]]` (ENetRole is 3 bits; the 8-bit write caused "Error reading RepIndex", `rsb:1426-1430`).
- `gun_state_machine_open_body` = `build_machine_open_body(actor, sm, "EquippableStateMachine")`.
- `gun_initial_state_body` = `build_auth_state_body(gun_equippable(load, allocator), "InactiveState", True)`.

### 6.4 Cosmetic choice for a bought gun (`ruh:1226-1248`, `rgl:248-305`)

Candidates in order, each wrapped in try/except (errors -> event `purchase_cosmetic_error`, next candidate):

1. `select_saved_cdo_paths(player, weapon_id)`: `resolve_equipped_cdo_paths(cached_skin_inventory(player), weapon_id)`; None when nothing is equipped or any of skin/level/chroma is not packaged.
2. `standard_cdo_paths(player, weapon_id)`: the first catalog entry in file order with the same weapon id whose `name` ends `_Standard`; skin = its `skin_id`, level = `skin_level_id`, chroma = `chroma_id` (catalog defaults); None if missing or not packaged. Unskinned guns have no first-person cosmetic mesh, so sights would have nothing to attach to (`rgl:289-290`).
3. None (unskinned).

The first candidate whose `gun_open_body` does not raise ValueError is used. Every store gun has a `_Standard` entry (X.10). The player id is `backend.offline_player_id` (`ruh:1230`, the constant `7c9f41a4-65e2-4d81-a8d5-f879c3e5ab61`); if that import fails, event `purchase_cosmetic_error` and only the unskinned candidate is left. With the current catalog and a fresh allocator the 7600-bit fallback is unreachable: the largest packaged skin / level / chroma open for any store gun is 6552 bits (`LeverSniperRifle_C`), so a skinned open only falls back on a resolution error, never on size.

`cached_skin_inventory(player_id)` (`rgl:248-273`): one `SkinInventory` per player id, rebuilt only when `offline_skin_state.json`'s `st_mtime_ns` changes (None if stat fails). The catalog file's mtime is not part of the key, so a catalog change needs a restart. Building one parses the whole catalog (seconds); doing it per purchase stalled the responder for about 4 s.

### 6.5 Stage 4: slot, correction, replacement (`ruh:1259-1285`)

1. ch8: `build_item_slot_contents_block(slot, actor)` (`rsb:733-749`): `[1: 1][1: 0][p: 62+2*slot][1: 0][1: 0][p: 61][1: 0][p: 60][p: rep_bits][enc(10) of [1: 0][p: 1][p: actor][p: 0]]` (rep_bits = 33 for actor GUIDs 128..16383).
2. `correction_index = auth_correct_rep_index[source] + 1`; ch8: `build_inventory_correction_body(correction_index, actor)` (`rsb:813-839`): `[1: 1][1: 0][p: 60][1: 1][p: bits][enc(10) of [1: 0][p: 25][p: actor][p: 26][32: index][p: 0]]`; store the index. The counter is shared with A27 and the input-correction responder.
3. Replacement: `replaced = purchased_slot_items[source].get(slot)`. If None, the slot is 1 (the starting pistol's slot) and the starting pistol was not replaced yet, then replaced = `(9, "BasePistol", 94, "BasePistol_C")` and the source is added to `starting_pistol_replaced` (the pistol leaves the mirror). If replaced: send a reliable close on the old channel (`send_reliable(old_ch, b"", 0, False, is_close=True)`: empty body, `bClose=1`, close reason `SerializeInt(0, 15)` = Destroyed, `rsb:1519-1521, 1576-1577`, next sequence of the old channel); drop the old key from `purchased_ready_equippables`; delete `purchased_class_guids[source][old_class]` if it still maps to the old actor. Event `purchase_replaced = [old class, old ch, old actor]`.
4. `purchased_slot_items[source][slot] = (ch, key, actor, class)`; `purchased_class_guids[source][class] = actor` (used by the input-correction responder, `ruh:1738-1740`). Event `purchase_slotted = [display, slot, correction_index]`.

### 6.6 Stage 5: registration (`ruh:1286-1312`)

1. `machine = read_slot_machines(reader, [slot]).get(slot)`, only if `state_reader is not None`; an exception sets event `purchase_register_error` and leaves `machine = None`. If `machine` is None or `machine.class_name != item.class_name`: `registered_tries += 1`; if it is now > 20, stage 7 and event `purchase_register_gave_up` (display name); else next = now + 0.3. So at most 21 attempts, and a missing reader or a read error counts as a failed attempt.
2. `names = list(reader._names.get(machine.state_machine, {}))`: the state names in live TMap order (state machine +0x160, 24-byte elements), taken from the reader's per-address cache that `read_slot_machines` just filled (section 8.4). For guns the list includes `InactiveState` (`gun_equippable` skips it); the ability job filters it out instead (`ruh:1159-1160`).
3. `extra = plan_extra_machines(...)` (allocates; 6.2), then `gun_equippable(load, allocator, names, extra)`.
4. `send_extra_machines` (`ruh:1120-1128`): for each extra machine, on the gun channel: `build_machine_open_body(actor, extra.guid, extra.name)` (exports), then `build_machine_state_body(eq, extra, extra.starting_state, True)` (exports); set its MirrorState to `auth=starting, exported={starting}, active=True`.
5. Replace any ready entry with the same slot by this one; event `purchase_extra_machines = [names]`, `purchase_ready = [display, len(names)]`; stage 7.

Live: the Vandal has 9 state names and one `ZoomStateMachine`; Sheriff 8 names and no extra machine; Operator / Marshal 10 (`responder_20928`).

### 6.7 Armor job (`ruh:1191-1212`)

| stage | action | next |
|---|---|---|
| 0 | register ch offset 0; `item_open_body(load)` (= `build_dynamic_actor_reference`, open, exports) on the armor channel +0; `build_channel_actor_owner_block(actor, 10)` +1 | now + 0.5 |
| 1 | ch8 `build_item_slot_contents_block(11, actor)` | now + 0.5 |
| 2 | `shield = armor_shield_values.get(class, 25.0)`; `export = source not in shield_section_exported`; ch8 `build_shield_section_body(shield, shield, export)` with `has_exports=export`; add source; event `purchase_shield = [display, shield]`; stage 7 | - |

Armor is not recorded in `purchased_slot_items` or `purchased_class_guids`. A second armor purchase opens a new actor and rewrites slot 11; the old armor channel stays open (Python behaviour).

### 6.8 Purchase timing (nominal, from the purchase datagram at t)

Gun with attachments: t pre-export, t+0.2 open + owner, t+0.7 role, t+1.2 state machine, t+1.7 InactiveState, t+2.2 slot + correction (+ close), t+2.7 registration (+0.3 per retry). Without attachments everything is 0.2 s earlier. Armor: t open + owner, t+0.5 slot, t+1.0 shield.

## 7. Cosmetics

### 7.1 Where a cosmetic open is built (production)

| actor | when | selection | builder | on error |
|---|---|---|---|---|
| BasePistol ch9 (A21) | first stage-chain tick eligible for A21 (03) | `select_base_pistol_cosmetic_loadout` (`ruh:96-115`): a fresh `SkinInventory` (no cache), `resolve_equipped_cosmetic` + `resolve_equipped_cdo_paths` | `rwc.build_base_pistol_initial_cosmetic_open(cosmetic, cdo_paths)` if all three are packaged, else the plain open (94/95/97/5, 837 bits) | not caught: the exception leaves the `except ValueError` handler and kills the responder (03 section 8.1) |
| Knife ch13 (B18 i=1) | B18 i=1 | `rmc.select_saved_melee_cdo_paths` (`rmc:53-75`): `cached_skin_inventory` for the default paths | `rmc.build_starting_melee_cosmetic_open` = `build_catalog_cosmetic_actor_open(paths, starting_melee_cosmetic_layout)` | caught: plain `build_starting_equippable_open(1)`, event `melee_initial_cosmetic_error` |
| Bought gun | purchase stage 0b | section 6.4 | `gun_open_body(load, cdo)` | caught per candidate |

Events: A21 `base_pistol_initial_cosmetic_bits` plus `base_pistol_initial_skin_id/_level_id/_chroma_id`, or `base_pistol_initial_cosmetic_unshipped`, or `base_pistol_initial_cosmetic_unselected`; knife `melee_initial_skin_id/_level_id/_chroma_id` or `melee_initial_cosmetic_unselected` or `melee_initial_cosmetic_error`.

### 7.2 BasePistol initial cosmetic open (`rwc:147-222`)

Input: `EquippedCosmetic` with `weapon_id == "29a0cfab-485b-f5d5-779a-b59f85e204a8"` and catalog `EquippedCdoPaths` for the same weapon. Each of skin / level / chroma must satisfy: `source_asset` starts `Equippables/Guns/Sidearms/BasePistol/`, `kind` matches, `asset_id == uuid_text`, `source_asset` equal, `package_path` starts `/Game/Equippables/Guns/Sidearms/BasePistol/`, packaged (`rwc:172-184`). Else ValueError.

```
[1: 0][32: 4]
static entry(Default__BasePistol_C, /Game/Equippables/Guns/Sidearms/BasePistol/BasePistol, 95, 97)
static entry(skin CDO, skin package, 229, 231)
static entry(level CDO, level package, 233, 235)
static entry(chroma CDO, chroma package, 237, 239)
[p: 94][p: 95][p: 5][4: 0]
[1: 1][1: 1][p: rep_bits][enc(94) of rep]
rep = [1: 0][p: 18][p: 229][p: 19][p: 233][p: 21][p: 237]
      [p: 24..27][32: skin words]  [p: 28..31][32: level words]  [p: 41..44][32: chroma words]  [p: 0]
```

Each leaf is `[p: h+k][32: word k]`. Guard: `bit_count < 8192` (`rwc:220-221`). When both succeed it is byte-identical to `build_catalog_cosmetic_actor_open(paths, base_pistol_cosmetic_layout)` (X.10), so the Rust port can emit it with the generic builder plus the pistol layout `(29a0cfab..., ch 9, 94, Default__BasePistol_C, /Game/Equippables/Guns/Sidearms/BasePistol/BasePistol, 95, 97, 5, (18,19,21), (24,28,41), ((229,231),(233,235),(237,239)))` (`rwc:247-262`). The two are not interchangeable on the error path: A21 must keep its own checks (the BasePistol-root prefixes of `rwc:172-184` and `cosmetic.weapon_id == 29a0cfab-...`) and its own guard (`< 8192`, not the generic `<= 7600`), because an A21 ValueError is the crash path (7.1). With the current catalog the guard is unreachable: the largest BasePistol skin / level / chroma combination is 6136 bits (`BasePistol_VCTCaps24_100T`, 660 packaged combinations checked offline) and the largest knife open is 5568 bits (`Melee_Esportsinvitational`, 750 combinations).

`guid_text_to_words(text)` (`rwc:51-62`): canonical 8-4-4-4-12 hex (regex `rwc:48`), hyphens removed, four 8-digit groups parsed as big-endian hex text -> four u32. Example `17f0db96-4e2d-6bfd-da2f-aaa54a1d3ffc` -> (0x17F0DB96, 0x4E2D6BFD, 0xDA2FAAA5, 0x4A1D3FFC). Proven against the live primary-asset CDOs' FGuid at +0x38 (`rwc:54-58`).

### 7.3 Generic cosmetic actor open (`rwc:265-354`)

`build_catalog_cosmetic_actor_open(cdo_paths, layout, attachments=())`. Checks (ValueError / TypeError): types; `cdo_paths.weapon_id == layout.weapon_id`; actor GUID positive even; the 8 static GUIDs (archetype, package, 3 pairs) distinct, positive, odd and not the actor GUID; handles positive; archetype package starts `/Game/Equippables/`; each path's kind matches, packaged, package starts `/Game/Equippables/`, object starts `Default__`; attachments only with the measured handles (18, 19, 21) / (24, 28, 41).

```
[1: 0][32: 4]  archetype entry, then skin, level, chroma entries (layout.asset_guid_pairs)
[p: actor][p: archetype][p: level][4: 0]
[1: 1][1: 1][p: rep_bits][enc(actor) of rep]
rep = [1: 0]
      [p: 18][p: skin obj]  [p: 19][p: level obj]
      (attachments: objarray(20, attachment object GUIDs))
      [p: 21][p: chroma obj]
      [p: 24..27] skin words   [p: 28..31] level words
      (attachments: guidarray(40, attachment words))
      [p: 41..44] chroma words
      [p: 0]
```

The arrays are inserted where the loop meets handle 21 and handle 41 (`rwc:331-343`), which keeps handles ascending. Guard 7600 bits.

Knife layout (`rmc:36-48`): weapon `2f59173c-4bed-b6c3-2191-dea9b58be9c7`, ch13, actor 204, archetype `Default__Ability_Melee_Base_C` / `/Game/Equippables/Melee/Ability_Melee_Base` 205 / 207, level 5, the pistol's handles (the knife's live layout matches through every cosmetic field, `rmc:3-11`), pairs (241,243), (245,247), (249,251).

### 7.4 Selection logic (`ec`, `si`)

`resolve_equipped_cosmetic(inventory, weapon_id)` (`ec:109-165`), under `inventory.lock`:

1. `weapon_id` must be a canonical UUID; lowercase it.
2. `skin_id = inventory.equipped.get(weapon_id)`; None -> return None (unskinned).
3. The catalog entry must exist, its `weapon_id` must match, the skin must be owned; else SkinInventoryError.
4. `inventory.loadout` must be a dict; `guns = loadout.get("guns", loadout.get("Guns", []))`; exactly one gun whose `iD` (or `ID`) lowercased equals the weapon id.
5. `_field(gun, "skinId", "SkinID")` (`ec:63-71`: every present key must be a valid UUID and they must agree; returns lowercase) must equal `skin_id`. Level = `_field("skinLevelId", "SkinLevelID")`, chroma = `_field("chromaId", "ChromaID")`.
6. `weapon_asset`, `skin_asset` validated by `_asset_path` (`ec:48-60`: str, relative, starts `Equippables/`, suffix `.uasset`, no `..`, no backslash). `level_asset` / `chroma_asset` resolved by `_component_asset`.

`_component_asset(entry, component_id, default_key, group, default_asset_key)` (`ec:74-106`): the entitlement type of the default component (exactly one); the component ids of that type in entitlement-list order; `source.unlock_assets[group]` (`levels` / `chromas`) must have the same length and unique ids; zip ids with paths; the default id must map to `source[default_asset_key]`; the selected id must be in the map.

`catalog_cdo_path(entry, kind, asset_id)` (`ec:168-212`): skin -> `asset_id` must equal the entry's `skin_id`, path `source.skin_asset`; level / chroma -> `_component_asset`. Path components must match `[A-Za-z0-9_-]+`. `package_path = "/Game/" + path without .uasset`; `object_name = "Default__" + last component + "_C"`; `packaged = not (kind == "level" and source.level_asset_packaged is False and path == source.level_asset)`. One catalog entry has `level_asset_packaged: false`: `SawedOffShotgun_Gothic`, whose default level asset `Equippables/Guns/Sidearms/Slim/Gothic/Levels/Slim_Gothic_Lv1_PrimaryAsset1.uasset` is absent from the paks (`skin_catalog_exclusions.json`, `runtime_verified_exceptions`). Only that skin's default level is unpackaged; its other levels resolve normally.

`resolve_equipped_cdo_paths` (`ec:215-226`) = the three `catalog_cdo_path` calls for the resolved selection.

### 7.5 `skin_inventory.SkinInventory` (shared with the platform backend)

Files:

- `skin_catalog.json`: `{"schema_version": 1, "skins": [...]}`, 1415 entries. Entry: `name`, `weapon_id`, `skin_id`, `skin_level_id`, `chroma_id`, `entitlements: [{type_id, item_id}]`, `source: {pak, weapon_pak, weapon_asset, skin_asset, level_asset, chroma_asset, guid_offsets_uexp, unlock_assets: {levels: [...], chromas: [...]}, optional uuid_layout, level_uuid_inherited_fields_of, level_asset_packaged, runtime_verified}`, optional `legacy_skin_ids`, `legacy_level_ids`, `legacy_chroma_ids` (42 entries).
- `offline_skin_state.json`: `{schema_version: 1, player_id, version, owned: [skin ids], equipped: {weapon_id: skin_id}, loadout: {subject, version, guns: [{iD, skinId, skinLevelId, chromaId, charmInstanceId, charmId, charmLevelId, attachments}], sprays, activeExpressions, dynamicOptions, maxExpressionSlots, incognito, identity}}`. Current copy: version 46, 1415 owned, 21 equipped (X.10).

Load rules (`si:46-198`): catalog schema 1; ids canonical (`str(uuid.UUID(v)) == v.lower()`); non-empty name; each of skin / level / chroma default has exactly one entitlement type and the three types differ; no duplicate skin id; legacy id maps validated. State: schema 1, `player_id` must equal the configured player; owned / equipped strings; legacy ids mapped to current ids; every owned skin in the catalog; every equipped entry valid, matching its weapon and owned; loadout dict with a `guns`/`Guns` list of at most 128 dicts. **The constructor rewrites the state file whenever `migrated` is true** (`si:170-173`): the lowercased + legacy-mapped `owned` list or `equipped` values differ from the file, or a loadout gun's skin / level / chroma id hit a legacy alias. So a single upper-case skin id in `owned` or `equipped` (no legacy mapping at all) also triggers the write. The write applies `_apply_equipped`, version + 1, writes `loadout["version"]` (or `"Version"`), then `_save` via `.tmp` + `os.replace` (`si:191-214`). The current copy is all lower case and has no legacy hit, so it is not rewritten (verified: constructing it on a scratch copy left that copy's mtime unchanged). The game server creates SkinInventory objects at A21, at B18 i=1 and per purchase (cached), so it can write this file.

Writers (platform backend only): `put_loadout` (`backend.py:1420-1427`, PUT `/personalization/v3/players/<id>/playerloadout`), `grant`, `grant_all`, `equip` (admin `/dev/skins/*`, `backend.py:1901-1925`). `get_loadout` returns the stored loadout with `_apply_equipped` applied (equipped skins forced into the gun entries, level / chroma reset to the catalog defaults if not granted); `entitlements()` derives instance ids with `uuid5(NAMESPACE_URL, f"offline:{player}:{type}:{item}")` (`si:330-346`). These belong to the platform port; the game server only needs the read path of 7.4.

## 8. Port notes

### 8.1 Per-connection state owned by this area (`ruh:879-935`)

| name | type | written | read |
|---|---|---|---|
| `equippable_mirror_polled` | src -> time | mirror pass | 50 ms gate |
| `equippable_mirror_states` | (src, key) -> MirrorState | mirror, `send_extra_machines` | mirror |
| `equippable_resource_states` | (src, key, name) -> ResourceMirrorState | mirror | mirror |
| `purchased_ready_equippables` | src -> [EquippableActor] | stage 4 (removal), stage 5 (add / replace by slot) | `mirror_ready_equippables` |
| `purchase_allocators` | src -> PurchaseAllocator | first purchase | plans, stage 3, stage 5 |
| `purchase_jobs` | src -> [job dict] | purchase decode | job loop |
| `purchase_sequences_seen` | src -> set of 12-bit sequences | decode | dedup (never cleared) |
| `purchase_package_guids` | src -> {package GUID: package path} | plan | `resolve_class_path` |
| `purchase_seen_paths` | src -> {class: full path} | decode | `resolve_class_path` fallback |
| `purchased_slot_items` | src -> {slot: (ch, key, actor, class)} | stage 4 | stage 4 replacement |
| `purchased_class_guids` | src -> {class: actor} | stage 4 | input-correction responder |
| `starting_pistol_replaced` | set | stage 4 | `mirror_ready_equippables`, stage 4 |
| `shield_section_exported` | set | armor stage 2 | armor stage 2 |
| `auth_correct_rep_index` | src -> int | A27, stage 4, input correction | the same |
| `channel_first_dynamic_offset` / `channel_next_offset` | ch -> offset (process-global) / (src, ch) -> offset | stage 0 registration; every send | `take_channel_sequence` |
| `ability_ready_equippables` | src -> {class name: EquippableActor} | ability job registration (`ruh:1167`) | `mirror_ready_equippables` (overrides the resource-only ability entries) |
| `inactive_state_link_sent_sources`, `melee_inactive_state_sent_sources`, `starting_equippable_index` | set / set / src -> int | A26, B21, B18 (other docs) | mirror membership (5.4), purchase gate (6.1) |
| `pistol_mirror_reader["reader"]` | one live reader (global); created on the first part-C datagram, retried on every datagram while creation fails (event `equippable_reader_error`) | first use | mirror, purchases, abilities |
| reader caches `_names`, `_class_names` | address -> names / FName, never invalidated | every read | section 8.4 |
| `_inventory_cache` (module global, `rgl:248`) | player -> (mtime_ns, SkinInventory) | `cached_skin_inventory` | knife, purchases |

NetGUID ranges (this area): 94/95/97 pistol, 96 SM, 98 / 99 / 240-252 pistol states, 204/205/207 knife, 206 knife SM, 256-304 knife states, 229-239 pistol cosmetics, 241-251 knife cosmetics, 306 / 308 pistol ammo, 310 / 312 / 314 ability charges, 316 ShieldDamageSection, 520+ purchases (per connection), 100000+ ability machines and states. Elsewhere: 254 money, 261-267 HUD classes, 512-515 ACR probe (dead), 190000+2i ability tuning, 300001+ bots, 610001+ projectiles. Channels: 9 pistol, 12-18 starting equippables, 19 reserved, 20+ purchases (never reused), 400+ bots, 10000+ projectiles. Purchase N (guns and armor both count) gets channel 19 + N, so the 381st purchase in one connection would land on channel 400, the first bot channel (`range_bots.py:44`; no guard in Python).

### 8.2 128 tps external server

- Python is receive-driven: every gate in this document is evaluated while one authenticated datagram is handled, with one `now` per datagram. In the 128 Hz loop (7.8125 ms per tick, `DESIGN.md`), drain the socket each tick and run the part-C systems per datagram in arrival order, comparing against a monotonic clock in seconds. Do not round 0.05 / 0.2 / 0.3 / 0.5 / 4.0 s to ticks.
- Keep the in-datagram order (`ruh:1499-2078`): input ack (ch8, before the part-C gate), then inside part C: reader creation, mirror (per equippable: Cloudburst hook, bot firing-seed hook, resources, extra machines, main machine), input correction, server performance RPC (off), Range HUD, streaming levels, ultimate points, purchases (decode, then jobs), Range settings, command queue, HUD refresh, projectile tick, bots, abilities; then, outside part C, the session-ack tick. Every frame takes the next server packet sequence, so the order decides the sequence numbers.
- Part C only runs for a datagram that is authenticated, is not a handshake packet, and comes from a source in `client_restart_sent_sources` (`ruh:1566-1568`). A tick with no such datagram from a source runs none of this area's systems for it: a job whose `next` has passed waits for the next datagram, and the mirror does not poll.
- The mirror poll at 50 ms means at most one pass per ~6.4 ticks. Running it every tick would send the same confirmations earlier; that is a behaviour change, not parity.
- The ability refill (`refill_after = 4.0`) compares against `sent_at` = the `now` of the datagram that carried the last send.
- Keep one bunch per datagram and no retransmission in parity mode (03 section 2.3).

### 8.3 Console and logging

The Rust server allocates a console when started hidden and logs there and to `logs/aries_server_<pid>.log` (`DESIGN.md`). For parity diffs, log one line per datagram with the Python event keys of this area: `sent_equippable_auth_states`, `equippable_reader_error`, `equippable_mirror_error`, `resource_mirror_error`, `machine_mirror_error`, `received_purchases`, `purchase_decode_error`, `purchase_unsupported`, `purchase_resolved_path`, `purchase_planned`, `purchase_attachments_exported`, `purchase_opened`, `purchase_cosmetic_error`, `purchase_slotted`, `purchase_replaced`, `purchase_shield`, `purchase_register_error`, `purchase_register_gave_up`, `extra_machines_error`, `purchase_extra_machines`, `purchase_ready`, `base_pistol_initial_*`, `melee_initial_*`. A readable console summary (one line per purchase stage and per confirmed state) is useful; the per-datagram JSON can go to the file only.

### 8.4 Live-memory inputs (oracle)

The mirror and purchase registration cannot run without these reads (`res`). Python reads them with ReadProcessMemory every 50 ms; the Rust oracle (or the planned in-process module) must supply them.

| input | read | offsets (13.06) | used by |
|---|---|---|---|
| per slot: item, item class name, state machine, predicted state name, auth state name, is current | `read_slot_machines` (`res:132-164`) | controller+0x4B0 pawn; pawn+0xBF8 inventory; inventory+0x1B8+8*slot UItemSlot; slot+0x38 item; item+0x10 class (FName at +0x18); item+0xE00 SM; inventory+0x278 current | mirror, stage 5 |
| state names (TMap order) | `state_names` (`res:77-87`) | SM+0x160 data, +0x168 count (1..64), 24-byte elements, key ptr first, state FName at +0x18 | names -> GUID allocation order |
| predicted / auth / starting state | `res:161-162, 198-200` | SM+0x1B0 predicted, +0x1C8 auth, +0x1E8 starting | mirror, extra machines |
| item machine array | `read_item_machines` (`res:177-202`) | item+0xE30 array, +0xE38 count (1..8); machine name = FName at +0x18 | extra machines |
| resource amounts | `read_resource_amounts` (`res:212-236`) | component = item+item_offset (name must equal the resource name); +0x10C predicted, +0x110 authoritative, default at +default_offset (0x12C or 0x188), all int32 | resource mirror |
| pawn (every call, not cached) | `find_pawn` (`res:30-44`) | GObjects index 0x107E2 = GEngine; +0xBD0 GameViewport; +0x80 World; World+0x1D8 GameInstance; +0x40 LocalPlayers data, element 0; +0x38 PlayerController; +0x4B0 pawn | every read above |

Reader caches (`res:97-98`, parity-relevant): `reader._names` maps a state-machine address to its `{state name: state address}` dict and `reader._class_names` maps an item / machine / component address to its decoded FName. Both are filled on first sight of an address and never invalidated for the life of the reader (one reader per responder process, `ruh:1591-1595`). Consequences to reproduce or consciously drop: a machine whose TMap read returned `{}` (count 0 or > 64 at that moment) stays nameless forever, so its `predicted` is always None and it is never mirrored; a freed item address reused by a different object keeps the old class name, so the mirror's class check can pass or fail on stale data. Stage 5 state names (6.6) come from this cache.

### 8.5 Rust layout suggestion

- `rep` crate: pure builders `auth_state_body`, `machine_open_body`, `resource_amount_body`, `shield_section_body`, `gun_open_body` (unskinned / skinned via the generic cosmetic open), `attachment_pre_export_body`, `channel_actor_owner_block`, `item_slot_contents_block`, `inventory_correction_body`, `role_body`, `stable_child_export`, `static_export_entry`, `object_array`, `guid_array`, `guid_text_to_words`. Golden-test each against appendix X.
- `world` crate: `PurchaseAllocator`, `EquippableActor`, `MirrorState`, `ResourceMirrorState`, `mirror_target`, `machine_mirror_target`, `resource_target`, the purchase job state machine, the slot / replacement tables.
- Data files from section 2; the skin catalog and saved-state JSON read through one read-only `SkinCatalog` + `SavedSelection` type (no write path in the game server).

## 9. Open questions

1. Default ammo and charge amounts come from client memory (`UAmmoComponent.Ammo` +0x12C, `MaxCharges` +0x188). 13 guns plus the three Wushu charges are logged (section 2.4); Stinger, Spectre, Bucky, Judge, Bulldog, Ares and Odin are not. Oracle read or a data table measured from the client? No such table exists in Python, and the 7 missing values must not be guessed.
2. Purchased-gun and ability state names, their order and the extra machines come from live memory at registration; GUID numbering depends on that order. Without the oracle, purchased-gun mirror bodies cannot be reproduced bit for bit.
3. A21 crashes the responder on a skin-resolution error; the knife and purchases fall back. Keep crash parity or fall back to the plain open? (also 03 question 3)
4. Constructing `SkinInventory` can rewrite `offline_skin_state.json` (legacy-ID migration, or merely a non-lowercase id in `owned` / `equipped`, section 7.5). The Rust game server should be read-only; agree with the platform port that it owns migrations. (also 03 question 4)
5. Purchases decoded before `starting_equippable_index >= 7` are dropped permanently. Keep, or queue them?
6. `purchase_sequences_seen` keys on the 12-bit channel sequence and is never cleared, so a purchase that reuses a sequence after 4096 controller-channel reliable bunches is ignored. Keep or fix?
7. A second armor purchase leaves the first armor channel open. Keep or close it?
8. A prediction change between a resource send and its application is absorbed into the baseline (X.12 gun trace step 9) and never confirmed. Observed harmless so far; keep for parity?
9. Light / Regen Shields class paths were never observed from the live client; `rst` does not need them, but a test vector for them would be a guess, so none is given.
10. Stinger, Spectre, Bucky, Judge, Bulldog and Ares class paths in X.1 follow the test convention; the convention matches all 13 live-observed paths.
11. `standard_cdo_paths` takes the first `_Standard` entry in catalog file order. The Rust catalog loader must keep file order.
12. Overlapping purchases for one slot (gun A, then gun B within about 0.5 s, or while A is still retrying stage 5). B's stage 4 closes A's channel, but A's job keeps running: its stage 5 reads the slot, and if the slot item has A's class (A and B are the same class) A is registered and replaces any ready entry for that slot. Until B's stage 5 replaces it back, the mirror sends A's GUIDs on A's closed channel. If A's stage 5 succeeds after B's, B is dropped from the mirror for good (its ammo is never confirmed). If the classes differ, A retries until it gives up (21 tries). Python behaviour from `ruh:1259-1312`; never observed live. Keep, or cancel a job whose item was replaced?
13. A Classic (`BasePistol_C`) purchase was never observed live. Its package `/Game/Equippables/Guns/Sidearms/BasePistol/BasePistol` is already mapped on the client as GUID 97 (A21). If the client sends the first Classic purchase as `<guid:97>.BasePistol_C`, `resolve_class_path` returns None, because 97 is not in `purchase_package_guids` and the class is not in `purchase_seen_paths`. The purchase is then dropped as `purchase_unsupported`. This needs a live capture before the port decides.

## Appendix X. Test vectors

Generator: `gen07.py` (scratchpad, not committed). It imports `range_equippables`, `range_gun_loader`, `range_store`, `range_attachments`, `range_weapon_cosmetics`, `equipped_cosmetic`, `skin_inventory`, `range_melee_equippable`, `range_melee_cosmetics`, `range_abilities` and `range_server_bunch` read-only, and records every `encode_short_actor_rep_payload` call (plain payload, bit count, seed, encoded) by wrapping the function in each module. Legend as in 03 / 04: `body` = the bunch body passed to `build_server_bunch_frame` (wrap it with the transport to get a frame); `plain` = the rep payload before the content transform; `encoded` = after it (same bit count). Hex is `BitWriter.to_bytes()` order (bit 0 = LSB of byte 0); long values are wrapped at 128 characters and must be concatenated. `sha256/16` = the first 16 hex digits of sha256(body bytes).

### X.1 Store table with class paths used by the vectors

`src` = where the class path comes from: `live` = sent by the live client in ServerPurchase (`runtime/responder_*.out`), `test` = `test_range_gun_loader.py`, `derived` = the `test_range_attachments.class_path` convention (catalog `weapon_asset` folder + class stem), not observed live. The server never needs this table: it takes the path from the RPC.

| display | class | cost | slot | gun | weapon_id | class path | src | split_class_path ok |
|---|---|---|---|---|---|---|---|---|
| Classic | BasePistol_C | 0 | 1 | 1 | 29a0cfab-485b-f5d5-779a-b59f85e204a8 | `/Game/Equippables/Guns/Sidearms/BasePistol/BasePistol.BasePistol_C` | test | True |
| Shorty | SawedOffShotgun_C | 300 | 1 | 1 | 42da8ccc-40d5-affc-beec-15aa47b42eda | `/Game/Equippables/Guns/Sidearms/Slim/SawedOffShotgun.SawedOffShotgun_C` | live | True |
| Frenzy | AutomaticPistol_C | 450 | 1 | 1 | 44d4e95c-4157-0037-81b2-17841bf2e8e3 | `/Game/Equippables/Guns/Sidearms/AutoPistol/AutomaticPistol.AutomaticPistol_C` | live | True |
| Ghost | LugerPistol_C | 500 | 1 | 1 | 1baa85b4-4c70-1284-64bb-6481dfc3bb4e | `/Game/Equippables/Guns/Sidearms/Luger/LugerPistol.LugerPistol_C` | live | True |
| Bandit | CompactPistol_C | 600 | 1 | 1 | 410b2e0b-4ceb-1321-1727-20858f7f3477 | `/Game/Equippables/Guns/Sidearms/Compact/CompactPistol.CompactPistol_C` | live | True |
| Sheriff | RevolverPistol_C | 800 | 1 | 1 | e336c6b8-418d-9340-d77f-7a9e4cfe0702 | `/Game/Equippables/Guns/Sidearms/Revolver/RevolverPistol.RevolverPistol_C` | live | True |
| Stinger | Vector_C | 1100 | 0 | 1 | f7e1b454-4ad4-1063-ec0a-159e56b58941 | `/Game/Equippables/Guns/SubMachineGuns/Vector/Vector.Vector_C` | derived | True |
| Spectre | SubMachineGun_MP5_C | 1600 | 0 | 1 | 462080d1-4035-2937-7c09-27aa2a5c27a7 | `/Game/Equippables/Guns/SubMachineGuns/MP5/SubMachineGun_MP5.SubMachineGun_MP5_C` | derived | True |
| Bucky | PumpShotgun_C | 850 | 0 | 1 | 910be174-449b-c412-ab22-d0873436b21b | `/Game/Equippables/Guns/Shotguns/PumpShotgun/PumpShotgun.PumpShotgun_C` | derived | True |
| Judge | AutomaticShotgun_C | 1850 | 0 | 1 | ec845bf4-4f79-ddda-a3da-0db3774b2794 | `/Game/Equippables/Guns/Shotguns/AutoShotgun/AutomaticShotgun.AutomaticShotgun_C` | derived | True |
| Bulldog | AssaultRifle_Burst_C | 2050 | 0 | 1 | ae3de142-4d85-2547-dd26-4e90bed35cf7 | `/Game/Equippables/Guns/Rifles/Burst/AssaultRifle_Burst.AssaultRifle_Burst_C` | derived | True |
| Guardian | DMR_C | 2250 | 0 | 1 | 4ade7faa-4cf1-8376-95ef-39884480959b | `/Game/Equippables/Guns/SniperRifles/DMR/DMR.DMR_C` | live | True |
| Phantom | AssaultRifle_ACR_C | 2900 | 0 | 1 | ee8e8d15-496b-07ac-e5f6-8fae5d4c7b1a | `/Game/Equippables/Guns/Rifles/Carbine/AssaultRifle_ACR.AssaultRifle_ACR_C` | live | True |
| Vandal | AssaultRifle_AK_C | 2900 | 0 | 1 | 9c82e19d-4575-0200-1a81-3eacf00cf872 | `/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK.AssaultRifle_AK_C` | live | True |
| Warden | BattleRifle_C | 2900 | 0 | 1 | 8db0a1bf-4a50-832a-4566-faaaa6d250ca | `/Game/Equippables/Guns/Rifles/BattleRifle/BattleRifle.BattleRifle_C` | live | True |
| Marshal | LeverSniperRifle_C | 950 | 0 | 1 | c4883e50-4494-202c-3ec3-6b8a9284f00b | `/Game/Equippables/Guns/SniperRifles/Leversniper/LeverSniperRifle.LeverSniperRifle_C` | live | True |
| Outlaw | DS_Gun_C | 2400 | 0 | 1 | 5f0aaf7a-4289-3998-d5ff-eb9a5cf7ef5c | `/Game/Equippables/Guns/SniperRifles/Doublesniper/DS_Gun.DS_Gun_C` | live | True |
| Operator | BoltSniper_C | 4700 | 0 | 1 | a03b24d3-4319-996d-0f8c-94bbfba1dfc7 | `/Game/Equippables/Guns/SniperRifles/Boltsniper/BoltSniper.BoltSniper_C` | live | True |
| Ares | LightMachineGun_C | 1600 | 0 | 1 | 55d8a0f4-4274-ca67-fe2c-06ab45efdf58 | `/Game/Equippables/Guns/HvyMachineGuns/LMG/LightMachineGun.LightMachineGun_C` | derived | True |
| Odin | HeavyMachineGun_C | 3200 | 0 | 1 | 63e6c2b6-4a8e-869c-3d4c-e38355226584 | `/Game/Equippables/Guns/HvyMachineGuns/HMG/HeavyMachineGun.HeavyMachineGun_C` | live | True |
| Light Shields | LightArmorItem_C | 400 | 11 | 0 | 4dec83d5-4902-9ab3-bed6-a7a390761157 | `(not observed)` | - | - |
| Heavy Shields | HeavyArmorItem_C | 1000 | 11 | 0 | 822bcab2-40a2-324e-c137-e09195ad7692 | `/Game/Gear/HeavyArmorItem.HeavyArmorItem_C` | live | True |
| Regen Shield | PlasmaArmorItem_C | 650 | 11 | 0 | b1b9086d-41bd-a516-5d29-e3b34a6f1644 | `(not observed)` | - | - |

Derived-convention check against live paths: HeavyMachineGun_C=match, AssaultRifle_AK_C=match, BattleRifle_C=match, AssaultRifle_ACR_C=match, AutomaticPistol_C=match, CompactPistol_C=match, LugerPistol_C=match, RevolverPistol_C=match, SawedOffShotgun_C=match, BoltSniper_C=match, DMR_C=match, DS_Gun_C=match, LeverSniperRifle_C=match

### X.2 PurchaseAllocator: fresh allocator per item (`plan_gun_load` / `plan_item_load`)

| class | ch | actor | archetype/package | SM | magazine | reserve | cosmetic pairs (skin, level, chroma) | attachment pairs | next_guid after plan | InactiveState (stage 3) |
|---|---|---|---|---|---|---|---|---|---|---|
| BasePistol_C | 20 | 520 | 523/525 | 528 | 530 | 532 | 535/537 539/541 543/545 | - | 547 | 548 |
| SawedOffShotgun_C | 20 | 520 | 523/525 | 528 | 530 | 532 | 535/537 539/541 543/545 | - | 547 | 548 |
| AutomaticPistol_C | 20 | 520 | 523/525 | 528 | 530 | 532 | 535/537 539/541 543/545 | - | 547 | 548 |
| LugerPistol_C | 20 | 520 | 523/525 | 528 | 530 | 532 | 535/537 539/541 543/545 | 547/549 | 551 | 552 |
| CompactPistol_C | 20 | 520 | 523/525 | 528 | 530 | 532 | 535/537 539/541 543/545 | - | 547 | 548 |
| RevolverPistol_C | 20 | 520 | 523/525 | 528 | 530 | 532 | 535/537 539/541 543/545 | - | 547 | 548 |
| Vector_C | 20 | 520 | 523/525 | 528 | 530 | 532 | 535/537 539/541 543/545 | 547/549 | 551 | 552 |
| SubMachineGun_MP5_C | 20 | 520 | 523/525 | 528 | 530 | 532 | 535/537 539/541 543/545 | 547/549 551/553 | 555 | 556 |
| PumpShotgun_C | 20 | 520 | 523/525 | 528 | 530 | 532 | 535/537 539/541 543/545 | - | 547 | 548 |
| AutomaticShotgun_C | 20 | 520 | 523/525 | 528 | 530 | 532 | 535/537 539/541 543/545 | - | 547 | 548 |
| AssaultRifle_Burst_C | 20 | 520 | 523/525 | 528 | 530 | 532 | 535/537 539/541 543/545 | 547/549 | 551 | 552 |
| DMR_C | 20 | 520 | 523/525 | 528 | 530 | 532 | 535/537 539/541 543/545 | 547/549 | 551 | 552 |
| AssaultRifle_ACR_C | 20 | 520 | 523/525 | 528 | 530 | 532 | 535/537 539/541 543/545 | 547/549 551/553 | 555 | 556 |
| AssaultRifle_AK_C | 20 | 520 | 523/525 | 528 | 530 | 532 | 535/537 539/541 543/545 | 547/549 | 551 | 552 |
| BattleRifle_C | 20 | 520 | 523/525 | 528 | 530 | 532 | 535/537 539/541 543/545 | 547/549 | 551 | 552 |
| LeverSniperRifle_C | 20 | 520 | 523/525 | 528 | 530 | 532 | 535/537 539/541 543/545 | 547/549 | 551 | 552 |
| DS_Gun_C | 20 | 520 | 523/525 | 528 | 530 | 532 | 535/537 539/541 543/545 | 547/549 | 551 | 552 |
| BoltSniper_C | 20 | 520 | 523/525 | 528 | 530 | 532 | 535/537 539/541 543/545 | 547/549 | 551 | 552 |
| LightMachineGun_C | 20 | 520 | 523/525 | 528 | 530 | 532 | 535/537 539/541 543/545 | 547/549 | 551 | 552 |
| HeavyMachineGun_C | 20 | 520 | 523/525 | 528 | 530 | 532 | 535/537 539/541 543/545 | 547/549 | 551 | 552 |
| HeavyArmorItem_C | 20 | 520 | 523/525 | - | - | - | - | - | 527 | - |

### X.3 PurchaseAllocator: one connection, sequential example

Order of calls exactly as the responder makes them when the jobs do not overlap: Vandal plan -> Vandal stage 3 (InactiveState) -> Vandal stage 5 (extra machines first, then the remaining main-machine state names) -> Heavy Shields plan -> Phantom plan. State names below are a SAMPLE (BasePistol's names in `rsb:757-767` order, one ZoomStateMachine with three sample states); in production they come from the live state machine's TMap order.

| step | allocated | next_guid after |
|---|---|---|
| plan Vandal | ch 20, actor 520, arch/pkg 523/525, SM 528, mag 530, reserve 532, cosmetics ((535, 537), (539, 541), (543, 545)), attachments ((547, 549),) | 551 |
| stage 3 InactiveState | InactiveState 552 | 554 |
| stage 5 extra machine | ZoomStateMachine 554, states {'ZoomStateDisabled': 556, 'ZoomStateUnzoomed': 558, 'ZoomStateLevel1': 560} | 562 |
| stage 5 main states | {'InactiveState': 552, 'EquipState': 562, 'IdleWaitingForEventState': 564, 'IdleState': 566, 'FiringState': 568, 'ReloadState': 570, 'ReadyingState': 572, 'JammedState': 574, 'FiringStateBurst': 576} | 578 |
| plan Heavy Shields | ch 21, actor 578, arch/pkg 581/583 | 585 |
| plan Phantom | ch 22, actor 586, arch/pkg 589/591, SM 594, mag 596, reserve 598, cosmetics ((601, 603), (605, 607), (609, 611)), attachments ((613, 615), (617, 619)) | 621 |

Interleaved example (the live log `responder_20928`, older build without attachments, shows the same effect): if Heavy Shields is planned while the Vandal job is still before stage 3, the armor takes the GUID that the Vandal's InactiveState would otherwise get.
Result: Vandal actor 520, Heavy Shields ch 21 actor 552 arch/pkg 555/557, Vandal InactiveState 560.

Ability allocator (`range_abilities.ability_allocator`, same class, `next_channel=0, next_guid=100000`): index 2: SM 100000 Inactive 100002, index 3: SM 100004 Inactive 100006, index 4: SM 100008 Inactive 100010, index 5: SM 100012 Inactive 100014; next_guid 100016.

### X.4 `range_equippables` builders

#### `pistol_auth_InactiveState_export`
- call: `build_auth_state_body(BasePistol, "InactiveState", export=True)`
- body: 285 bits
  ```
  02000000880b78011c00000092dcc2c6e8d2eccaa6e8c2e8ca0000000000022ec3210705
  ```
- plain (seed 94): 25 bits `0c880100` -> encoded `1c725000`

#### `pistol_auth_InactiveState_ref`
- call: `build_auth_state_body(BasePistol, "InactiveState", export=False)`
- body: 44 bits
  ```
  0197e1908302
  ```
- plain (seed 94): 25 bits `0c880100` -> encoded `1c725000`

#### `pistol_auth_EquipState_export`
- call: `build_auth_state_body(BasePistol, "EquipState", export=True)`
- body: 261 bits
  ```
  020000008c0b7801160000008ae2ead2e0a6e8c2e8ca0000000000022ec3a10b05
  ```
- plain (seed 94): 25 bits `0c8c0100` -> encoded `1cba5000`

#### `pistol_auth_EquipState_ref`
- call: `build_auth_state_body(BasePistol, "EquipState", export=False)`
- body: 44 bits
  ```
  0197e1d08502
  ```
- plain (seed 94): 25 bits `0c8c0100` -> encoded `1cba5000`

#### `pistol_auth_IdleWaitingForEventState_export`
- call: `build_auth_state_body(BasePistol, "IdleWaitingForEventState", export=True)`
- body: 389 bits
  ```
  02000000c2050a78013200000092c8d8caaec2d2e8d2dcce8cdee48aeccadce8a6e8c2e8ca0000000000022e647d09b00f
  ```
- plain (seed 94): 33 bits `0cc2050000` -> encoded `d69700fb00`

#### `pistol_auth_IdleWaitingForEventState_ref`
- call: `build_auth_state_body(BasePistol, "IdleWaitingForEventState", export=False)`
- body: 52 bits
  ```
  0117b2be04d807
  ```
- plain (seed 94): 33 bits `0cc2050000` -> encoded `d69700fb00`

#### `pistol_auth_IdleState_export`
- call: `build_auth_state_body(BasePistol, "IdleState", export=True)`
- body: 269 bits
  ```
  02000000ca050a78011400000092c8d8caa6e8c2e8ca0000000000022e647d89b80f
  ```
- plain (seed 94): 33 bits `0cca050000` -> encoded `d69788fb00`

#### `pistol_auth_IdleState_ref`
- call: `build_auth_state_body(BasePistol, "IdleState", export=False)`
- body: 52 bits
  ```
  0117b2be44dc07
  ```
- plain (seed 94): 33 bits `0cca050000` -> encoded `d69788fb00`

#### `pistol_auth_FiringState_export`
- call: `build_auth_state_body(BasePistol, "FiringState", export=True)`
- body: 285 bits
  ```
  02000000d2050a7801180000008cd2e4d2dccea6e8c2e8ca0000000000022e647d99b60f
  ```
- plain (seed 94): 33 bits `0cd2050000` -> encoded `d69769fb00`

#### `pistol_auth_FiringState_ref`
- call: `build_auth_state_body(BasePistol, "FiringState", export=False)`
- body: 52 bits
  ```
  0117b2be4cdb07
  ```
- plain (seed 94): 33 bits `0cd2050000` -> encoded `d69769fb00`

#### `pistol_auth_ReloadState_export`
- call: `build_auth_state_body(BasePistol, "ReloadState", export=True)`
- body: 285 bits
  ```
  02000000da050a780118000000a4cad8dec2c8a6e8c2e8ca0000000000022e647d99be0f
  ```
- plain (seed 94): 33 bits `0cda050000` -> encoded `d697e9fb00`

#### `pistol_auth_ReloadState_ref`
- call: `build_auth_state_body(BasePistol, "ReloadState", export=False)`
- body: 52 bits
  ```
  0117b2be4cdf07
  ```
- plain (seed 94): 33 bits `0cda050000` -> encoded `d697e9fb00`

#### `pistol_auth_ReadyingState_export`
- call: `build_auth_state_body(BasePistol, "ReadyingState", export=True)`
- body: 301 bits
  ```
  02000000e2050a78011c000000a4cac2c8f2d2dccea6e8c2e8ca0000000000022e647db9bc0f
  ```
- plain (seed 94): 33 bits `0ce2050000` -> encoded `d697cbfb00`

#### `pistol_auth_ReadyingState_ref`
- call: `build_auth_state_body(BasePistol, "ReadyingState", export=False)`
- body: 52 bits
  ```
  0117b2be5cde07
  ```
- plain (seed 94): 33 bits `0ce2050000` -> encoded `d697cbfb00`

#### `pistol_auth_JammedState_export`
- call: `build_auth_state_body(BasePistol, "JammedState", export=True)`
- body: 285 bits
  ```
  02000000ea050a78011800000094c2dadacac8a6e8c2e8ca0000000000022e647d59b90f
  ```
- plain (seed 94): 33 bits `0cea050000` -> encoded `d69795fb00`

#### `pistol_auth_JammedState_ref`
- call: `build_auth_state_body(BasePistol, "JammedState", export=False)`
- body: 52 bits
  ```
  0117b2beacdc07
  ```
- plain (seed 94): 33 bits `0cea050000` -> encoded `d69795fb00`

#### `pistol_auth_FiringStateBurst_export`
- call: `build_auth_state_body(BasePistol, "FiringStateBurst", export=True)`
- body: 325 bits
  ```
  02000000f2050a7801220000008cd2e4d2dccea6e8c2e8ca84eae4e6e80000000000022e647d59be0f
  ```
- plain (seed 94): 33 bits `0cf2050000` -> encoded `d697e5fb00`

#### `pistol_auth_FiringStateBurst_ref`
- call: `build_auth_state_body(BasePistol, "FiringStateBurst", export=False)`
- body: 52 bits
  ```
  0117b2be2cdf07
  ```
- plain (seed 94): 33 bits `0cf2050000` -> encoded `d697e5fb00`

#### `melee_auth_InactiveState_export`
- call: `build_auth_state_body(Melee, "InactiveState", export=True)`
- body: 317 bits
  ```
  0200000002080a3205001c00000092dcc2c6e8d2eccaa6e8c2e8ca0000000000ea142864ed911e15
  ```
- plain (seed 204): 33 bits `0c02080000` -> encoded `d61ee95101`

#### `melee_auth_InactiveState_ref`
- call: `build_auth_state_body(Melee, "InactiveState", export=False)`
- body: 60 bits
  ```
  750a14b2f6488f0a
  ```
- plain (seed 204): 33 bits `0c02080000` -> encoded `d61ee95101`

#### `melee_auth_EquipState_export`
- call: `build_auth_state_body(Melee, "EquipState", export=True)`
- body: 293 bits
  ```
  020000000a080a320500160000008ae2ead2e0a6e8c2e8ca0000000000ea142864edf11115
  ```
- plain (seed 204): 33 bits `0c0a080000` -> encoded `d61e1f5101`

#### `melee_auth_EquipState_ref`
- call: `build_auth_state_body(Melee, "EquipState", export=False)`
- body: 60 bits
  ```
  750a14b2f6f8880a
  ```
- plain (seed 204): 33 bits `0c0a080000` -> encoded `d61e1f5101`

#### `melee_auth_ReadyingState_export`
- call: `build_auth_state_body(Melee, "ReadyingState", export=True)`
- body: 317 bits
  ```
  0200000012080a3205001c000000a4cac2c8f2d2dccea6e8c2e8ca0000000000ea142864eda11915
  ```
- plain (seed 204): 33 bits `0c12080000` -> encoded `d61e9a5101`

#### `melee_auth_ReadyingState_ref`
- call: `build_auth_state_body(Melee, "ReadyingState", export=False)`
- body: 60 bits
  ```
  750a14b2f6d08c0a
  ```
- plain (seed 204): 33 bits `0c12080000` -> encoded `d61e9a5101`

#### `melee_auth_FirstAttackIdle_export`
- call: `build_auth_state_body(Melee, "FirstAttackIdle", export=True)`
- body: 333 bits
  ```
  020000002a080a320500200000008cd2e4e6e882e8e8c2c6d692c8d8ca0000000000ea142864ed611f15
  ```
- plain (seed 204): 33 bits `0c2a080000` -> encoded `d61ef65101`

#### `melee_auth_FirstAttackIdle_ref`
- call: `build_auth_state_body(Melee, "FirstAttackIdle", export=False)`
- body: 60 bits
  ```
  750a14b2f6b08f0a
  ```
- plain (seed 204): 33 bits `0c2a080000` -> encoded `d61ef65101`

#### `melee_auth_MeleeAttackState_Alt_export`
- call: `build_auth_state_body(Melee, "MeleeAttackState_Alt", export=True)`
- body: 373 bits
  ```
  020000009a080a3205002a0000009acad8caca82e8e8c2c6d6a6e8c2e8cabe82d8e80000000000ea142864ed711315
  ```
- plain (seed 204): 33 bits `0c9a080000` -> encoded `d61e375101`

#### `melee_auth_MeleeAttackState_Alt_ref`
- call: `build_auth_state_body(Melee, "MeleeAttackState_Alt", export=False)`
- body: 60 bits
  ```
  750a14b2f6b8890a
  ```
- plain (seed 204): 33 bits `0c9a080000` -> encoded `d61e375101`

#### `melee_auth_WaitForComboInput5_export`
- call: `build_auth_state_body(Melee, "WaitForComboInput5", export=True)`
- body: 357 bits
  ```
  02000000c2080a32050026000000aec2d2e88cdee486dedac4de92dce0eae86a0000000000ea142864edb11f15
  ```
- plain (seed 204): 33 bits `0cc2080000` -> encoded `d61efb5101`

#### `melee_auth_WaitForComboInput5_ref`
- call: `build_auth_state_body(Melee, "WaitForComboInput5", export=False)`
- body: 60 bits
  ```
  750a14b2f6d88f0a
  ```
- plain (seed 204): 33 bits `0cc2080000` -> encoded `d61efb5101`

#### `machine_open_zoom_vandal`
- call: `build_machine_open_body(520, 554, "ZoomStateMachine")`
- body: 317 bits
  ```
  02000000aa100a22100022000000b4dededaa6e8c2e8ca9ac2c6d0d2dcca0000000000aa4228f109
  ```
- plain (seed 520): 9 bits `0000` -> encoded `9f00`

#### `machine_state_zoom_disabled_export`
- call: `build_machine_state_body(<X.3 Vandal>, <X.3 ZoomStateMachine>, "ZoomStateDisabled", True)`
- body: 349 bits
  ```
  02000000b2100a22100024000000b4dededaa6e8c2e8ca88d2e6c2c4d8cac80000000000aa4228644d121806
  ```
- plain (seed 520): 33 bits `0cb2100000` -> encoded `d624816100`

#### `machine_state_zoom_unzoomed_ref`
- call: `build_machine_state_body(<X.3 Vandal>, <X.3 ZoomStateMachine>, "ZoomStateUnzoomed", False)`
- body: 60 bits
  ```
  552114b26ea20e03
  ```
- plain (seed 520): 33 bits `0cba100000` -> encoded `d64dd46100`

#### `vandal_auth_EquipState_export`
- call: `build_auth_state_body(<X.3 Vandal>, "EquipState", True)`
- body: 293 bits
  ```
  02000000ca100a221000160000008ae2ead2e0a6e8c2e8ca00000000000a4128644df21d06
  ```
- plain (seed 520): 33 bits `0cca100000` -> encoded `d624df6100`

#### `shield_25_export`
- call: `build_shield_section_body(25.0, 25.0, True)`
- body: 422 bits
  ```
  02000000f2080a280028000000a6d0d2cad8c888c2dac2cecaa6cac6e8d2dedc0000000000ca2348fcac15069ec91716f6a7c25229
  ```
- plain (seed 10): 98 bits `0c220000200729000020070100` -> encoded `cf5a61e0997c61617f2a2c9502`

#### `shield_25_ref`
- call: `build_shield_section_body(25.0, 25.0, False)`
- body: 125 bits
  ```
  e511247ed60a03cfe40b0bfb5361a914
  ```
- plain (seed 10): 98 bits `0c220000200729000020070100` -> encoded `cf5a61e0997c61617f2a2c9502`

#### `shield_50_export`
- call: `build_shield_section_body(50.0, 50.0, True)`
- body: 422 bits
  ```
  02000000f2080a280028000000a6d0d2cad8c888c2dac2cecaa6cac6e8d2dedc0000000000ca23481ca415069ec91716f687ca5229
  ```
- plain (seed 10): 98 bits `0c220000200929000020090100` -> encoded `415a61e0997c61617fa82c9502`

#### `shield_50_ref`
- call: `build_shield_section_body(50.0, 50.0, False)`
- body: 125 bits
  ```
  e511240ed20a03cfe40b0bfb4365a914
  ```
- plain (seed 10): 98 bits `0c220000200929000020090100` -> encoded `415a61e0997c61617fa82c9502`

#### `pistol_MagazineAmmo_12_export`
- call: `build_resource_amount_body(BasePistol, MagazineAmmo(guid 308), 12, True)`
- body: 317 bits
  ```
  02000000d2080a78011a0000009ac2cec2f4d2dcca82dadade00000000004a232866dd4a6e851d10
  ```
- plain (seed 94): 49 bits `0c180000000000` -> encoded `d6ade456d80101`

#### `pistol_MagazineAmmo_12_ref`
- call: `build_resource_amount_body(BasePistol, MagazineAmmo(guid 308), 12, False)`
- body: 76 bits
  ```
  a51114b36e25b7c20e08
  ```
- plain (seed 94): 49 bits `0c180000000000` -> encoded `d6ade456d80101`

#### `pistol_MagazineAmmo_11_ref`
- call: `build_resource_amount_body(BasePistol, MagazineAmmo(guid 308), 11, False)`
- body: 76 bits
  ```
  a51114b36e95b7c20e08
  ```
- plain (seed 94): 49 bits `0c160000000000` -> encoded `d6adf256d80101`

#### `pistol_ReserveAmmo_36_export`
- call: `build_resource_amount_body(BasePistol, ReserveAmmo(guid 306), 36, True)`
- body: 309 bits
  ```
  02000000ca080a780118000000a4cae6cae4ecca82dadade00000000002a232866dd0a66851d10
  ```
- plain (seed 94): 49 bits `0c480000000000` -> encoded `d6ad6056d80101`

#### `pistol_ReserveAmmo_36_ref`
- call: `build_resource_amount_body(BasePistol, ReserveAmmo(guid 306), 36, False)`
- body: 76 bits
  ```
  951114b36e05b3c20e08
  ```
- plain (seed 94): 49 bits `0c480000000000` -> encoded `d6ad6056d80101`

#### `vandal_MagazineAmmo_25_export`
- call: `build_resource_amount_body(<X.3 Vandal>, MagazineAmmo(guid 530), 25, True)`
- body: 325 bits
  ```
  020000004a100a2210001a0000009ac2cec2f4d2dcca82dadade00000000002a412866ad2bfa418519
  ```
- plain (seed 520): 49 bits `0c320000000000` -> encoded `d6baa21f549801`

#### `vandal_MagazineAmmo_24_ref`
- call: `build_resource_amount_body(<X.3 Vandal>, MagazineAmmo(guid 530), 24, False)`
- body: 76 bits
  ```
  952014b3d655ffa0c20c
  ```
- plain (seed 520): 49 bits `0c300000000000` -> encoded `d6baea1f549801`

#### `vandal_ReserveAmmo_50_export`
- call: `build_resource_amount_body(<X.3 Vandal>, ReserveAmmo(guid 532), 50, True)`
- body: 317 bits
  ```
  0200000052100a22100018000000a4cae6cae4ecca82dadade00000000004a412866adabf4418519
  ```
- plain (seed 520): 49 bits `0c640000000000` -> encoded `d6ba4a1f549801`

#### `ability_Ability_Wushu_4_Smoke_C_EquipmentCharge_2_export`
- call: `build_resource_amount_body(Ability_Wushu_4_Smoke_C(ch 14, actor 208), EquipmentCharge(guid 310), 2, True)`
- body: 349 bits
  ```
  02000000da080a420500200000008ae2ead2e0dacadce886d0c2e4ceca00000000006a2328666da2ed9b6e1a
  ```
- plain (seed 208): 49 bits `0c040000000000` -> encoded `d626dabee9a601`

#### `ability_Ability_Wushu_4_Smoke_C_EquipmentCharge_0_ref`
- call: `build_resource_amount_body(Ability_Wushu_4_Smoke_C(ch 14, actor 208), EquipmentCharge(guid 310), 0, False)`
- body: 76 bits
  ```
  b51114b336b9f64d370d
  ```
- plain (seed 208): 49 bits `0c000000000000` -> encoded `d626d7bee9a601`

#### `ability_Ability_Wushu_Q_CycloneBoost_C_EquipmentCharge_1_export`
- call: `build_resource_amount_body(Ability_Wushu_Q_CycloneBoost_C(ch 16, actor 216), EquipmentCharge(guid 312), 1, True)`
- body: 349 bits
  ```
  02000000e2080a620500200000008ae2ead2e0dacadce886d0c2e4ceca00000000008a232866ed921e06d312
  ```
- plain (seed 216): 49 bits `0c020000000000` -> encoded `d62ee961302d01`

#### `ability_Ability_Wushu_Q_CycloneBoost_C_EquipmentCharge_0_ref`
- call: `build_resource_amount_body(Ability_Wushu_Q_CycloneBoost_C(ch 16, actor 216), EquipmentCharge(guid 312), 0, False)`
- body: 76 bits
  ```
  c51114b376b90e836909
  ```
- plain (seed 216): 49 bits `0c000000000000` -> encoded `d62ed761302d01`

#### `ability_Ability_Wushu_E_Dash_C_SignatureAbilityResource_1_export`
- call: `build_resource_amount_body(Ability_Wushu_E_Dash_C(ch 15, actor 212), SignatureAbilityResource(guid 314), 1, True)`
- body: 421 bits
  ```
  02000000ea080a52050032000000a6d2cedcc2e8eae4ca82c4d2d8d2e8f2a4cae6deeae4c6ca0000000000aa2328667d920e563c16
  ```
- plain (seed 212): 49 bits `0c020000000000` -> encoded `d627e960c56301`

#### `ability_Ability_Wushu_E_Dash_C_SignatureAbilityResource_0_ref`
- call: `build_resource_amount_body(Ability_Wushu_E_Dash_C(ch 15, actor 212), SignatureAbilityResource(guid 314), 0, False)`
- body: 76 bits
  ```
  d51114b33eb9062b1e0b
  ```
- plain (seed 212): 49 bits `0c000000000000` -> encoded `d627d760c56301`

### X.5 `range_gun_loader` setup bodies (Vandal, fresh allocator, live class path)

Load: ch 20, actor 520, archetype/package 523/525, SM 528, magazine 530, reserve 532, cosmetic pairs ((535, 537), (539, 541), (543, 545)), attachment pairs ((547, 549),).

#### `vandal_attachment_pre_export`
- call: `attachment_pre_export_body(forced_attachments("AssaultRifle_AK_C"), ((547, 549),))`
- body: 1305 bits
  ```
  020000008e100a96100a00b80000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e82965e82e8e8c2c6d0dacadce8e65ea4caccd8
  caf05ea4cac888dee85e82e8e8c2c6d0dacadce8be8296bea4caccd8caf0a0e4d2dac2e4f282e6e6cae800000000005800000088caccc2ead8e8bebe82e8e8c2
  c6d0dacadce8be8296bea4caccd8caf0a0e4d2dac2e4f282e6e6cae8be86000000000000
  ```

#### `vandal_open_unskinned`
- call: `gun_open_body(load, None)`
- body: 1144 bits
  ```
  020000002e100a36100a00620000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e82965e82e6e6c2ead8e8a4d2ccd8cabe829600
  000000003600000088caccc2ead8e8bebe82e6e6c2ead8e8a4d2ccd8cabe8296be86000000000022102e1014e00182f8d46ea01f6c85b002061137cf2d7b889e
  351ee6012ddca232bca453034ef3c8
  ```
- plain (seed 520): 257 bits
  ```
  500200048e1000a002000420e71fb20990dc9b9a0c3e79991a11246d24f6000000
  ```
- encoded (seed 520): 257 bits
  ```
  f1a9dd403fd80a61050c226e9e5bf6103d6b3ccc035ab845657849a7069ce69101
  ```

#### `vandal_owner`
- call: `gun_owner_body(load)`
- body: 35 bits
  ```
  cb94985a05
  ```
- plain (seed 520): 25 bits `30280000` -> encoded `25a65601`

#### `vandal_role`
- call: `gun_role_body(load)`
- body: 30 bits
  ```
  a37c832f
  ```
- plain (seed 520): 20 bits `100400` -> encoded `dfe00b`

#### `vandal_state_machine_open`
- call: `gun_state_machine_open_body(load)`
- body: 365 bits
  ```
  0200000042100a2210002e0000008ae2ead2e0e0c2c4d8caa6e8c2e8ca9ac2c6d0d2dcca00000000000a4128f109
  ```
- plain (seed 520): 9 bits `0000` -> encoded `9f00`

#### `vandal_initial_inactive`
- call: `gun_initial_state_body(load, allocator)  # allocates InactiveState 552`
- body: 317 bits
  ```
  02000000a2100a2210001c00000092dcc2c6e8d2eccaa6e8c2e8ca00000000000a4128644d421506
  ```
- plain (seed 520): 33 bits `0ca2100000` -> encoded `d624546100`

#### `vandal_open_standard`
- call: `gun_open_body(load, standard_cdo_paths -> AssaultRifle_AK_Standard)`
- body: 5688 bits
  ```
  080000002e100a36100a00620000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e82965e82e6e6c2ead8e8a4d2ccd8cabe829600
  000000003600000088caccc2ead8e8bebe82e6e6c2ead8e8a4d2ccd8cabe8296be8600000000005e100a66100a00a00000005e8ec2daca5e8ae2ead2e0e0c2c4
  d8cae65e8eeadce65ea4d2ccd8cae65e82965ea6e8c2dcc8c2e4c85e82e6e6c2ead8e8a4d2ccd8cabe8296bea6e8c2dcc8c2e4c8bea0e4d2dac2e4f282e6e6ca
  e800000000006200000088caccc2ead8e8bebe82e6e6c2ead8e8a4d2ccd8cabe8296bea6e8c2dcc8c2e4c8bea0e4d2dac2e4f282e6e6cae8be8600000000006e
  100a76100a00a80000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e82965ea6e8c2dcc8c2e4c85e82e6e6c2ead8e8a4d2ccd8ca
  be8296bea6e8c2dcc8c2e4c8be98ec62bea0e4d2dac2e4f282e6e6cae800000000006a00000088caccc2ead8e8bebe82e6e6c2ead8e8a4d2ccd8cabe8296bea6
  e8c2dcc8c2e4c8be98ec62bea0e4d2dac2e4f282e6e6cae8be8600000000007e100a86100a00d40000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce6
  5ea4d2ccd8cae65e82965ea6e8c2dcc8c2e4c85e86d0e4dedac2e65ea6e8c2dcc8c2e4c85e82e6e6c2ead8e8a4d2ccd8cabe8296bea6e8c2dcc8c2e4c8bea6e8
  c2dcc8c2e4c8bea0e4d2dac2e4f282e6e6cae800000000007400000088caccc2ead8e8bebe82e6e6c2ead8e8a4d2ccd8cabe8296bea6e8c2dcc8c2e4c8bea6e8
  c2dcc8c2e4c8bea0e4d2dac2e4f282e6e6cae8be86000000000022102e1014e02986fc11bf80eeb95afc2de0911d761aac11c3065f8865f577e4f64a6c04f199
  15e84e32cbcd3cdab0b9ac08f099da9afe5fe51b0692977ccce5146a202cce5f758187767216535861c8ce87ee80e66ef9d8ed320b4b76fc8b4885bb81d83e57
  a6927ea4211fc4
  ```
- plain (seed 520): 809 bits
  ```
  485e104c6e10500200048e1000547e10602e3be44f64387a9798680663103e6c087d611570cc5c6e35744067469b7802219f2c7c48ebc027a002000420e71fb2
  0990dc9b9a0c3e79991a11246d24f600a4c235c532a8305d2d93ac444984eeb0323fa8810000
  ```
- encoded (seed 520): 809 bits
  ```
  f9237e01dd73b5f85bc0233bec345823860dbe10cbeaefc8ed95d808e2332bd09d64969b79b461735911e033b535fdbfca370c242ff998cb29d440589cbfea02
  0fede42ca6b0c2909d0fdd01cdddf2b1db651696ecf817910a7703b17dae4c25fd48433e8801
  ```

`gun_setup_bodies(load, allocator, cdo)` flags `(is_open, has_exports)` in order: [(True, True), (False, False), (False, False), (False, True), (False, True)]. The responder does not call `gun_setup_bodies`; it calls the five builders one per stage (section 6).

### X.6 Gun actor-open bodies for every store gun (fresh allocator, so actor 520)

Unskinned = `gun_open_body(load, None)`. Standard = `gun_open_body(load, <weapon>_Standard CDO paths>)`. Full hex for the unskinned body follows the table; Standard bodies are given as bit count + sha256/16 here and in full for Vandal (X.5), Classic and Phantom below.

| class | attachments | unskinned bits | unskinned sha256/16 | Standard entry | Standard bits | Standard sha256/16 |
|---|---|---|---|---|---|---|
| BasePistol_C | 0 | 869 | 1e35e18c5999cda3 | BasePistol_Standard | 5440 | 0a461b016f328951 |
| SawedOffShotgun_C | 0 | 901 | bb0f03dbfe6c5e21 | SawedOffShotgun_Standard | 5568 | e31d168872be59a9 |
| AutomaticPistol_C | 0 | 949 | a15ec8732c3fc6f4 | AutomaticPistol_Standard | 5760 | 931a4bafc4198987 |
| LugerPistol_C | 1 | 1120 | d030ffd71a5431f4 | LugerPistol_Standard | 5592 | 5ea6583e2bc4bb10 |
| CompactPistol_C | 0 | 893 | 2ef43cdcdf4a1817 | Compact_Standard | 5176 | 94f54d6efd713711 |
| RevolverPistol_C | 0 | 917 | 76c182855d7015c2 | RevolverPistol_Standard | 5632 | 5427e916a957afa7 |
| Vector_C | 1 | 1096 | 90d0027dfc150b1c | Vector_Standard | 5496 | 25b012539a4a555a |
| SubMachineGun_MP5_C | 2 | 1432 | 4434de8134197335 | SubMachineGun_MP5_Standard | 6288 | d1c42a56af3ed0a9 |
| PumpShotgun_C | 0 | 893 | 93b129bc49c7bc15 | PumpShotgun_Standard | 5536 | f6102da939316fb3 |
| AutomaticShotgun_C | 0 | 973 | 6ae74a356e3d637d | AutomaticShotgun_Standard | 5856 | bd6d52bcbe830516 |
| AssaultRifle_Burst_C | 1 | 1216 | 8877310707647b50 | AssaultRifle_Burst_Standard | 5976 | 0e765ad9ede19c60 |
| DMR_C | 1 | 1008 | d9d5b469540e031a | DMR_Standard | 5144 | f2b6d5cd01793ff7 |
| AssaultRifle_ACR_C | 2 | 1384 | b47c12ee2b8187e0 | AssaultRifle_ACR_Standard | 6096 | 14c8fbd0d6857129 |
| AssaultRifle_AK_C | 1 | 1144 | 43cb31d1b00e6005 | AssaultRifle_AK_Standard | 5688 | 5e54bf86c149052a |
| BattleRifle_C | 1 | 1152 | 59f3c7e8228c2f4e | BattleRifle_Standard | 5584 | cb1663e9c98235f3 |
| LeverSniperRifle_C | 1 | 1280 | cad8ea3cf9f320ea | LeverSniperRifle_Standard | 6232 | ca7b4b1084f268ea |
| DS_Gun_C | 1 | 1128 | 94c38ad490bc38c4 | DS_Gun_Standard | 5552 | 6fbc09f47dc70654 |
| BoltSniper_C | 1 | 1176 | b7b2956fd9c5f9b1 | BoltSniper_Standard | 5816 | 0d27b31191f69f79 |
| LightMachineGun_C | 1 | 1216 | 13c75c13437ff48e | LightMachineGun_Standard | 5976 | 4331e83fdc75f45d |
| HeavyMachineGun_C | 1 | 1216 | d94b429736adf722 | HeavyMachineGun_Standard | 5976 | a233a7aa26aa540e |

#### `open_unskinned_BasePistol_C`
- body: 869 bits
  ```
  020000002e100a36100a006c0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6d2c8cac2e4dae65e84c2e6caa0d2e6e8ded85e84c2e6caa0d2
  e6e8ded800000000002c00000088caccc2ead8e8bebe84c2e6caa0d2e6e8ded8be86000000000022102e101400
  ```

#### `open_unskinned_SawedOffShotgun_C`
- body: 901 bits
  ```
  020000002e100a36100a006a0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6d2c8cac2e4dae65ea6d8d2da5ea6c2eecac89ecccca6d0dee8
  ceeadc00000000003600000088caccc2ead8e8bebea6c2eecac89ecccca6d0dee8ceeadcbe86000000000022102e101400
  ```

#### `open_unskinned_AutomaticPistol_C`
- body: 949 bits
  ```
  020000002e100a36100a00760000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6d2c8cac2e4dae65e82eae8dea0d2e6e8ded85e82eae8dedac2
  e8d2c6a0d2e6e8ded800000000003600000088caccc2ead8e8bebe82eae8dedac2e8d2c6a0d2e6e8ded8be86000000000022102e101400
  ```

#### `open_unskinned_LugerPistol_C`
- body: 1120 bits
  ```
  020000002e100a36100a00640000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6d2c8cac2e4dae65e98eacecae45e98eacecae4a0d2e6e8ded8
  00000000002e00000088caccc2ead8e8bebe98eacecae4a0d2e6e8ded8be86000000000022102e1014e00182f8d46ea01f6c853064578558cfad4488c7353963
  7172c4703a8da45303db67d8
  ```

#### `open_unskinned_CompactPistol_C`
- body: 893 bits
  ```
  020000002e100a36100a006c0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6d2c8cac2e4dae65e86dedae0c2c6e85e86dedae0c2c6e8a0d2
  e6e8ded800000000003200000088caccc2ead8e8bebe86dedae0c2c6e8a0d2e6e8ded8be86000000000022102e101400
  ```

#### `open_unskinned_RevolverPistol_C`
- body: 917 bits
  ```
  020000002e100a36100a00700000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6d2c8cac2e4dae65ea4caecded8eccae45ea4caecded8eccae4
  a0d2e6e8ded800000000003400000088caccc2ead8e8bebea4caecded8eccae4a0d2e6e8ded8be86000000000022102e101400
  ```

#### `open_unskinned_Vector_C`
- body: 1096 bits
  ```
  020000002e100a36100a00680000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6eac49ac2c6d0d2dcca8eeadce65eaccac6e8dee45eaccac6e8
  dee400000000002400000088caccc2ead8e8bebeaccac6e8dee4be86000000000022102e1014e00182f8d46ea01f6c8530db061161cfad4088ba35c7135aff5b
  a47fa624d024cd5ec4
  ```

#### `open_unskinned_SubMachineGun_MP5_C`
- body: 1432 bits
  ```
  020000002e100a36100a00780000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6eac49ac2c6d0d2dcca8eeadce65e9aa06a5ea6eac49ac2c6d0
  d2dcca8eeadcbe9aa06a00000000003a00000088caccc2ead8e8bebea6eac49ac2c6d0d2dcca8eeadcbe9aa06abe86000000000022102e1014e03983f9543bfc
  c8038530fdae8ce387a33e8398ff304b2c50152b15a01fd39523a00eb0a640ad5beaae18404e3b890b7612bc091bedf8a41d43
  ```

#### `open_unskinned_PumpShotgun_C`
- body: 893 bits
  ```
  020000002e100a36100a00700000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6d0dee8ceeadce65ea0eadae0a6d0dee8ceeadc5ea0eadae0a6
  d0dee8ceeadc00000000002e00000088caccc2ead8e8bebea0eadae0a6d0dee8ceeadcbe86000000000022102e101400
  ```

#### `open_unskinned_AutomaticShotgun_C`
- body: 973 bits
  ```
  020000002e100a36100a007a0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6d0dee8ceeadce65e82eae8dea6d0dee8ceeadc5e82eae8deda
  c2e8d2c6a6d0dee8ceeadc00000000003800000088caccc2ead8e8bebe82eae8dedac2e8d2c6a6d0dee8ceeadcbe86000000000022102e101400
  ```

#### `open_unskinned_AssaultRifle_Burst_C`
- body: 1216 bits
  ```
  020000002e100a36100a006e0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e84eae4e6e85e82e6e6c2ead8e8a4d2ccd8cabe
  84eae4e6e800000000003c00000088caccc2ead8e8bebe82e6e6c2ead8e8a4d2ccd8cabe84eae4e6e8be86000000000022102e1014e00182f8d46ea01f6c85b0
  40079178cfad35882afbef5895bc80a500c9245003f6b4ca
  ```

#### `open_unskinned_DMR_C`
- body: 1008 bits
  ```
  020000002e100a36100a00580000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6dcd2e0cae4a4d2ccd8cae65e889aa45e889aa400000000001e
  00000088caccc2ead8e8bebe889aa4be86000000000022102e1014e00182f8d46ea01f6c8530127e855ccfad1c88027be39b53dcb58eb0a8a453830757c2
  ```

#### `open_unskinned_AssaultRifle_ACR_C`
- body: 1384 bits
  ```
  020000002e100a36100a006e0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e86c2e4c4d2dcca5e82e6e6c2ead8e8a4d2ccd8
  cabe8286a400000000003800000088caccc2ead8e8bebe82e6e6c2ead8e8a4d2ccd8cabe8286a4be86000000000022102e1014e03983f9543bfcc8038530fd2e
  51b287a33e835380f18920365dabeb8d68630ee70464390904ad1bcf29cb103d50ffada7cdbe8a8df878c41d43
  ```

#### `open_unskinned_AssaultRifle_AK_C`
- body: 1144 bits
  ```
  020000002e100a36100a00620000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e82965e82e6e6c2ead8e8a4d2ccd8cabe829600
  000000003600000088caccc2ead8e8bebe82e6e6c2ead8e8a4d2ccd8cabe8296be86000000000022102e1014e00182f8d46ea01f6c85b002061137cf2d7b889e
  351ee6012ddca232bca453034ef3c8
  ```

#### `open_unskinned_BattleRifle_C`
- body: 1152 bits
  ```
  020000002e100a36100a006c0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e84c2e8e8d8caa4d2ccd8ca5e84c2e8e8d8caa4
  d2ccd8ca00000000002e00000088caccc2ead8e8bebe84c2e8e8d8caa4d2ccd8cabe86000000000022102e1014e00182f8d46ea01f6c85b0ce241159cf2d7688
  a9b5316376ff20bba8b8a4d35dd7dfbd
  ```

#### `open_unskinned_LeverSniperRifle_C`
- body: 1280 bits
  ```
  020000002e100a36100a00820000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6dcd2e0cae4a4d2ccd8cae65e98caeccae4e6dcd2e0cae45e98
  caeccae4a6dcd2e0cae4a4d2ccd8ca00000000003800000088caccc2ead8e8bebe98caeccae4a6dcd2e0cae4a4d2ccd8cabe86000000000022102e1014e00182
  f8d46ea01f6c85b0ea3b0512cfad1a8829fb77cb19e00d3d21f5a4d324e9a0d0
  ```

#### `open_unskinned_DS_Gun_C`
- body: 1128 bits
  ```
  020000002e100a36100a00700000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6dcd2e0cae4a4d2ccd8cae65e88deeac4d8cae6dcd2e0cae45e
  88a6be8eeadc00000000002400000088caccc2ead8e8bebe88a6be8eeadcbe86000000000022102e1014e00182f8d46ea01f6c85b0de73056acf2d18882f7baa
  c3dacde112cb82a4d324822cc2
  ```

#### `open_unskinned_BoltSniper_C`
- body: 1176 bits
  ```
  020000002e100a36100a00740000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6dcd2e0cae4a4d2ccd8cae65e84ded8e8e6dcd2e0cae45e84de
  d8e8a6dcd2e0cae400000000002c00000088caccc2ead8e8bebe84ded8e8a6dcd2e0cae4be86000000000022102e1014e00182f8d46ea01f6c853033299147cf
  ad5c88297bbcd8b874114a9bfda4d3dd5b16ca
  ```

#### `open_unskinned_LightMachineGun_C`
- body: 1216 bits
  ```
  020000002e100a36100a00740000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65e90ecf29ac2c6d0d2dcca8eeadce65e989a8e5e98d2ced0e89ac2
  c6d0d2dcca8eeadc00000000003600000088caccc2ead8e8bebe98d2ced0e89ac2c6d0d2dcca8eeadcbe86000000000022102e1014e00182f8d46ea01f6c8530
  144b9152cf2d2988d4b56c8bcf514e91aca524d06937cfab
  ```

#### `open_unskinned_HeavyMachineGun_C`
- body: 1216 bits
  ```
  020000002e100a36100a00740000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65e90ecf29ac2c6d0d2dcca8eeadce65e909a8e5e90cac2ecf29ac2
  c6d0d2dcca8eeadc00000000003600000088caccc2ead8e8bebe90cac2ecf29ac2c6d0d2dcca8eeadcbe86000000000022102e1014e00182f8d46ea01f6c85b0
  23069169cf2d6b8810fb5c76cd3dbed5ad9ea45303ac44b7
  ```

#### `open_standard_BasePistol_C`
- call: `gun_open_body(load(fresh), BasePistol_Standard)`
- body: 5440 bits
  ```
  080000002e100a36100a006c0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6d2c8cac2e4dae65e84c2e6caa0d2e6e8ded85e84c2e6caa0d2
  e6e8ded800000000002c00000088caccc2ead8e8bebe84c2e6caa0d2e6e8ded8be8600000000005e100a66100a00aa0000005e8ec2daca5e8ae2ead2e0e0c2c4
  d8cae65e8eeadce65ea6d2c8cac2e4dae65e84c2e6caa0d2e6e8ded85ea6e8c2dcc8c2e4c85e84c2e6caa0d2e6e8ded8bea6e8c2dcc8c2e4c8bea0e4d2dac2e4
  f282e6e6cae800000000005800000088caccc2ead8e8bebe84c2e6caa0d2e6e8ded8bea6e8c2dcc8c2e4c8bea0e4d2dac2e4f282e6e6cae8be8600000000006e
  100a76100a00b20000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6d2c8cac2e4dae65e84c2e6caa0d2e6e8ded85ea6e8c2dcc8c2e4c85e84c2
  e6caa0d2e6e8ded8bea6e8c2dcc8c2e4c8be98ec62bea0e4d2dac2e4f282e6e6cae800000000006000000088caccc2ead8e8bebe84c2e6caa0d2e6e8ded8bea6
  e8c2dcc8c2e4c8be98ec62bea0e4d2dac2e4f282e6e6cae8be8600000000007e100a86100a00de0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce6
  5ea6d2c8cac2e4dae65e84c2e6caa0d2e6e8ded85ea6e8c2dcc8c2e4c85e86d0e4dedac2e65ea6e8c2dcc8c2e4c85e84c2e6caa0d2e6e8ded8bea6e8c2dcc8c2
  e4c8bea6e8c2dcc8c2e4c8bea0e4d2dac2e4f282e6e6cae800000000006a00000088caccc2ead8e8bebe84c2e6caa0d2e6e8ded8bea6e8c2dcc8c2e4c8bea6e8
  c2dcc8c2e4c8bea0e4d2dac2e4f282e6e6cae8be86000000000022102e1014e03104fc910a9980f95afcd96f6bbb64130b5bf248ebc2ab3fdbfe58898d807de8
  244def3b567a946a61fc3d5067b898e3ffd6e1f141131a87196b6055855bf074f3052e497b36a214
  ```
- plain (seed 520): 561 bits
  ```
  485e104c6e10547e10602ed15d4964fa61b9996852372ccb6d64a34d1f705a9997a374daa1f890781691fb5f7c66f58184a49eb65a96a81084aa92ac5a57d950
  b0ba194ad80100
  ```
- encoded (seed 520): 561 bits
  ```
  f823153201f3b5f8b3dfd676c92616b6e491d685577fb6fdb1121b01fbd0499ade77acf428d5c2f87ba0ce7031c7ffadc3e38326340e33d6c0aa0ab7e0e9e60b
  5c92f66c442900
  ```

#### `open_standard_AssaultRifle_ACR_C`
- call: `gun_open_body(load(fresh), AssaultRifle_ACR_Standard)`
- body: 6096 bits
  ```
  080000002e100a36100a006e0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e86c2e4c4d2dcca5e82e6e6c2ead8e8a4d2ccd8
  cabe8286a400000000003800000088caccc2ead8e8bebe82e6e6c2ead8e8a4d2ccd8cabe8286a4be8600000000005e100a66100a00ac0000005e8ec2daca5e8a
  e2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e86c2e4c4d2dcca5ea6e8c2dcc8c2e4c85e82e6e6c2ead8e8a4d2ccd8cabe8286a4bea6e8c2dcc8c2e4
  c8bea0e4d2dac2e4f282e6e6cae800000000006400000088caccc2ead8e8bebe82e6e6c2ead8e8a4d2ccd8cabe8286a4bea6e8c2dcc8c2e4c8bea0e4d2dac2e4
  f282e6e6cae8be8600000000006e100a76100a00b40000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e86c2e4c4d2dcca5ea6e8
  c2dcc8c2e4c85e82e6e6c2ead8e8a4d2ccd8cabe8286a4bea6e8c2dcc8c2e4c8be98ec62bea0e4d2dac2e4f282e6e6cae800000000006c00000088caccc2ead8
  e8bebe82e6e6c2ead8e8a4d2ccd8cabe8286a4bea6e8c2dcc8c2e4c8be98ec62bea0e4d2dac2e4f282e6e6cae8be8600000000007e100a86100a00e00000005e
  8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e86c2e4c4d2dcca5ea6e8c2dcc8c2e4c85e86d0e4dedac2e65ea6e8c2dcc8c2e4c85e82
  e6e6c2ead8e8a4d2ccd8cabe8286a4bea6e8c2dcc8c2e4c8bea6e8c2dcc8c2e4c8bea0e4d2dac2e4f282e6e6cae800000000007600000088caccc2ead8e8bebe
  82e6e6c2ead8e8a4d2ccd8cabe8286a4bea6e8c2dcc8c2e4c8bea6e8c2dcc8c2e4c8bea0e4d2dac2e4f282e6e6cae8be86000000000022102e1014e06187fd91
  bab0ee975afc9496c5c037d7c38ce0ee287c71f573710ff0c58730eb720a54ebcdaa7402ce0b7b9a88b5fc4d7a0d0b800aabe92cc8782f75f98118f685fa2736
  05408be876428e5d4c76571040ce9565794dca82e514007ba25df5a7eb4c5ffb5fb5c6a75bc889eb3f871e7eeeabc90931215f5f0e4ec6a555bd
  ```
- plain (seed 520): 993 bits
  ```
  485e104c6e10500400048e10089e1000547e10602c64f96664bab0dd946834e585116d98f02e6671dae73c0e75b4d65b8a780e3bd67a7c9c6b1e0ca004000444
  b4338209fab43a830c389d075d10ca338e02154a174fe21978c78f8b1c3c7b7264213a3137cc01a4443744a4a8ecd8999cac06690219b184a4daa00000
  ```
- encoded (seed 520): 993 bits
  ```
  fb237561dd2fb5f8292d8b816fae8719c1dd51f8e2eae7e21ee08b0f61d6e514a8d69b55e9049c17f634116bf99bf41a16001556d35990f15eeaf20331ec0bf5
  4f6c0a8016d1ed841cbb98ecae20809c2bcbf29a9405cb2900f644bbea4fd799bef6bf6a8d4fb79013d77f0e3dfcdc5793136242bebe1c9c8c4bab7a01
  ```

### X.7 Attachment pre-export bodies (fresh allocator pairs)

#### `attachments_AssaultRifle_ACR_C`
- call: `attachment_pre_export_body(forced_attachments("AssaultRifle_ACR_C"), ((547, 549), (551, 553)))`
- body: 2865 bits
  ```
  040000008e100a96100a00d20000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e86c2e4c4d2dcca5e82e8e8c2c6d0dacadce8e6
  5ea4caccd8caf05ea4cac888dee85e82e8e8c2c6d0dacadce8be8286a4bea4caccd8caf0bea4cac888dee8a0e4d2dac2e4f282e6e6cae8000000000068000000
  88caccc2ead8e8bebe82e8e8c2c6d0dacadce8be8286a4bea4caccd8caf0bea4cac888dee8a0e4d2dac2e4f282e6e6cae8be8600000000009e100aa6100a00ce
  0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e86c2e4c4d2dcca5e82e8e8c2c6d0dacadce8e65e9aeaf4f4d8ca5ea6d2d8ca
  dcc6cae45e82e8e8c2c6d0dacadce8be8286a4bea6d2d8cadcc6cae4bea0e4d2dac2e4f282e6e6cae800000000006000000088caccc2ead8e8bebe82e8e8c2c6
  d0dacadce8be8286a4bea6d2d8cadcc6cae4bea0e4d2dac2e4f282e6e6cae8be86000000000000
  ```

#### `attachments_SubMachineGun_MP5_C`
- call: `attachment_pre_export_body(forced_attachments("SubMachineGun_MP5_C"), ((547, 549), (551, 553)))`
- body: 2929 bits
  ```
  040000008e100a96100a00da0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6eac49ac2c6d0d2dcca8eeadce65e9aa06a5e82e8e8c2c6d0da
  cadce8e65ea4caccd8caf05ea4cac888dee85e82e8e8c2c6d0dacadce8be9aa06abea4caccd8caf0bea4cac888dee8a0e4d2dac2e4f282e6e6cae80000000000
  6800000088caccc2ead8e8bebe82e8e8c2c6d0dacadce8be9aa06abea4caccd8caf0bea4cac888dee8a0e4d2dac2e4f282e6e6cae8be8600000000009e100aa6
  100a00d60000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6eac49ac2c6d0d2dcca8eeadce65e9aa06a5e82e8e8c2c6d0dacadce8e65e9aeaf4
  f4d8ca5ea6d2d8cadcc6cae45e82e8e8c2c6d0dacadce8be9aa06abea6d2d8cadcc6cae4bea0e4d2dac2e4f282e6e6cae800000000006000000088caccc2ead8
  e8bebe82e8e8c2c6d0dacadce8be9aa06abea6d2d8cadcc6cae4bea0e4d2dac2e4f282e6e6cae8be86000000000000
  ```

#### `attachments_AssaultRifle_AK_C`
- call: `attachment_pre_export_body(forced_attachments("AssaultRifle_AK_C"), ((547, 549),))`
- body: 1305 bits
  ```
  020000008e100a96100a00b80000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e82965e82e8e8c2c6d0dacadce8e65ea4caccd8
  caf05ea4cac888dee85e82e8e8c2c6d0dacadce8be8296bea4caccd8caf0a0e4d2dac2e4f282e6e6cae800000000005800000088caccc2ead8e8bebe82e8e8c2
  c6d0dacadce8be8296bea4caccd8caf0a0e4d2dac2e4f282e6e6cae8be86000000000000
  ```

#### `attachments_AssaultRifle_Burst_C`
- call: `attachment_pre_export_body(forced_attachments("AssaultRifle_Burst_C"), ((547, 549),))`
- body: 1465 bits
  ```
  020000008e100a96100a00cc0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e84eae4e6e85e82e8e8c2c6d0dacadce8e65ea4
  ccf05ea4cac888dee85e82e8e8c2c6d0dacadce8be84eae4e6e8bea4caccd8caf0bea4cac888dee8a0e4d2dac2e4f282e6e6cae800000000006c00000088cacc
  c2ead8e8bebe82e8e8c2c6d0dacadce8be84eae4e6e8bea4caccd8caf0bea4cac888dee8a0e4d2dac2e4f282e6e6cae8be86000000000000
  ```

#### `attachments_DMR_C`
- call: `attachment_pre_export_body(forced_attachments("DMR_C"), ((547, 549),))`
- body: 1473 bits
  ```
  020000008e100a96100a00d20000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6dcd2e0cae4a4d2ccd8cae65e889aa45e82e8e8c2c6d0dacadc
  e8e65ea4caccd8caf05e82869e8e5e82e8e8c2c6d0dacadce8be889aa4bea4caccd8caf0bea4cac8c8dee8a0e4d2dac2e4f282e6e6cae8000000000068000000
  88caccc2ead8e8bebe82e8e8c2c6d0dacadce8be889aa4bea4caccd8caf0bea4cac8c8dee8a0e4d2dac2e4f282e6e6cae8be86000000000000
  ```

#### `attachments_BattleRifle_C`
- call: `attachment_pre_export_body(forced_attachments("BattleRifle_C"), ((547, 549),))`
- body: 1569 bits
  ```
  020000008e100a96100a00e00000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea4d2ccd8cae65e84c2e8e8d8caa4d2ccd8ca5e82e8e8c2c6d0da
  cadce8e65ea4caccd8caf05e82c6dece5e82e8e8c2c6d0dacadce8be84c2e8e8d8caa4d2ccd8cabe82869e8ea6c6dee0cabea0e4d2dac2e4f282e6e6cae80000
  0000007200000088caccc2ead8e8bebe82e8e8c2c6d0dacadce8be84c2e8e8d8caa4d2ccd8cabe82869e8ea6c6dee0cabea0e4d2dac2e4f282e6e6cae8be8600
  0000000000
  ```

#### `attachments_Vector_C`
- call: `attachment_pre_export_body(forced_attachments("Vector_C"), ((547, 549),))`
- body: 1577 bits
  ```
  020000008e100a96100a00e60000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6eac49ac2c6d0d2dcca8eeadce65eaccac6e8dee45e82e8e8c2
  c6d0dacadce8e65ea4caccd8caf05ea4cac888dee85e82e8e8c2c6d0dacadce8beaccac6e8dee4bea4caccd8caf0bea4cac888dee8a0e4d2dac2e4f282e6e6ca
  e800000000006e00000088caccc2ead8e8bebe82e8e8c2c6d0dacadce8beaccac6e8dee4bea4caccd8caf0bea4cac888dee8a0e4d2dac2e4f282e6e6cae8be86
  000000000000
  ```

#### `attachments_LightMachineGun_C`
- call: `attachment_pre_export_body(forced_attachments("LightMachineGun_C"), ((547, 549),))`
- body: 1505 bits
  ```
  020000008e100a96100a00da0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65e90ecf29ac2c6d0d2dcca8eeadce65e989a8e5e82e8e8c2c6d0da
  cadce8e65ea4caccd8caf05ea4cac888dee85e82e8e8c2c6d0dacadce8be989a8ebea4caccd8caf0bea4cac888dee8a0e4d2dac2e4f282e6e6cae80000000000
  6800000088caccc2ead8e8bebe82e8e8c2c6d0dacadce8be989a8ebea4caccd8caf0bea4cac888dee8a0e4d2dac2e4f282e6e6cae8be86000000000000
  ```

#### `attachments_HeavyMachineGun_C`
- call: `attachment_pre_export_body(forced_attachments("HeavyMachineGun_C"), ((547, 549),))`
- body: 1337 bits
  ```
  020000008e100a96100a00be0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65e90ecf29ac2c6d0d2dcca8eeadce65e909a8e5e82e8e8c2c6d0da
  cadce8e65ea4caccd8caf05e82e8e8c2c6d0dacadce8be909a8ebea4caccd8caf0a0e4d2dac2e4f282e6e6cae800000000005a00000088caccc2ead8e8bebe82
  e8e8c2c6d0dacadce8be909a8ebea4caccd8caf0a0e4d2dac2e4f282e6e6cae8be86000000000000
  ```

#### `attachments_LeverSniperRifle_C`
- call: `attachment_pre_export_body(forced_attachments("LeverSniperRifle_C"), ((547, 549),))`
- body: 1257 bits
  ```
  020000008e100a96100a00c20000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6dcd2e0cae4a4d2ccd8cae65e98caeccae4e6dcd2e0cae45e82
  e8e8c2c6d0dacadce8e65ea4caccd8caf05e88caccc2ead8e85ea6c6dee0cabe98caeccae4be88c2e8c282e6e6cae800000000004200000088caccc2ead8e8be
  bea6c6dee0cabe98caeccae4be88c2e8c282e6e6cae8be86000000000000
  ```

#### `attachments_DS_Gun_C`
- call: `attachment_pre_export_body(forced_attachments("DS_Gun_C"), ((547, 549),))`
- body: 1145 bits
  ```
  020000008e100a96100a00ac0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6dcd2e0cae4a4d2ccd8cae65e88deeac4d8cae6dcd2e0cae45e
  82e8e8c2c6d0dacadce8e65ea6c6dee0ca5ea6c6dee0cabe88a6be88c2e8c282e6e6cae800000000003c00000088caccc2ead8e8bebea6c6dee0cabe88a6be88
  c2e8c282e6e6cae8be86000000000000
  ```

#### `attachments_BoltSniper_C`
- call: `attachment_pre_export_body(forced_attachments("BoltSniper_C"), ((547, 549),))`
- body: 1529 bits
  ```
  020000008e100a96100a00dc0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6dcd2e0cae4a4d2ccd8cae65e84ded8e8e6dcd2e0cae45e82e8
  e8c2c6d0dacadce8e65e64b0a6c6dee0ca5e82e8e8c2c6d0dacadce8be84ded8e8a6dcd2e0cae4be64b0a6c6dee0cabea0e4d2dac2e4f282e6e6cae800000000
  006c00000088caccc2ead8e8bebe82e8e8c2c6d0dacadce8be84ded8e8a6dcd2e0cae4be64b0a6c6dee0cabea0e4d2dac2e4f282e6e6cae8be86000000000000
  ```

#### `attachments_LugerPistol_C`
- call: `attachment_pre_export_body(forced_attachments("LugerPistol_C"), ((547, 549),))`
- body: 1401 bits
  ```
  020000008e100a96100a00c40000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6d2c8cac2e4dae65e98eacecae45e82e8e8c2c6d0dacadce8e6
  5ea6d2d8cadcc6cae45e82e8e8c2c6d0dacadce8be98eacecae4bea6d2d8cadcc6cae4bea0e4d2dac2e4f282e6e6cae800000000006400000088caccc2ead8e8
  bebe82e8e8c2c6d0dacadce8be98eacecae4bea6d2d8cadcc6cae4bea0e4d2dac2e4f282e6e6cae8be86000000000000
  ```

### X.8 Armor item open (Heavy Shields, live path, fresh allocator)

#### `heavy_armor_open`
- call: `item_open_body(plan_item_load("/Game/Gear/HeavyArmorItem.HeavyArmorItem_C"))`
- body: 677 bits
  ```
  020000002e100a36100a00340000005e8ec2daca5e8ecac2e45e90cac2ecf282e4dadee492e8cada00000000003400000088caccc2ead8e8bebe90cac2ecf282
  e4dadee492e8cadabe86000000000022102e101400
  ```

#### `heavy_armor_owner`
- call: `build_channel_actor_owner_block(520, 10)`
- body: 35 bits
  ```
  cb94985a05
  ```
- plain (seed 520): 25 bits `30280000` -> encoded `25a65601`

#### `heavy_armor_slot11_contents`
- call: `build_item_slot_contents_block(11, 520)`
- body: 70 bits
  ```
  a1a2074fc8da042028
  ```
- plain (seed 10): 33 bits `0422100000` -> encoded `d626004101`

#### `vandal_slot0_contents`
- call: `build_item_slot_contents_block(0, 520)`
- body: 70 bits
  ```
  f1a1074fc8da042028
  ```
- plain (seed 10): 33 bits `0422100000` -> encoded `d626004101`

#### `inventory_correction_6_vandal`
- call: `build_inventory_correction_body(6, 520)`
- body: 92 bits
  ```
  e1950c130b43c94419c20709
  ```
- plain (seed 10): 73 bits `642210680c0000000000` -> encoded `61626128992843f82001`

### X.9 Cross-check: live `purchase_opened` bit counts vs. offline builder

`runtime/responder_*.out` logs `purchase_opened = [name, channel, actor, skin_id, open_bits, n_attachments]`. Offline rebuild with a fresh allocator (GUID byte widths are identical for 520..16383) and the catalog default level and chroma of the logged skin. A mismatch only means the saved level/chroma at that time differed (their path lengths change the bit count).

| logged | skin | logged bits | offline bits (default level/chroma) | offline bits (current saved level/chroma) |
|---|---|---|---|---|
| Bandit | Compact_Liberate | 5160 | 5304 | 5160 |
| Frenzy | AutomaticPistol_Dragon | 5576 | 5672 | 5576 |
| Ghost | Luger_Commando | 5216 | 5360 | 5216 |
| Marshal | LeverSniper_Ninja | 5832 | 5832 | 5832 |
| Operator | BoltSniper_Rogue | 5512 | 5656 | 5512 |
| Outlaw | DS_Gun_Afterglow3 | 5680 | 5824 | 5680 |
| Shorty | Slim_Gunslinger2 | 5168 | 5312 | 5168 |
| Vandal | AK_StarPower | 5192 | 5192 | 5192 |
| Warden | BattleRifle_Galleria2 | 5848 | 5848 | 5848 |

### X.10 `range_weapon_cosmetics` / `equipped_cosmetic`

- `guid_text_to_words("17f0db96-4e2d-6bfd-da2f-aaa54a1d3ffc")` = (0x17F0DB96, 0x4E2D6BFD, 0xDA2FAAA5, 0x4A1D3FFC)
- `guid_text_to_words("29a0cfab-485b-f5d5-779a-b59f85e204a8")` = (0x29A0CFAB, 0x485BF5D5, 0x779AB59F, 0x85E204A8)
- `guid_text_to_words("2f59173c-4bed-b6c3-2191-dea9b58be9c7")` = (0x2F59173C, 0x4BEDB6C3, 0x2191DEA9, 0xB58BE9C7)

#### `base_pistol_initial_cosmetic_VCTCaps24_GX_lv3`
- call: `build_base_pistol_initial_cosmetic_open(<test_range_weapon_cosmetics.selected_base_pistol()>, None)`
- body: 5992 bits
  ```
  080000007c0b840b006c0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6d2c8cac2e4dae65e84c2e6caa0d2e6e8ded85e84c2e6caa0d2e6e8
  ded800000000002c00000088caccc2ead8e8bebe84c2e6caa0d2e6e8ded8be86000000000096050a9e050a00ce0000005e8ec2daca5e8ae2ead2e0e0c2c4d8ca
  e65e8eeadce65ea6d2c8cac2e4dae65e84c2e6caa0d2e6e8ded85eac86a886c2e0e664685eac86a886c2e0e66468be8eb05e84c2e6caa0d2e6e8ded8beac86a8
  86c2e0e66468be8eb0bea0e4d2dac2e4f282e6e6cae800000000006000000088caccc2ead8e8bebe84c2e6caa0d2e6e8ded8beac86a886c2e0e66468be8eb0be
  a0e4d2dac2e4f282e6e6cae8be860000000000a6050aae050a00e40000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6d2c8cac2e4dae65e84c2
  e6caa0d2e6e8ded85eac86a886c2e0e664685eac86a886c2e0e66468be8eb05e98caeccad8e65e84c2e6caa0d2e6e8ded8beac86a886c2e0e66468be8eb0be98
  ec66bea0e4d2dac2e4f282e6e6cae800000000006800000088caccc2ead8e8bebe84c2e6caa0d2e6e8ded8beac86a886c2e0e66468be8eb0be98ec66bea0e4d2
  dac2e4f282e6e6cae8be860000000000b6050abe050a00020100005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6d2c8cac2e4dae65e84c2e6caa0
  d2e6e8ded85eac86a886c2e0e664685eac86a886c2e0e66468be8eb05e86d0e4dedac2e65ea6e8c2dcc8c2e4c85e84c2e6caa0d2e6e8ded8beac86a886c2e0e6
  6468be8eb0bea6e8c2dcc8c2e4c8bea0e4d2dac2e4f282e6e6cae800000000007200000088caccc2ead8e8bebe84c2e6caa0d2e6e8ded8beac86a886c2e0e664
  68be8eb0bea6e8c2dcc8c2e4c8bea0e4d2dac2e4f282e6e6cae8be860000000000787d15e03184dc3e6ab79bbe9b58fc5240081cdd7fc42625496985beac3c45
  d4b04098158b81002c1da0ade11af68a2b28f1e2d8756233e29a936cdf6ea515dec8514fca459b89cec7e5effa
  ```
- plain (seed 94): 561 bits
  ```
  4896054ca60554b605602cb7e12f64fad75a9c684a555fb46df87f3a94708c718ad87522589599780cc9f4027d50297507a5b68486b8a91cf27998ac328d76c2
  b050d791d80000
  ```
- encoded (seed 94): 561 bits
  ```
  b97dd46e377d37b1f8a5801038baff884d4a92d20a7d59798aa86181302b160301583a405bc335ec155750e2c5b1ebc466c43527d9bedd4a2bbc91a39e948b36
  139d8fcbdff501
  ```

Same selection with catalog CDO paths and through `build_catalog_cosmetic_actor_open(paths, base_pistol_cosmetic_layout)`: identical = True.

#### `base_pistol_plain_open`
- call: `build_dynamic_actor_reference(("Default__BasePistol_C", "/Game/Equippables/Guns/Sidearms/BasePistol/BasePistol"), 94, 95, 97, 5, None)`
- body: 837 bits
  ```
  020000007c0b840b006c0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e8eeadce65ea6d2c8cac2e4dae65e84c2e6caa0d2e6e8ded85e84c2e6caa0d2e6e8
  ded800000000002c00000088caccc2ead8e8bebe84c2e6caa0d2e6e8ded8be860000000000787d1500
  ```

#### Current saved selections (`offline_skin_state.json` copy taken 2026-10-14, version 46, 1415 owned, 21 equipped)

| weapon | weapon_id | equipped skin entry | level id | chroma id | all packaged |
|---|---|---|---|---|---|
| Ghost | 1baa85b4-4c70-1284-64bb-6481dfc3bb4e | Luger_Commando | 68a98c93-4c40-5263-0057-d18285e96d14 | d6c2e200-4ac7-291d-d25a-8aa59502ad10 | True |
| Classic | 29a0cfab-485b-f5d5-779a-b59f85e204a8 | BasePistol_Coven | 48d4142e-4538-0bb6-9fa1-57b36dd87b47 | b36bfcf3-45a0-64c5-fa75-61bd9b20c107 | True |
| Melee | 2f59173c-4bed-b6c3-2191-dea9b58be9c7 | Melee_Rogue | 319ba790-4bfc-0099-4c50-de822474a7b8 | a0887c73-4ba7-3064-2378-3981c6ed3e2c | True |
| Bandit | 410b2e0b-4ceb-1321-1727-20858f7f3477 | Compact_Liberate | d8606d62-4a88-39d6-4e65-3993b0f3f9b2 | ba5bf29a-484b-30c2-46d1-6091c0b1b085 | True |
| Shorty | 42da8ccc-40d5-affc-beec-15aa47b42eda | Slim_Gunslinger2 | d31e4f0e-4048-4474-dd43-109ee72d4555 | 0a91c670-4ae3-ec59-a557-7c97ebd969d4 | True |
| Frenzy | 44d4e95c-4157-0037-81b2-17841bf2e8e3 | AutomaticPistol_Dragon | f70dc825-4be6-0074-0526-758b3f183152 | 3f8b9999-4a92-de0a-8a0a-4ebf04258950 | True |
| Spectre | 462080d1-4035-2937-7c09-27aa2a5c27a7 | MP5_Afterglow2 | 66a49dfe-414c-e55c-4961-51885b097521 | 47cd4c1c-4c8d-d262-a9f0-359206bf40dc | True |
| Guardian | 4ade7faa-4cf1-8376-95ef-39884480959b | DMR_Afterglow | 3595b261-40e6-a344-5689-a7a28128895b | c438dbb7-4ade-8240-90b5-b88b33aa5be4 | True |
| Ares | 55d8a0f4-4274-ca67-fe2c-06ab45efdf58 | LMG_Ashen2 | fb45bedd-4596-43dc-19cf-c99863cf83d0 | 9a1234a0-4167-a3c9-ec45-da8dbbf30f11 | True |
| Outlaw | 5f0aaf7a-4289-3998-d5ff-eb9a5cf7ef5c | DS_Gun_Afterglow3 | ce3e11b5-4c90-fb97-0048-5f8d93cefa1f | cba542ef-496f-2efa-bde6-999a9400d3f0 | True |
| Odin | 63e6c2b6-4a8e-869c-3d4c-e38355226584 | HMG_Cyberknight | dc308671-40a0-b7ce-ea59-7c9cfa123346 | 61e66028-439a-f814-b4a4-6baf6a4a709e | True |
| Warden | 8db0a1bf-4a50-832a-4566-faaaa6d250ca | BattleRifle_Galleria2 | 3cda025f-468b-3a1d-8936-b1b26d7c8fe5 | deb6d1b0-4a31-ba38-a1f1-f8965af2aee3 | True |
| Bucky | 910be174-449b-c412-ab22-d0873436b21b | PumpShotgun_Motorbike | 46668a73-48a3-d79a-8ed0-be9f41396bfd | c1c6684e-481b-ed62-c1b0-a1aa423b7732 | True |
| Vandal | 9c82e19d-4575-0200-1a81-3eacf00cf872 | AK_StarPower | cdb2688f-4a03-3d6a-39e0-1bafe0d9c8ce | 1bf0d77f-4e94-9dc0-7a7a-fb8430b569d3 | True |
| Operator | a03b24d3-4319-996d-0f8c-94bbfba1dfc7 | BoltSniper_Rogue | 4c974cbb-4739-562c-5e13-6eb15923e5e0 | d14b651f-45f2-e7c8-f30d-198fd5eef49e | True |
| Bulldog | ae3de142-4d85-2547-dd26-4e90bed35cf7 | Burst_Demonstone2 | 2b6edc6f-42d2-3d74-04b9-3eb83b855067 | 461e98c4-4104-5227-d802-4c8ef1bf17c4 | True |
| Marshal | c4883e50-4494-202c-3ec3-6b8a9284f00b | LeverSniper_Ninja | 50e45282-418c-6658-0a5e-e48db6c9542c | ace6ba33-47b6-8d06-df03-2ca380d78f70 | True |
| Sheriff | e336c6b8-418d-9340-d77f-7a9e4cfe0702 | RevolverPistol_Edge | 65c2c8d3-4473-4f74-305c-ff90fbf318e8 | 78c77ae1-4987-223d-720e-c596cd1f3a6c | True |
| Judge | ec845bf4-4f79-ddda-a3da-0db3774b2794 | AutoShotgun_Cyberknight | 78fefa71-4659-203a-cc3d-85b3f34a2e2f | b3d42eae-496a-aaf1-c450-6d8bce1e40de | True |
| Phantom | ee8e8d15-496b-07ac-e5f6-8fae5d4c7b1a | AssaultRifle_ACR_Edge | 9e2df910-4927-7313-e6b9-639454ef0e91 | a5529ec0-4bc2-d6c1-7699-a1a3d3809313 | True |
| Stinger | f7e1b454-4ad4-1063-ec0a-159e56b58941 | Vector_Afterglow | 7dd59c7b-4ca4-1638-4a10-cb998d6b84c8 | fa548ae1-4f50-0730-0c4f-b18c473c8542 | True |

#### `base_pistol_initial_cosmetic_current_saved`
- call: `build_base_pistol_initial_cosmetic_open(<saved BasePistol_Coven>, <catalog paths>)`
- body: 5104 bits
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
- plain (seed 94): 561 bits
  ```
  4896054ca60554b60560a0b43e0a64a4282b97685a4315e36daece4cb0715c28a891746c17708a7866af423f7d8ef6b0dba4e6f9d766a98ac9408bac7ac3eaf4
  b10e8241360100
  ```
- encoded (seed 94): 561 bits
  ```
  b97dd46e377d37b1b9f29e1038ba6cf16d8a00b048fb593f4bd53301a52b23fc889a3a79d98be3d419f7b8f5a7b23ffbd722692765dae9f4e8955c8ad9278bdc
  b7e5779e8f0f01
  ```

#### `melee_initial_cosmetic_current_saved`
- call: `build_starting_melee_cosmetic_open(<saved knife Melee_Rogue>)`
- body: 4416 bits
  ```
  0800000036050a3e050a00560000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e9acad8caca5e82c4d2d8d2e8f2be9acad8cacabe84c2e6ca00000000003c00
  000088caccc2ead8e8bebe82c4d2d8d2e8f2be9acad8cacabe84c2e6cabe860000000000c6050ace050a006e0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae6
  5e9acad8caca5ea4deceeaca5e9acad8cacabea4deceeacabea0e4d2dac2e4f282e6e6cae800000000004800000088caccc2ead8e8bebe9acad8cacabea4dece
  eacabea0e4d2dac2e4f282e6e6cae8be860000000000d6050ade050a00840000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e9acad8caca5ea4deceeaca5e98
  caeccad8e65e9acad8cacabea4deceeacabe98ec64bea0e4d2dac2e4f282e6e6cae800000000005000000088caccc2ead8e8bebe9acad8cacabea4deceeacabe
  98ec64bea0e4d2dac2e4f282e6e6cae8be860000000000e6050aee050a008a0000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e9acad8caca5ea4deceeaca5e
  86d0e4dedac2e65eec665e9acad8cacabea4deceeacabeec66bea0e4d2dac2e4f282e6e6cae800000000004e00000088caccc2ead8e8bebe9acad8cacabea4de
  ceeacabeec66bea0e4d2dac2e4f282e6e6cae8be8600000000003205360514e03184ddbc533b50e9da589456920fc94009f30a99567d90aab1b920547752d8ab
  ac4688ea7aa496aaf4e8154f666cd6c0c7ae0c6e750567f7f8a3d88ffb5acea0f05d1fc1bcb53df5
  ```
- plain (seed 204): 561 bits
  ```
  48c6054cd60554e60560d208013964d85cdd95683221a4f96d408dd5d271204f3763743201f8977804bda1987c704fe948a4e6f81041a9c8604e97ac0273f046
  b0587cda8d0100
  ```
- encoded (seed 204): 561 bits
  ```
  bb79a776a0d2b5b128ad241f928112e61532adfa2055637341a8eea4b057598d10d5f5482d55e9d12b9eccd8ac818f5d19dcea0aceeef147b11ff7b59c41e1bb
  3e82796b7bea01
  ```

#### `melee_plain_open`
- call: `build_starting_equippable_open(1)`
- body: 845 bits
  ```
  0200000036050a3e050a00560000005e8ec2daca5e8ae2ead2e0e0c2c4d8cae65e9acad8caca5e82c4d2d8d2e8f2be9acad8cacabe84c2e6ca00000000003c00
  000088caccc2ead8e8bebe82c4d2d8d2e8f2be9acad8cacabe84c2e6cabe860000000000320536051400
  ```

#### Standard (default) skin lookup per store gun (`standard_cdo_paths`)

First catalog entry (file order) with matching `weapon_id` and a name ending `_Standard`.

| class | entry | skin id | level id | chroma id | skin package |
|---|---|---|---|---|---|
| BasePistol_C | BasePistol_Standard | 24aee897-4cdc-b0fd-e596-1ba90fa6d1b2 | 51cbccad-487c-50ed-2ffd-c88b4240fab3 | 4b2d5b4f-4955-4208-286c-abadec250cdd | `/Game/Equippables/Guns/Sidearms/BasePistol/Standard/BasePistol_Standard_PrimaryAsset` |
| SawedOffShotgun_C | SawedOffShotgun_Standard | 48ad078a-4dae-2b85-a945-f4b6d1efecbb | a7f92a1c-4465-5ea3-7745-bd876117f4a7 | 95608504-4c8b-1408-1612-0f8200421c49 | `/Game/Equippables/Guns/Sidearms/Slim/Standard/SawedOffShotgun_Standard_PrimaryAsset` |
| AutomaticPistol_C | AutomaticPistol_Standard | f06657f3-48b6-6314-7235-a9a2749df5b9 | 80fabd74-4438-a2dd-0c39-42ab449f9ec6 | dc99ed5a-4d75-87a0-c921-75963ea3c1e1 | `/Game/Equippables/Guns/Sidearms/AutoPistol/Standard/AutomaticPistol_Standard_PrimaryAsset` |
| LugerPistol_C | LugerPistol_Standard | 1c63b43b-43c4-04e4-01c9-7aa1bffa5ac1 | 0a7e786c-444e-6a80-8bda-e2b714d68332 | 947a28b6-4e0f-61fb-e795-bc9a5e7b7129 | `/Game/Equippables/Guns/Sidearms/Luger/Standard/LugerPistol_Standard_PrimaryAsset` |
| CompactPistol_C | Compact_Standard | b576b9f1-407d-310a-6009-6287fb6829bc | 7bf8a186-42f3-0028-c096-4fb539a31be2 | ff308f94-427d-e6d3-8fcb-258f5d4c2c29 | `/Game/Equippables/Guns/Sidearms/Compact/Compact_Standard_PrimaryAsset` |
| RevolverPistol_C | RevolverPistol_Standard | 1ef6ba68-4dbe-30c7-6bc8-93a6c6f13f04 | feaf05a1-492f-d154-a9f5-0eb1fe9a603e | 5a59bd61-48a9-af61-c00f-4aa21deca9a8 | `/Game/Equippables/Guns/Sidearms/Revolver/Standard/RevolverPistol_Standard_PrimaryAsset` |
| Vector_C | Vector_Standard | 940fb417-4a9c-3004-41f5-3e8f1f4178b2 | 471fc2a5-47a7-5b12-2895-0899117d2f57 | 31bb2115-4c62-d37c-43c4-11b8fee7f212 | `/Game/Equippables/Guns/SubMachineGuns/Vector/Standard/Vector_Standard_PrimaryAsset` |
| SubMachineGun_MP5_C | SubMachineGun_MP5_Standard | f01d1307-4299-42f5-2c5e-7dab7e69ab19 | 1dc45e18-4a07-c85f-0020-6da4db1486ce | a9aaccca-4cdc-02ea-1d7e-89bbacecc0e2 | `/Game/Equippables/Guns/SubMachineGuns/MP5/Standard/SubMachineGun_MP5_Standard_PrimaryAsset` |
| PumpShotgun_C | PumpShotgun_Standard | 70c97fb2-4d79-d4bb-5173-a1888cd4bfd9 | 2f5078c7-4381-492d-cc00-9f96966ba1ec | 3d8ffcfe-4786-0180-42d7-e1be18dd1cab | `/Game/Equippables/Guns/Shotguns/PumpShotgun/Standard/PumpShotgun_Standard_PrimaryAsset` |
| AutomaticShotgun_C | AutomaticShotgun_Standard | acd26127-48ff-8b9e-7ba6-b989af8a4b24 | 6942d8d1-4370-a144-2140-22a6d2be2697 | b71ae8d6-44bb-aa4c-0d2a-dc9ed9e66410 | `/Game/Equippables/Guns/Shotguns/AutoShotgun/Standard/AutomaticShotgun_Standard_PrimaryAsset` |
| AssaultRifle_Burst_C | AssaultRifle_Burst_Standard | 724a7f42-4315-eccf-0e76-77bdd3ec2e09 | c8e6ac70-48ef-9d96-d964-a88e8890b885 | bf35f404-4a14-6953-ced2-5bafd21639a0 | `/Game/Equippables/Guns/Rifles/Burst/Standard/AssaultRifle_Burst_Standard_PrimaryAsset` |
| DMR_C | DMR_Standard | 3bf1e8e0-47e8-f27a-6054-929575f41a54 | 414d888a-41ce-fcf0-e545-c49018ec9cf4 | 0f934388-418a-a9e7-42a7-21b27402e46c | `/Game/Equippables/Guns/SniperRifles/DMR/Standard/DMR_Standard_PrimaryAsset` |
| AssaultRifle_ACR_C | AssaultRifle_ACR_Standard | 337cb216-4a6e-d85d-88c2-f29ab317784c | 871e73ed-452d-eb5a-3d6b-1d87060f35ce | 52221ba2-4e4c-ec76-8c81-3483506d5242 | `/Game/Equippables/Guns/Rifles/Carbine/Standard/AssaultRifle_ACR_Standard_PrimaryAsset` |
| AssaultRifle_AK_C | AssaultRifle_AK_Standard | 27f21d97-4c4b-bd1c-1f08-31830ab0be84 | 1ab72e66-4da3-33a0-164f-908113e075a4 | 19629ae1-4996-ae98-7742-24a240d41f99 | `/Game/Equippables/Guns/Rifles/AK/Standard/AssaultRifle_AK_Standard_PrimaryAsset` |
| BattleRifle_C | BattleRifle_Standard | 61d99a36-4033-0fa9-2c94-71aaf901e120 | 45a2214f-4730-8c89-b0a4-a7a2b5125ac8 | 44b01f26-4569-0161-681f-e2a677f317ff | `/Game/Equippables/Guns/Rifles/BattleRifle/Standard/BattleRifle_Standard_PrimaryAsset` |
| LeverSniperRifle_C | LeverSniperRifle_Standard | fd44b2d5-49ee-77ab-fa56-588f3ac0c268 | f0389390-49eb-a43e-27fa-fc9f9f8aa9de | 1afec971-4170-f29b-1c94-07a0eff270ab | `/Game/Equippables/Guns/SniperRifles/Leversniper/Standard/LeverSniperRifle_Standard_PrimaryAsset` |
| DS_Gun_C | DS_Gun_Standard | 740b9572-44b1-57bf-767e-6aa01811f94d | e4d16fc0-47a2-323f-0564-9495418237e8 | 66c8d241-4f7c-6652-3aaa-51bafffbd493 | `/Game/Equippables/Guns/SniperRifles/Doublesniper/Standard/DS_Gun_Standard_PrimaryAsset` |
| BoltSniper_C | BoltSniper_Standard | d1f2920f-469a-3431-ad96-96afbd0017f2 | 88cba358-4f4d-4d0e-69fc-b48f4c65cb2d | 4914f50d-49f9-6424-ca80-9486c45a138d | `/Game/Equippables/Guns/SniperRifles/Boltsniper/Standard/BoltSniper_Standard_PrimaryAsset` |
| LightMachineGun_C | LightMachineGun_Standard | 5305d9c4-4f46-fbf4-9e9a-dea772c263b5 | 0f5f60f4-4c94-e4b2-ceab-e2b4e8b41784 | b33de820-4061-8b85-31ce-808f1a2c58f5 | `/Game/Equippables/Guns/HvyMachineGuns/LMG/Standard/LightMachineGun_Standard_PrimaryAsset` |
| HeavyMachineGun_C | HeavyMachineGun_Standard | f454efd1-49cb-372f-7096-d394df615308 | d91fb318-4e40-b4c9-8c0b-bb9da28bac55 | 2f93861d-4b2f-2175-af0c-3ba0c736e257 | `/Game/Equippables/Guns/HvyMachineGuns/HMG/Standard/HeavyMachineGun_Standard_PrimaryAsset` |

### X.11 `range_melee_equippable`

#### `melee_role`
- call: `build_melee_autonomous_role_body()`
- body: 30 bits
  ```
  a3acf008
  ```
- plain (seed 204): 20 bits `100400` -> encoded `2b3c02`

#### `melee_state_machine_open`
- call: `build_melee_state_machine_open()`
- body: 365 bits
  ```
  020000003a050a3205002e0000008ae2ead2e0e0c2c4d8caa6e8c2e8ca9ac2c6d0d2dcca0000000000ea1428e110
  ```
- plain (seed 204): 9 bits `0000` -> encoded `0e01`

#### `melee_inactive_export`
- call: `build_melee_auth_state_body("InactiveState")`
- body: 317 bits
  ```
  0200000002080a3205001c00000092dcc2c6e8d2eccaa6e8c2e8ca0000000000ea142864ed911e15
  ```
- plain (seed 204): 33 bits `0c02080000` -> encoded `d61ee95101`

`build_melee_auth_state_body(s, e)` == `build_auth_state_body(starting_equippables()[1], s, e)` for every s, e (`test_range_equippables.py:15-24`): True

`build_pistol_auth_state_body(s, e)` == `build_auth_state_body(starting_equippables()[0], s, e)` for every s, e: True

`gun_state_machine_open_body(load)` == `build_machine_open_body(load.actor_guid, load.state_machine_guid, "EquippableStateMachine")`: True

`build_melee_state_machine_open()` == `build_machine_open_body(204, 206, "EquippableStateMachine")`: True

### X.12 Mirror-policy traces

#### BasePistol made current by a correction, then fired (pistol)

| step | predicted | is_current | target | auth after | active | kicked | export? |
|---|---|---|---|---|---|---|---|
| 1 | InactiveState | True | EquipState | EquipState | False | True | True |
| 2 | InactiveState | True | None | EquipState | False | True | None |
| 3 | ReadyingState | True | ReadyingState | ReadyingState | True | True | True |
| 4 | ReadyingState | True | None | ReadyingState | True | True | None |
| 5 | IdleState | True | IdleState | IdleState | True | True | True |
| 6 | FiringState | True | FiringState | FiringState | True | True | True |
| 7 | IdleState | True | IdleState | IdleState | True | True | False |
| 8 | ReloadState | True | ReloadState | ReloadState | True | True | True |
| 9 | IdleState | True | IdleState | IdleState | True | True | False |
| 10 | InactiveState | False | InactiveState | InactiveState | True | True | False |
| 11 | NotAState | False | None | InactiveState | True | True | None |
| 12 | None | False | None | InactiveState | True | True | None |

#### Knife equipped by the player, then put away

| step | predicted | is_current | target | auth after | active | kicked | export? |
|---|---|---|---|---|---|---|---|
| 1 | InactiveState | False | None | InactiveState | False | False | None |
| 2 | ReadyingState | True | ReadyingState | ReadyingState | True | False | True |
| 3 | FirstAttackIdle | True | FirstAttackIdle | FirstAttackIdle | True | False | True |
| 4 | MeleeAttackState1 | True | MeleeAttackState1 | MeleeAttackState1 | True | False | True |
| 5 | FirstAttackIdle | True | FirstAttackIdle | FirstAttackIdle | True | False | False |
| 6 | InactiveState | False | InactiveState | InactiveState | True | False | False |
| 7 | InactiveState | False | None | InactiveState | True | False | None |

#### `resource_target` / `resource_confirmed` (gun magazine, default 25, no refill)

| step | now | predicted | authoritative (client) | target | auth after | baseline after |
|---|---|---|---|---|---|---|
| 1 | 0.00 | 0 | 0 | 25 | 25 | None |
| 2 | 0.05 | 0 | 0 | None | 25 | None |
| 3 | 0.10 | 0 | 25 | None | 25 | 0 |
| 4 | 0.15 | 25 | 25 | None | 25 | 0 |
| 5 | 0.20 | 24 | 25 | 24 | 24 | None |
| 6 | 0.25 | 24 | 25 | None | 24 | None |
| 7 | 0.30 | 24 | 24 | None | 24 | 24 |
| 8 | 0.35 | 23 | 24 | 23 | 23 | None |
| 9 | 0.40 | 25 | 23 | None | 23 | 25 |
| 10 | 0.45 | -1 | 25 | None | 23 | 25 |

#### `resource_target` with `refill_after=4.0` (Cloudburst charges, default 2)

| step | now | predicted | authoritative | target | auth after | baseline after | sent_at after |
|---|---|---|---|---|---|---|---|
| 1 | 0.0 | 0 | 0 | 2 | 2 | None | 0.0 |
| 2 | 0.5 | 2 | 2 | None | 2 | 2 | 0.0 |
| 3 | 2.0 | 1 | 2 | 1 | 1 | None | 2.0 |
| 4 | 2.05 | 1 | 2 | None | 1 | None | 2.0 |
| 5 | 2.1 | 1 | 1 | None | 1 | 1 | 2.0 |
| 6 | 5.0 | 1 | 1 | None | 1 | 1 | 2.0 |
| 7 | 6.05 | 1 | 1 | 2 | 2 | None | 6.05 |
| 8 | 6.1 | 2 | 2 | None | 2 | 2 | 6.05 |
| 9 | 6.15 | 2 | 2 | None | 2 | 2 | 6.05 |
| 10 | 7.0 | 1 | 2 | 1 | 1 | None | 7.0 |
| 11 | 7.05 | 1 | 1 | None | 1 | 1 | 7.0 |
| 12 | 8.0 | 0 | 1 | 0 | 0 | None | 8.0 |
| 13 | 8.05 | 0 | 0 | None | 0 | 0 | 8.0 |
| 14 | 12.1 | 0 | 0 | 2 | 2 | None | 12.1 |

#### `machine_mirror_target` (extra machine, starting `ZoomStateDisabled`, auth = starting)

| step | predicted | target | auth after |
|---|---|---|---|
| 1 | ZoomStateDisabled | None | ZoomStateDisabled |
| 2 | ZoomStateUnzoomed | ZoomStateUnzoomed | ZoomStateUnzoomed |
| 3 | ZoomStateUnzoomed | None | ZoomStateUnzoomed |
| 4 | ZoomStateLevel1 | ZoomStateLevel1 | ZoomStateLevel1 |
| 5 | Unknown | None | ZoomStateLevel1 |
| 6 | None | None | ZoomStateLevel1 |
| 7 | ZoomStateDisabled | ZoomStateDisabled | ZoomStateDisabled |

### X.13 `range_store` helpers

| call | result |
|---|---|
| `resolve_class_path('/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK.AssaultRifle_AK_C', {}, {})` | `'/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK.AssaultRifle_AK_C'` |
| `resolve_class_path('<guid:525>.AssaultRifle_AK_C', {525: '/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK'}, {})` | `'/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK.AssaultRifle_AK_C'` |
| `resolve_class_path('<guid:9>.AssaultRifle_AK_C', {}, {'AssaultRifle_AK_C': '/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK.AssaultRifle_AK_C'})` | `'/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK.AssaultRifle_AK_C'` |
| `resolve_class_path('<guid:9>.AssaultRifle_AK_C', {}, {})` | `None` |
| `resolve_class_path('<guid:525>', {525: '/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK'}, {})` | `None` |
| `resolve_class_path('<guid:x>.AssaultRifle_AK_C', {525: '/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK'}, {})` | `None` |
| `resolve_class_path('', {}, {})` | `None` |
| `resolve_class_path(None, {}, {})` | `None` |
| `split_class_path('/Game/Equippables/Guns/Rifles/BattleRifle/BattleRifle.BattleRifle_C')` | `('/Game/Equippables/Guns/Rifles/BattleRifle/BattleRifle', 'BattleRifle_C', 'Default__BattleRifle_C')` |
| `split_class_path('/Game/Gear/HeavyArmorItem.HeavyArmorItem_C')` | `('/Game/Gear/HeavyArmorItem', 'HeavyArmorItem_C', 'Default__HeavyArmorItem_C')` |
| `split_class_path('BattleRifle_C')` | `ValueError(expected a /Game/<package>.<Class>_C path)` |
| `split_class_path('/Game/X/Y.Z_C')` | `ValueError(class does not match its package)` |
| `split_class_path('/Engine/A/A.A_C')` | `ValueError(expected a /Game/<package>.<Class>_C path)` |
| `split_class_path('/Game/A/B.C.D_C')` | `ValueError(expected a /Game/<package>.<Class>_C path)` |
| `split_class_path('/Game/A/B.B')` | `ValueError(class does not match its package)` |


## Verifier corrections

An independent verifier re-read the Python and regenerated every appendix vector offline, using its own generator against scratch copies of the skin JSONs (state version 46). It checked:

- all 107 bodies, with every plain and encoded rep payload;
- the X.2 table (20 rows) and the X.6 table (20 rows);
- the X.3 sequential and interleaved allocations, and the ability allocator;
- the X.12 traces and the X.13 store helpers;
- the store and attachment tables;
- the builder equivalences.

All of these match byte for byte. The live X.9 bit counts (9 of 9) and the live attachment pre-export sizes (AK 1305, BattleRifle 1569, BoltSniper 1529, Luger 1401, DS 1145, Lever 1257) also match. The 61 unit tests pass. Prose changes:

1. 2.4 and question 1: live default ammo is logged for 13 guns, not only Classic and Vandal. The values are now listed, along with the 7 store guns that have no logged value.
2. 8.1: the purchase-channel overflow was off by one. Purchase N uses channel 19 + N, so the 381st purchase reaches bot channel 400 (the old text said "more than 379").
3. 7.5 and question 4: the `SkinInventory` constructor rewrites the state file whenever `migrated` is true. That includes a merely non-lowercase id in `owned` / `equipped`, not only a legacy-id mapping.
4. 6.6: registration uses `reader._names.get(..., {})`. A missing reader or a read exception (event `purchase_register_error`) counts as a failed try. Gun state names include `InactiveState`; the ability job filters it out.
5. 8.4 and 8.1: added the `find_pawn` pointer chain and the reader's per-address caches. These are never invalidated: an empty TMap read stays cached, and a reused item address keeps a stale class name. Also added `ability_ready_equippables`, the mirror-membership sets and reader-creation retry to the state table.
6. 7.4: the unpackaged-level catalog entry is `SawedOffShotgun_Gothic` (level asset `Slim_Gothic_Lv1_PrimaryAsset1`), not an entry named `Slim_Gothic`.
7. 6.1: fields 157 / 161 still enter the dedup set. `target_slot` / `purchasable_type` are only logged, and the slot comes from the store table. `resolve_class_path` returning None also yields `purchase_unsupported`.
8. 5.2: the extra-machine MirrorState is created with `active=True` only by `send_extra_machines`. The mirror's `setdefault` fallback uses `active=False`, which has no wire effect.
9. 7.2: the A21 builder and the generic builder are byte-identical only on success. A21 keeps its own BasePistol-root checks and its `< 8192` guard, versus the generic `<= 7600`, because its errors are the crash path. Both guards are unreachable with the current catalog: pistol max 6136 bits, knife max 5568, bought guns max 6552.
10. 6.4: the purchase player id comes from `backend.offline_player_id`. The skin-inventory cache ignores the catalog's mtime. The 7600-bit size fallback is unreachable with the current catalog.
11. 8.2: the full in-datagram system order is now spelled out. Added that part C runs only on authenticated, non-handshake datagrams from a `client_restart_sent_sources` source, so the Rust 128 Hz loop must not run these systems on ticks with no such datagram.
12. Questions 12 and 13 are new: overlapping same-slot purchases (a replaced gun's stage 5 can re-register it on its closed channel), and an unobserved Classic purchase that may arrive as `<guid:97>.BasePistol_C` and be dropped.
