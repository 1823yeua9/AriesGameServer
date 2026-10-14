"""guns, shop and skins (docs/port/07_gameplay_guns.md): store helpers, the
purchase allocator, equippable builders, the mirror policies, the gun loader,
attachments, cosmetic opens and saved-skin selection.

every expected value comes from calling the python. skin data is read from
temp copies of skin_catalog.json / offline_skin_state.json, so nothing in
local_backend is ever written (SkinInventory rewrites its state file when a
legacy id migrates). consumer logic that lives inside run_probe closures
(candidate loop, plan_extra_machines, the knife fallback) is copied verbatim
with its range_udp_handshake line numbers.
"""

import copy
import dataclasses
import json
import random
import shutil
import tempfile
from pathlib import Path

# backend.py:36 offline_player_id (importing backend starts its tls setup, so it is copied)
PLAYER = "7c9f41a4-65e2-4d81-a8d5-f879c3e5ab61"

# class paths the client sends in ServerPurchase (07 appendix X.1: live / test / derived)
CLASS_PATHS = {
    "BasePistol_C": "/Game/Equippables/Guns/Sidearms/BasePistol/BasePistol.BasePistol_C",
    "SawedOffShotgun_C": "/Game/Equippables/Guns/Sidearms/Slim/SawedOffShotgun.SawedOffShotgun_C",
    "AutomaticPistol_C": "/Game/Equippables/Guns/Sidearms/AutoPistol/AutomaticPistol.AutomaticPistol_C",
    "LugerPistol_C": "/Game/Equippables/Guns/Sidearms/Luger/LugerPistol.LugerPistol_C",
    "CompactPistol_C": "/Game/Equippables/Guns/Sidearms/Compact/CompactPistol.CompactPistol_C",
    "RevolverPistol_C": "/Game/Equippables/Guns/Sidearms/Revolver/RevolverPistol.RevolverPistol_C",
    "Vector_C": "/Game/Equippables/Guns/SubMachineGuns/Vector/Vector.Vector_C",
    "SubMachineGun_MP5_C": "/Game/Equippables/Guns/SubMachineGuns/MP5/SubMachineGun_MP5.SubMachineGun_MP5_C",
    "PumpShotgun_C": "/Game/Equippables/Guns/Shotguns/PumpShotgun/PumpShotgun.PumpShotgun_C",
    "AutomaticShotgun_C": "/Game/Equippables/Guns/Shotguns/AutoShotgun/AutomaticShotgun.AutomaticShotgun_C",
    "AssaultRifle_Burst_C": "/Game/Equippables/Guns/Rifles/Burst/AssaultRifle_Burst.AssaultRifle_Burst_C",
    "DMR_C": "/Game/Equippables/Guns/SniperRifles/DMR/DMR.DMR_C",
    "AssaultRifle_ACR_C": "/Game/Equippables/Guns/Rifles/Carbine/AssaultRifle_ACR.AssaultRifle_ACR_C",
    "AssaultRifle_AK_C": "/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK.AssaultRifle_AK_C",
    "BattleRifle_C": "/Game/Equippables/Guns/Rifles/BattleRifle/BattleRifle.BattleRifle_C",
    "LeverSniperRifle_C": "/Game/Equippables/Guns/SniperRifles/Leversniper/LeverSniperRifle.LeverSniperRifle_C",
    "DS_Gun_C": "/Game/Equippables/Guns/SniperRifles/Doublesniper/DS_Gun.DS_Gun_C",
    "BoltSniper_C": "/Game/Equippables/Guns/SniperRifles/Boltsniper/BoltSniper.BoltSniper_C",
    "LightMachineGun_C": "/Game/Equippables/Guns/HvyMachineGuns/LMG/LightMachineGun.LightMachineGun_C",
    "HeavyMachineGun_C": "/Game/Equippables/Guns/HvyMachineGuns/HMG/HeavyMachineGun.HeavyMachineGun_C",
    "HeavyArmorItem_C": "/Game/Gear/HeavyArmorItem.HeavyArmorItem_C",
}

# sample live state-machine names (07 X.3: BasePistol order); production reads them from the client
SAMPLE_NAMES = ["InactiveState", "EquipState", "IdleWaitingForEventState", "IdleState", "FiringState",
                "ReloadState", "ReadyingState", "JammedState", "FiringStateBurst"]
ZOOM = {"name": "ZoomStateMachine", "address": 0x2000, "starting": "ZoomStateDisabled",
        "states": ["ZoomStateDisabled", "ZoomStateUnzoomed", "ZoomStateLevel1"]}
MAIN_SM = 0x1000


def hb(pair):
    body, bits = pair
    return {"hex": body.hex(), "bit_count": bits}


def err(error):
    return {"error": type(error).__name__, "message": str(error)}


def attempt(fn, *args, **kwargs):
    try:
        return hb(fn(*args, **kwargs))
    except Exception as error:  # recorded for parity
        return err(error)


def row(*values):
    """one compact json row: the policy tables are large and the exporter indents every list item."""
    return json.dumps(list(values), separators=(",", ":"))


def value(fn):
    try:
        return {"value": fn()}
    except Exception as error:
        return err(error)


def backend_dir():
    import range_store
    return Path(range_store.__file__).resolve().parent


# ---------------------------------------------------------------- dict views

def item_dict(item):
    return {"display": item.display_name, "class": item.class_name, "cost": item.cost,
            "weapon_id": item.weapon_id, "slot": item.slot, "is_gun": item.is_gun}


def load_dict(load):
    out = {"class": load.item.class_name, "package": load.package, "archetype": load.archetype,
           "channel": load.channel, "actor": load.actor_guid, "archetype_guid": load.archetype_guid,
           "package_guid": load.package_guid}
    if hasattr(load, "state_machine_guid"):
        out.update({"key": load.key, "state_machine": load.state_machine_guid, "magazine": load.magazine_guid,
                    "reserve": load.reserve_guid,
                    "cosmetic_pairs": [list(p) for p in load.cosmetic_guid_pairs],
                    "attachment_pairs": [list(p) for p in load.attachment_guid_pairs],
                    "state_guids": [[k, v] for k, v in load.state_guids.items()]})
    return out


def eq_dict(eq):
    return {"key": eq.key, "class": eq.class_name, "channel": eq.channel, "actor": eq.actor_guid,
            "slot": eq.slot, "state_machine": eq.state_machine_guid,
            "states": [[k, v] for k, v in eq.state_guids.items()],
            "resources": [[r.name, r.guid, r.item_offset, r.default_offset, r.refill_after] for r in eq.resources],
            "extra": [extra_dict(m) for m in eq.extra_machines],
            "mirror_states": eq.mirror_states}


def extra_dict(m):
    return {"name": m.name, "guid": m.guid, "starting": m.starting_state,
            "states": [[k, v] for k, v in m.state_guids.items()]}


def cdo_dict(path):
    return {"kind": path.kind, "asset_id": path.asset_id, "source_asset": path.source_asset,
            "package_path": path.package_path, "object_name": path.object_name, "packaged": path.packaged}


def paths_dict(paths):
    if paths is None:
        return None
    return {"weapon_id": paths.weapon_id, "skin": cdo_dict(paths.skin), "level": cdo_dict(paths.level),
            "chroma": cdo_dict(paths.chroma)}


def cosmetic_dict(c):
    if c is None:
        return None
    return dataclasses.asdict(c)


def layout_dict(layout):
    return {"weapon_id": layout.weapon_id, "channel": layout.channel_index, "actor": layout.actor_net_guid,
            "archetype_name": layout.archetype_name, "archetype_package": layout.archetype_package,
            "archetype_guid": layout.archetype_net_guid, "package_guid": layout.package_net_guid,
            "level_guid": layout.level_net_guid, "pointer_handles": list(layout.pointer_handles),
            "uuid_handles": list(layout.uuid_handles), "pairs": [list(p) for p in layout.asset_guid_pairs]}


# ---------------------------------------------------------------- skin data on temp copies

class SkinFiles:
    """catalog + state text written to a fresh temp dir per inventory."""

    def __init__(self, catalog_text, state_text):
        self.catalog_text = catalog_text
        self.state_text = state_text
        self._dirs = []

    def paths(self):
        tmp = tempfile.mkdtemp(prefix="aries_golden_skins_")
        self._dirs.append(tmp)
        catalog = Path(tmp) / "skin_catalog.json"
        state = Path(tmp) / "offline_skin_state.json"
        catalog.write_text(self.catalog_text, encoding="utf-8")
        if self.state_text is not None:
            state.write_text(self.state_text, encoding="utf-8")
        return catalog, state

    def inventory(self, player=PLAYER):
        """(inventory or error, rewritten) - rewritten is python's legacy-migration save."""
        from skin_inventory import SkinInventory
        catalog, state = self.paths()
        try:
            inventory = SkinInventory(player, catalog_path=catalog, state_path=state)
        except Exception as error:
            return error, False
        rewritten = self.state_text is not None and state.read_text(encoding="utf-8") != self.state_text
        return inventory, rewritten

    def close(self):
        for tmp in self._dirs:
            shutil.rmtree(tmp, ignore_errors=True)


def with_inventory(inventory, fn):
    """runs fn with range_gun_loader.cached_skin_inventory pinned to this inventory."""
    import range_gun_loader as rgl
    saved = rgl.cached_skin_inventory
    rgl.cached_skin_inventory = lambda player_id: inventory
    try:
        return fn()
    finally:
        rgl.cached_skin_inventory = saved


def real_skin_files():
    root = backend_dir()
    catalog_text = (root / "skin_catalog.json").read_text(encoding="utf-8")
    state_text = (root / "offline_skin_state.json").read_text(encoding="utf-8")
    return catalog_text, state_text


def saved_inventory():
    catalog_text, state_text = real_skin_files()
    files = SkinFiles(catalog_text, state_text)
    inventory, rewritten = files.inventory()
    if isinstance(inventory, Exception):
        raise inventory
    return files, inventory, rewritten


# ---------------------------------------------------------------- store

def section_store():
    import range_store as rst

    out = {"slots": [rst.slot_primary, rst.slot_secondary, rst.slot_armor],
           "items": [item_dict(i) for i in rst._items], "split": [], "item_for_path": [], "resolve": []}
    paths = list(CLASS_PATHS.values()) + [
        "BattleRifle_C", "/Game/X/Y.Z_C", "/Engine/A/A.A_C", "/Game/A/B.C.D_C", "/Game/A/B.B", "",
        "/Game/", "/Game/A/A.A_C", "/Game/A.A_C", "/Game/._C", "/Game/A/._C", "/game/A/A.A_C",
        "/Game/A/A.a_c", "/Game/A/A.A_c", "/Game/A/BA.A_C", "/Game/A/A_C.A_C", "/Game/A/A.A_C.",
        "/Game/Ü/Ü.Ü_C", "/Game/Gear/LightArmorItem.LightArmorItem_C", "/Game/Gear/Thing.Thing_C",
        "/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK.AssaultRifle_ACR_C",
        "/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK/AssaultRifle_AK.AssaultRifle_AK_C",
        "/Game/A/A.A_C ", " /Game/A/A.A_C", "/Game//A.A_C", "/Game/A/_C._C",
    ]
    for path in paths:
        try:
            out["split"].append({"path": path, "ok": list(rst.split_class_path(path))})
        except Exception as error:
            out["split"].append({"path": path, **err(error)})
        try:
            item = rst.store_item_for_path(path)
            out["item_for_path"].append({"path": path, "class": item.class_name if item else None})
        except Exception as error:
            out["item_for_path"].append({"path": path, **err(error)})

    ak_package = "/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK"
    ak_path = CLASS_PATHS["AssaultRifle_AK_C"]
    pkg = {525: ak_package, 591: "/Game/Equippables/Guns/Rifles/Carbine/AssaultRifle_ACR"}
    seen = {"AssaultRifle_AK_C": ak_path, "BattleRifle_C": CLASS_PATHS["BattleRifle_C"]}
    resolves = [
        (ak_path, {}, {}), ("<guid:525>.AssaultRifle_AK_C", {525: ak_package}, {}),
        ("<guid:9>.AssaultRifle_AK_C", {}, {"AssaultRifle_AK_C": ak_path}), ("<guid:9>.AssaultRifle_AK_C", {}, {}),
        ("<guid:525>", {525: ak_package}, {}), ("<guid:x>.AssaultRifle_AK_C", {525: ak_package}, {}),
        ("", {}, {}), (None, {}, {}),
        ("<guid:591>.AssaultRifle_ACR_C", pkg, seen), ("<guid:591>.Anything_C", pkg, seen),
        ("<guid:9>.BattleRifle_C", pkg, seen), ("<guid:525>.BattleRifle_C", pkg, seen),
        ("<guid: 525 >.AssaultRifle_AK_C", pkg, {}), ("<guid:+525>.AssaultRifle_AK_C", pkg, {}),
        ("<guid:-525>.AssaultRifle_AK_C", pkg, seen), ("<guid:-525>.BattleRifle_C", pkg, seen),
        ("<guid:5_25>.AssaultRifle_AK_C", pkg, {}), ("<guid:5__25>.AssaultRifle_AK_C", pkg, seen),
        ("<guid:_525>.AssaultRifle_AK_C", pkg, seen), ("<guid:525_>.AssaultRifle_AK_C", pkg, seen),
        ("<guid:0525>.AssaultRifle_AK_C", pkg, {}), ("<guid:525>>.AssaultRifle_AK_C", pkg, {}),
        ("<guid:525>>>.AssaultRifle_AK_C", pkg, {}), ("<guid:525>.", pkg, seen), ("<guid:>.BattleRifle_C", pkg, seen),
        ("<guid:525", pkg, seen), ("<guid:525>.X>.Y", pkg, seen), ("<guid:\t525\n>.AssaultRifle_AK_C", pkg, {}),
        ("<guid:\x1c525>.AssaultRifle_AK_C", pkg, {}), ("<guid:　525>.AssaultRifle_AK_C", pkg, {}),
        # int() strips unicode White_Space only (not \x1c-\x1f, not zero-width space)
        ("<guid:\x1f525>.AssaultRifle_AK_C", pkg, {}), ("<guid:\x0b525\x0c>.AssaultRifle_AK_C", pkg, {}),
        ("<guid:\x85525\xa0>.AssaultRifle_AK_C", pkg, {}), ("<guid: 525 >.AssaultRifle_AK_C", pkg, {}),
        ("<guid:​525>.AssaultRifle_AK_C", pkg, {}), ("<guid:᠎525>.AssaultRifle_AK_C", pkg, {}),
        ("<guid:- 525>.BattleRifle_C", pkg, seen), ("<guid:+_525>.BattleRifle_C", pkg, seen), ("<guid:->.BattleRifle_C", pkg, seen),
        ("<guid:99999999999999999999999999999999>.BattleRifle_C", pkg, seen),
        ("<guid:" + "9" * 4300 + ">.BattleRifle_C", pkg, seen), ("<guid:" + "9" * 4301 + ">.BattleRifle_C", pkg, seen),
        ("<guid:1e3>.BattleRifle_C", pkg, seen), ("<guid:0x20b>.BattleRifle_C", pkg, seen),
        ("<GUID:525>.AssaultRifle_AK_C", pkg, seen), ("guid:525.AssaultRifle_AK_C", pkg, seen),
        ("<guid:4294967296>.BattleRifle_C", {4294967296 % (1 << 32): "/Game/Wrap"}, seen),
        ("<guid:525>.<guid:9>.X", pkg, seen),
        # int() takes any unicode decimal digit (Nd); the text can be a client FString name
        ("<guid:٥٢٥>.AssaultRifle_AK_C", pkg, {}), ("<guid:５２５>.AssaultRifle_AK_C", pkg, {}),
        ("<guid:5_٢٥>.AssaultRifle_AK_C", pkg, {}), ("<guid:　\U0001d7d3\U0001d7d0\U0001d7d3 >.AssaultRifle_AK_C", pkg, {}),
        ("<guid:-٥٢٥>.BattleRifle_C", pkg, seen), ("<guid:²>.BattleRifle_C", pkg, seen),
        ("<guid:᧚>.BattleRifle_C", pkg, seen), ("<guid:٥_>.BattleRifle_C", pkg, seen),
        ("<guid:−525>.BattleRifle_C", pkg, seen), ("<guid:5　5>.BattleRifle_C", pkg, seen),
        ("<guid:\U0001fbf5\U0001fbf2\U0001fbf5>.AssaultRifle_AK_C", pkg, {}), ("<guid:༩༠>.BattleRifle_C", pkg, seen),
        ("<guid:" + "9_" * 4299 + "9>.BattleRifle_C", pkg, seen), ("<guid:" + "9_" * 4300 + "9>.BattleRifle_C", pkg, seen),
    ]
    for item_path, package_guids, seen_paths in resolves:
        out["resolve"].append({"item_path": item_path,
                               "package_guids": [[k, v] for k, v in package_guids.items()],
                               "seen_paths": [[k, v] for k, v in seen_paths.items()],
                               "result": rst.resolve_class_path(item_path, package_guids, seen_paths)})
    return out


# ---------------------------------------------------------------- allocator

def section_alloc():
    import range_gun_loader as rgl
    import range_store as rst

    out = {"first": [rgl.first_purchase_channel, rgl.first_purchase_guid, rgl.setup_bunch_count],
           "fresh": [], "raw": []}
    for class_name, path in CLASS_PATHS.items():
        item = rst.store_items[class_name]
        alloc = rgl.PurchaseAllocator()
        if item.is_gun:
            load = rgl.plan_gun_load(path, item, alloc)
            planned = alloc.next_guid
            inactive = rgl.gun_equippable(load, alloc).state_guids["InactiveState"]
        else:
            load = rgl.plan_item_load(path, item, alloc)
            planned, inactive = alloc.next_guid, None
        out["fresh"].append({"path": path, "load": load_dict(load), "next_guid_after_plan": planned,
                             "inactive": inactive, "next_channel": alloc.next_channel, "next_guid": alloc.next_guid})

    rng = random.Random(1307)
    for start_channel, start_guid in ((20, 520), (0, 100000), (5, 7), (0, 0), (1, 1), (379, 4095), (20, 16383)):
        alloc = rgl.PurchaseAllocator(start_channel, start_guid)
        steps = []
        for _ in range(48):
            op = rng.choice(("channel", "dynamic", "static"))
            result = {"channel": alloc.channel, "dynamic": alloc.dynamic_guid, "static": alloc.static_pair}[op]()
            steps.append([op, list(result) if isinstance(result, tuple) else result,
                          alloc.next_channel, alloc.next_guid])
        out["raw"].append({"start": [start_channel, start_guid], "steps": steps})
    return out


# ---------------------------------------------------------------- equippables builders

def section_equippables():
    import range_equippables as req
    import range_melee_equippable as rme
    import range_server_bunch as rsb

    pistol, knife = req.starting_equippables()
    out = {"starting": [eq_dict(e) for e in req.starting_equippables()],
           "abilities": [eq_dict(e) for e in req.starting_ability_resources()],
           "armor": [[k, v] for k, v in req.armor_shield_values.items()],
           "pistol_state_guids": [[k, v] for k, v in rsb.base_pistol_state_guids.items()],
           "melee": {"channel": rme.melee_channel, "actor": rme.melee_actor_guid,
                     "state_machine": rme.melee_state_machine_guid,
                     "states": [[k, v] for k, v in rme.melee_state_guids.items()],
                     "role": attempt(rme.build_melee_autonomous_role_body),
                     "sm_open": attempt(rme.build_melee_state_machine_open),
                     "auth": []},
           "auth": [], "machine_open": [], "machine_state": [], "resource": [], "shield": [], "validate": []}

    for eq in (pistol, knife):
        for state in list(eq.state_guids) + ["NotAState"]:
            for export in (True, False):
                out["auth"].append({"eq": eq.key, "state": state, "export": export,
                                    **attempt(req.build_auth_state_body, eq, state, export)})
    for state in list(rme.melee_state_guids) + ["NotAState"]:
        for export in (True, False):
            out["melee"]["auth"].append({"state": state, "export": export,
                                         **attempt(rme.build_melee_auth_state_body, state, export)})

    for actor, machine, name in ((94, 96, "EquippableStateMachine"), (204, 206, "EquippableStateMachine"),
                                 (520, 554, "ZoomStateMachine"), (520, 528, "EquippableStateMachine"),
                                 (208, 100000, "EquippableStateMachine"), (16384, 16386, "M"),
                                 (2, 4294967295, "EquippableStateMachine")):
        out["machine_open"].append({"actor": actor, "machine": machine, "name": name,
                                    **attempt(req.build_machine_open_body, actor, machine, name)})

    vandal = req.EquippableActor(
        key="AssaultRifle_AK_C@20", class_name="AssaultRifle_AK_C", channel=20, actor_guid=520, slot=0,
        state_machine_guid=528, state_guids={"InactiveState": 552, "EquipState": 562},
        resources=(req.ResourceSubobject("MagazineAmmo", 530, 0x1208), req.ResourceSubobject("ReserveAmmo", 532, 0x1200)),
        extra_machines=(req.ExtraStateMachine("ZoomStateMachine", 554, "ZoomStateDisabled",
                                              {"ZoomStateDisabled": 556, "ZoomStateUnzoomed": 558,
                                               "ZoomStateLevel1": 560}),))
    out["vandal"] = eq_dict(vandal)
    for state in ["ZoomStateDisabled", "ZoomStateUnzoomed", "ZoomStateLevel1", "Nope"]:
        for export in (True, False):
            out["machine_state"].append({"state": state, "export": export,
                                         **attempt(req.build_machine_state_body, vandal, vandal.extra_machines[0],
                                                   state, export)})
    for state in ("InactiveState", "EquipState"):
        out["auth"].append({"eq": "vandal", "state": state, "export": True,
                            **attempt(req.build_auth_state_body, vandal, state, True)})

    resource_owners = {"BasePistol": pistol, "vandal": vandal}
    for index, ability in enumerate(req.starting_ability_resources()):
        resource_owners[ability.key] = ability
    amounts = [(("BasePistol", 0), (12, 11, 0, 1, 127, 128, 16383, 16384, (1 << 31) - 1, 1 << 31, -1)),
               (("BasePistol", 1), (36, 35)), (("vandal", 0), (25, 24)), (("vandal", 1), (50, 49))]
    for ability in req.starting_ability_resources():
        amounts.append(((ability.key, 0), (0, 1, 2)))
    for (owner, index), values in amounts:
        eq = resource_owners[owner]
        for amount in values:
            for export in (True, False):
                out["resource"].append({"eq": owner, "resource": index, "amount": amount, "export": export,
                                        **attempt(req.build_resource_amount_body, eq, eq.resources[index], amount,
                                                  export)})

    for life, maximum in ((25.0, 25.0), (50.0, 50.0), (0.0, 0.0), (0.0, 1000.0), (1000.0, 1000.0), (12.5, 50.0),
                          (0.1, 0.1), (1e-45, 1e-45), (33.333333333333336, 100.0), (-1.0, 25.0), (30.0, 25.0),
                          (0.0, 1000.5), (-0.0, 0.0), (999.9999999999999, 1000.0)):
        for export in (True, False):
            out["shield"].append({"life": life, "maximum": maximum, "export": export,
                                  **attempt(req.build_shield_section_body, life, maximum, export)})

    for kwargs in ({"actor_guid": 0}, {"actor_guid": 95}, {"actor_guid": -2}, {"slot": 16}, {"slot": -1},
                   {"slot": 15}, {"state_guids": {"EquipState": 1}}, {"state_guids": {}}, {}):
        args = dict(key="K", class_name="K_C", channel=30, actor_guid=600, slot=3, state_machine_guid=602,
                    state_guids={"InactiveState": 604})
        args.update(kwargs)
        try:
            req.EquippableActor(**args)
            result = {"ok": True}
        except Exception as error:
            result = err(error)
        out["validate"].append({"args": {k: ([[a, b] for a, b in v.items()] if isinstance(v, dict) else v)
                                         for k, v in args.items()}, **result})
    return out


# ---------------------------------------------------------------- mirror policy

def section_mirror():
    import range_equippables as req

    pistol, knife = req.starting_equippables()
    resource_only = req.starting_ability_resources()[0]
    no_equip = req.EquippableActor(key="NoEquip", class_name="NoEquip_C", channel=30, actor_guid=600, slot=3,
                                   state_machine_guid=602,
                                   state_guids={"InactiveState": 604, "ReadyingState": 606, "IdleState": 608})
    equippables = {"BasePistol": pistol, "Melee": knife, "resource_only": resource_only, "no_equip": no_equip}
    out = {"equippables": {k: eq_dict(v) for k, v in equippables.items()},
           "mirror_table": [], "mirror_traces": [], "machine_table": [], "machine_traces": [],
           "resource_table": [], "resource_traces": []}

    knife_auths = ["InactiveState", "EquipState", "ReadyingState", "FirstAttackIdle", "WaitForComboInput5",
                   "NotAState"]
    for name, eq in equippables.items():
        auths = knife_auths if name == "Melee" else list(eq.state_guids) + ["NotAState"]
        predicted_values = [None] + (knife_auths[:-1] + ["MeleeAttackState1"] if name == "Melee"
                                     else list(eq.state_guids)) + ["NotAState", ""]
        for auth in auths:
            for active in (False, True):
                for kicked in (False, True):
                    for predicted in predicted_values:
                        for is_current in (False, True):
                            st = req.MirrorState(auth=auth, exported={"InactiveState"}, active=active, kicked=kicked)
                            target = req.mirror_target(eq, st, predicted, is_current)
                            out["mirror_table"].append(row(name, auth, active, kicked, predicted, is_current,
                                                           target, st.auth, st.active, st.kicked))

    def mirror_trace(name, steps):
        eq = equippables[name]
        st = req.MirrorState()
        rows = []
        for predicted, is_current in steps:
            target = req.mirror_target(eq, st, predicted, is_current)
            export = None
            if target is not None:
                # ruh:1704-1708
                export = target not in st.exported
                st.exported.add(target)
                st.auth = target
            rows.append(row(predicted, is_current, target, export, st.auth, st.active, st.kicked,
                            sorted(st.exported)))
        out["mirror_traces"].append({"eq": name, "rows": rows})

    # 07 X.12 traces
    mirror_trace("BasePistol", [("InactiveState", True), ("InactiveState", True), ("ReadyingState", True),
                                ("ReadyingState", True), ("IdleState", True), ("FiringState", True),
                                ("IdleState", True), ("ReloadState", True), ("IdleState", True),
                                ("InactiveState", False), ("NotAState", False), (None, False)])
    mirror_trace("Melee", [("InactiveState", False), ("ReadyingState", True), ("FirstAttackIdle", True),
                           ("MeleeAttackState1", True), ("FirstAttackIdle", True), ("InactiveState", False),
                           ("InactiveState", False)])
    rng = random.Random(70)
    for name, eq in equippables.items():
        pool = list(eq.state_guids) + [None, "NotAState"]
        for _ in range(3):
            steps = []
            for _ in range(40):
                predicted = "InactiveState" if rng.random() < 0.3 else rng.choice(pool)
                steps.append((predicted, rng.random() < 0.5))
            mirror_trace(name, steps)

    zoom = {"ZoomStateDisabled": 556, "ZoomStateUnzoomed": 558, "ZoomStateLevel1": 560}
    out["zoom"] = [[k, v] for k, v in zoom.items()]
    for auth in list(zoom) + ["Other"]:
        for predicted in [None, "Unknown", ""] + list(zoom):
            st = req.MirrorState(auth=auth, exported={auth})
            out["machine_table"].append(row(auth, predicted, req.machine_mirror_target(st, predicted, zoom)))

    def machine_trace(steps):
        st = req.MirrorState(auth="ZoomStateDisabled", exported={"ZoomStateDisabled"}, active=True)
        rows = []
        for predicted in steps:
            target = req.machine_mirror_target(st, predicted, zoom)
            export = None
            if target is not None:
                # ruh:1691-1696
                export = target not in st.exported
                st.exported.add(target)
                st.auth = target
            rows.append(row(predicted, target, export, st.auth, sorted(st.exported)))
        out["machine_traces"].append(rows)

    machine_trace(["ZoomStateDisabled", "ZoomStateUnzoomed", "ZoomStateUnzoomed", "ZoomStateLevel1", "Unknown",
                   None, "ZoomStateDisabled"])
    for _ in range(4):
        machine_trace([rng.choice([None, "Unknown"] + list(zoom)) for _ in range(30)])

    # single resource_target calls over a sampled domain (state before -> decision, state after)
    domain = {"auth": [None, 0, 1, 2, 24, 25, 9999, 10000], "baseline": [None, 0, 1, 2, 25, -1],
              "sent_at": [0.0, 2.0, 6.05], "predicted": [-1, 0, 1, 2, 24, 25, 9999, 10000],
              "authoritative": [-1, 0, 1, 2, 25, 9999], "default": [-1, 0, 1, 2, 25, 9999, 10000],
              "now": [0.0, 3.99, 4.0, 6.0, 8.05, 10.05, 12.1], "refill": [None, 4.0, 0.0]}
    seen = set()
    while len(out["resource_table"]) < 1500:
        pick = {k: rng.choice(v) for k, v in domain.items()}
        key = tuple(pick.values())
        if key in seen:
            continue
        seen.add(key)
        st = req.ResourceMirrorState(auth=pick["auth"], baseline=pick["baseline"], sent_at=pick["sent_at"])
        target = req.resource_target(st, pick["predicted"], pick["authoritative"], pick["default"], pick["now"],
                                     pick["refill"])
        out["resource_table"].append(row(pick["auth"], pick["baseline"], pick["sent_at"], pick["predicted"],
                                         pick["authoritative"], pick["default"], pick["now"], pick["refill"],
                                         target, st.auth, st.baseline, st.sent_at))

    def resource_trace(default, refill, steps):
        st = req.ResourceMirrorState()
        rows = []
        for now, predicted, authoritative in steps:
            target = req.resource_target(st, predicted, authoritative, default, now, refill)
            export = None
            if target is not None:
                # ruh:1660-1664
                export = st.auth is None
                req.resource_confirmed(st, target, now)
            rows.append(row(now, predicted, authoritative, target, export, st.auth, st.baseline, st.sent_at))
        out["resource_traces"].append({"default": default, "refill": refill, "rows": rows})

    # 07 X.12 timelines
    resource_trace(25, None, [(0.0, 0, 0), (0.05, 0, 0), (0.10, 0, 25), (0.15, 25, 25), (0.20, 24, 25),
                              (0.25, 24, 25), (0.30, 24, 24), (0.35, 23, 24), (0.40, 25, 23), (0.45, -1, 25)])
    resource_trace(2, 4.0, [(0.0, 0, 0), (0.5, 2, 2), (2.0, 1, 2), (2.05, 1, 2), (2.1, 1, 1), (5.0, 1, 1),
                            (6.05, 1, 1), (6.1, 2, 2), (6.15, 2, 2), (7.0, 1, 2), (7.05, 1, 1), (8.0, 0, 1),
                            (8.05, 0, 0), (12.1, 0, 0)])
    # scripted client: applies our last value after a delay, shoots and reloads at random (50 ms polls)
    for default, refill, seed in ((25, None, 1), (12, None, 2), (2, 4.0, 3), (1, 4.0, 4), (36, None, 5),
                                  (10000, None, 6), (-1, 4.0, 7), (0, 4.0, 8)):
        trng = random.Random(seed)
        st = req.ResourceMirrorState()
        client_auth, predicted, pending = 0, 0, []
        now = round(trng.uniform(0.0, 1.0), 3)
        rows = []
        for _ in range(120):
            now = round(now + 0.05 + trng.choice((0.0, 0.0, 0.001, 0.013)), 3)
            applied = [p for p in pending if p[0] <= now]
            pending = [p for p in pending if p[0] > now]
            for _, amount in applied:
                client_auth = amount
                if trng.random() < 0.7:
                    predicted = amount
            roll = trng.random()
            if roll < 0.15 and predicted > 0:
                predicted -= 1
            elif roll < 0.18:
                predicted = default if default >= 0 else 0
            elif roll < 0.2:
                predicted = trng.choice((-1, 10000, 9999))
            target = req.resource_target(st, predicted, client_auth, default, now, refill)
            export = None
            if target is not None:
                export = st.auth is None
                req.resource_confirmed(st, target, now)
                pending.append((round(now + trng.choice((0.02, 0.06, 0.11)), 3), target))
            rows.append(row(now, predicted, client_auth, target, export, st.auth, st.baseline, st.sent_at))
        out["resource_traces"].append({"default": default, "refill": refill, "rows": rows})
    return out


# ---------------------------------------------------------------- gun loader

def candidate_open(load, saved, standard):
    """ruh:1228-1248 candidate loop (selection errors are passed in as exceptions)."""
    import range_gun_loader as rgl
    candidates = []
    errors = []
    for found in (saved, standard):
        if isinstance(found, Exception):
            errors.append(type(found).__name__)
            continue
        if found is not None:
            candidates.append(found)
    candidates.append(None)
    for cosmetics in candidates:
        try:
            open_body, open_bits = rgl.gun_open_body(load, cosmetics)
            break
        except ValueError as error:
            errors.append(type(error).__name__)
    return {"skin": cosmetics.skin.asset_id if cosmetics else None, "errors": errors,
            **hb((open_body, open_bits))}


def select(fn, *args):
    try:
        return fn(*args)
    except Exception as error:
        return error


def selection_dict(found):
    if isinstance(found, Exception):
        return err(found)
    return {"value": paths_dict(found)}


def section_loader():
    import range_attachments as rat
    import range_gun_loader as rgl
    import range_store as rst

    files, inventory, rewritten = saved_inventory()
    try:
        out = {"state_rewritten": rewritten, "guns": [], "armor": [], "plan_errors": [], "sequences": []}
        for class_name, path in CLASS_PATHS.items():
            item = rst.store_items[class_name]
            if not item.is_gun:
                continue
            alloc = rgl.PurchaseAllocator()
            load = rgl.plan_gun_load(path, item, alloc)
            assets = rat.forced_attachments(class_name)
            saved = with_inventory(inventory, lambda: select(rgl.select_saved_cdo_paths, PLAYER, item.weapon_id))
            standard = with_inventory(inventory, lambda: select(rgl.standard_cdo_paths, PLAYER, item.weapon_id))
            gun = {"path": path, "load": load_dict(load),
                   "attachments": [[a.package_path, a.cdo_name, list(a.uuid_words)] for a in assets],
                   "pre_export": attempt(rat.attachment_pre_export_body, assets, load.attachment_guid_pairs),
                   "open_unskinned": attempt(rgl.gun_open_body, load, None),
                   "saved": selection_dict(saved), "standard": selection_dict(standard),
                   "open_saved": attempt(rgl.gun_open_body, load, saved) if saved and not isinstance(saved, Exception) else None,
                   "open_standard": attempt(rgl.gun_open_body, load, standard) if standard and not isinstance(standard, Exception) else None,
                   "chosen": candidate_open(load, saved, standard),
                   "chosen_unskinned_only": candidate_open(load, None, None),
                   "owner": attempt(rgl.gun_owner_body, load), "role": attempt(rgl.gun_role_body, load),
                   "sm_open": attempt(rgl.gun_state_machine_open_body, load)}
            gun["initial"] = attempt(rgl.gun_initial_state_body, load, alloc)
            gun["inactive"] = load.state_guids["InactiveState"]
            gun["next_guid"] = alloc.next_guid
            if assets:
                broken = dataclasses.replace(load, attachment_guid_pairs=load.attachment_guid_pairs[:-1])
                gun["open_missing_pair"] = attempt(rgl.gun_open_body, broken, None)
            out["guns"].append(gun)

        for class_name in ("HeavyArmorItem_C",):
            alloc = rgl.PurchaseAllocator()
            load = rgl.plan_item_load(CLASS_PATHS[class_name], rst.store_items[class_name], alloc)
            out["armor"].append({"path": CLASS_PATHS[class_name], "load": load_dict(load),
                                 "open": attempt(rgl.item_open_body, load), "next_guid": alloc.next_guid})

        for path, class_name, kind in ((CLASS_PATHS["HeavyArmorItem_C"], "HeavyArmorItem_C", "gun"),
                                       (CLASS_PATHS["AssaultRifle_AK_C"], "AssaultRifle_AK_C", "item"),
                                       (CLASS_PATHS["AssaultRifle_AK_C"], "AssaultRifle_ACR_C", "gun"),
                                       (CLASS_PATHS["HeavyArmorItem_C"], "LightArmorItem_C", "item"),
                                       ("AssaultRifle_AK_C", "AssaultRifle_AK_C", "gun"),
                                       ("/Game/X/Y.Z_C", "HeavyArmorItem_C", "item")):
            alloc = rgl.PurchaseAllocator()
            plan = rgl.plan_gun_load if kind == "gun" else rgl.plan_item_load
            try:
                plan(path, rst.store_items[class_name], alloc)
                result = {"ok": True}
            except Exception as error:
                result = err(error)
            out["plan_errors"].append({"path": path, "class": class_name, "kind": kind, **result,
                                       "next_channel": alloc.next_channel, "next_guid": alloc.next_guid})

        # gun_equippable without an allocator before InactiveState exists
        alloc = rgl.PurchaseAllocator()
        load = rgl.plan_gun_load(CLASS_PATHS["RevolverPistol_C"], rst.store_items["RevolverPistol_C"], alloc)
        try:
            rgl.gun_equippable(load, None, SAMPLE_NAMES)
            out["equippable_without_allocator"] = {"ok": True}
        except Exception as error:
            out["equippable_without_allocator"] = err(error)

        scenarios = {
            # 07 X.3 sequential example
            "sequential": [("plan", "AssaultRifle_AK_C"), ("stage3", 0),
                           ("stage5", 0, SAMPLE_NAMES, [ZOOM]), ("plan", "HeavyArmorItem_C"),
                           ("plan", "AssaultRifle_ACR_C")],
            # 07 X.3 interleaved: armor planned before the vandal reaches stage 3
            "interleaved": [("plan", "AssaultRifle_AK_C"), ("plan", "HeavyArmorItem_C"), ("stage3", 0),
                            ("stage5", 0, SAMPLE_NAMES, [ZOOM])],
            "five_in_a_row": [("plan", "AssaultRifle_AK_C"), ("plan", "RevolverPistol_C"), ("stage3", 0),
                              ("plan", "AssaultRifle_ACR_C"), ("stage3", 1), ("stage5", 0, SAMPLE_NAMES, [ZOOM]),
                              ("plan", "HeavyArmorItem_C"), ("stage3", 2),
                              ("stage5", 1, SAMPLE_NAMES[:8], []),
                              ("stage5", 2, SAMPLE_NAMES + ["ZoomFiringState"],
                               [{"name": "EquippableStateMachine", "address": MAIN_SM, "starting": "InactiveState",
                                 "states": SAMPLE_NAMES},
                                ZOOM,
                                {"name": "NoStartMachine", "address": 0x3000, "starting": None, "states": ["A"]},
                                {"name": "EmptyStartMachine", "address": 0x3100, "starting": "", "states": ["A"]},
                                {"name": "DupMachine", "address": 0x3200, "starting": "B", "states": ["B", "C", "B"]}]),
                              ("plan", "BoltSniper_C"), ("plan", "SubMachineGun_MP5_C"), ("stage3", 5),
                              ("stage3", 4), ("plan", "LugerPistol_C"), ("stage3", 6),
                              ("stage5", 6, ["EquipState", "InactiveState", "IdleState"], [])],
        }
        for name, steps in scenarios.items():
            alloc = rgl.PurchaseAllocator()
            loads = []
            rows = []
            for step in steps:
                row = {"op": step[0]}
                if step[0] == "plan":
                    item = rst.store_items[step[1]]
                    path = CLASS_PATHS[step[1]]
                    row["path"] = path
                    if item.is_gun:
                        load = rgl.plan_gun_load(path, item, alloc)
                        row["pre_export"] = attempt(rat.attachment_pre_export_body,
                                                    rat.forced_attachments(item.class_name),
                                                    load.attachment_guid_pairs)
                        row["open"] = attempt(rgl.gun_open_body, load, None)
                    else:
                        load = rgl.plan_item_load(path, item, alloc)
                        row["open"] = attempt(rgl.item_open_body, load)
                    loads.append(load)
                    row["load"] = load_dict(load)
                elif step[0] == "stage3":
                    load = loads[step[1]]
                    row["load_index"] = step[1]
                    row["body"] = attempt(rgl.gun_initial_state_body, load, alloc)
                    row["state_guids"] = [[k, v] for k, v in load.state_guids.items()]
                else:
                    import range_equippables as req
                    _, index, names, machines = step
                    load = loads[index]
                    row.update({"load_index": index, "names": names, "machines": machines, "main_sm": MAIN_SM})
                    # ruh:1113-1118 plan_extra_machines (closure inside run_probe)
                    extra = [req.ExtraStateMachine(
                                 name=m["name"], guid=alloc.dynamic_guid(), starting_state=m["starting"],
                                 state_guids={state: alloc.dynamic_guid() for state in m["states"]})
                             for m in machines if m["address"] != MAIN_SM and m["starting"]]
                    eq = rgl.gun_equippable(load, alloc, names, extra)
                    row["equippable"] = eq_dict(eq)
                    # ruh:1122-1126 send_extra_machines
                    row["extra_bodies"] = [[attempt(req.build_machine_open_body, eq.actor_guid, m.guid, m.name),
                                            attempt(req.build_machine_state_body, eq, m, m.starting_state, True)]
                                           for m in eq.extra_machines]
                    row["auth"] = [[state, attempt(req.build_auth_state_body, eq, state, state != "InactiveState")]
                                   for state in eq.state_guids]
                    row["resources"] = [attempt(req.build_resource_amount_body, eq, r, amount, True)
                                        for r, amount in zip(eq.resources, (25, 50))]
                row["next_channel"] = alloc.next_channel
                row["next_guid"] = alloc.next_guid
                rows.append(row)
            out["sequences"].append({"name": name, "steps": rows})
        return out
    finally:
        files.close()


# ---------------------------------------------------------------- attachments

def section_attachments():
    import range_attachments as rat
    from range_server_bunch import BitWriter

    out = {"handles": [rat.attachment_assets_handle, rat.attachment_asset_ids_handle],
           "guard": rat.gun_open_body_guard_bits,
           "table": [[k, [[a.package_path, a.cdo_name, list(a.uuid_words)] for a in v]]
                     for k, v in rat.FORCED_ATTACHMENTS.items()],
           "lookup": [[c, len(rat.forced_attachments(c))] for c in list(CLASS_PATHS) + ["Nope_C", ""]],
           "object_arrays": [], "guid_arrays": [], "pre_export": []}
    for handle, guids in ((20, [547]), (20, [547, 551]), (20, []), (3, [1, 127, 128, 16383, 16384, 0xFFFFFFFF]),
                          (200, list(range(1, 40)))):
        w = BitWriter()
        rat.append_object_array_property(w, handle, guids)
        out["object_arrays"].append({"handle": handle, "guids": guids, "hex": w.to_bytes().hex(),
                                     "bit_count": w.bit_count})
    words = [list(a.uuid_words) for v in rat.FORCED_ATTACHMENTS.values() for a in v]
    for handle, chunk in ((40, words[:1]), (40, words[:2]), (40, []), (40, words), (130, words[3:7])):
        w = BitWriter()
        rat.append_guid_array_property(w, handle, chunk)
        out["guid_arrays"].append({"handle": handle, "words": chunk, "hex": w.to_bytes().hex(),
                                   "bit_count": w.bit_count})
    acr = rat.forced_attachments("AssaultRifle_ACR_C")
    ak = rat.forced_attachments("AssaultRifle_AK_C")
    for cls, pairs in (("AssaultRifle_ACR_C", [(547, 549), (551, 553)]), ("AssaultRifle_AK_C", [(547, 549)]),
                       ("AssaultRifle_AK_C", [(613, 615)]), ("AssaultRifle_ACR_C", [(547, 549)]),
                       ("AssaultRifle_AK_C", []), ("BasePistol_C", []), ("DS_Gun_C", [(0xFFFFFFFF, 1)])):
        assets = {"AssaultRifle_ACR_C": acr, "AssaultRifle_AK_C": ak}.get(cls, rat.forced_attachments(cls))
        out["pre_export"].append({"class": cls, "pairs": [list(p) for p in pairs],
                                  **attempt(rat.attachment_pre_export_body, assets, pairs)})
    return out


# ---------------------------------------------------------------- cosmetics

def section_cosmetics():
    import range_melee_cosmetics as rmc
    import range_server_bunch as rsb
    import range_weapon_cosmetics as rwc
    import test_range_weapon_cosmetics as twc
    from equipped_cosmetic import CosmeticCdoPath, EquippedCdoPaths

    out = {"words": [], "layouts": {"base_pistol": layout_dict(rwc.base_pistol_cosmetic_layout),
                                    "melee": layout_dict(rmc.starting_melee_cosmetic_layout)},
           "opens": [], "pistol_initial": [], "plain": {}}
    for text in ("17f0db96-4e2d-6bfd-da2f-aaa54a1d3ffc", "29a0cfab-485b-f5d5-779a-b59f85e204a8",
                 "2f59173c-4bed-b6c3-2191-dea9b58be9c7", "FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF",
                 "00000000-0000-0000-0000-000000000000", "AbCdEf01-2345-6789-aBcD-eF0123456789",
                 "17f0db964e2d6bfdda2faaa54a1d3ffc", "{17f0db96-4e2d-6bfd-da2f-aaa54a1d3ffc}",
                 "17f0db96-4e2d-6bfd-da2f-aaa54a1d3ffc ", "17f0db96-4e2d-6bfd-da2f-aaa54a1d3ff",
                 "17f0db96-4e2d-6bfd-da2f-aaa54a1d3ffg", "17f0db96_4e2d_6bfd_da2f_aaa54a1d3ffc", "",
                 "17f0db96-4e2d-6bfd-da2f-aaa54a1d3ffc\n", "１7f0db96-4e2d-6bfd-da2f-aaa54a1d3ffc"):
        try:
            out["words"].append({"text": text, "words": list(rwc.guid_text_to_words(text))})
        except Exception as error:
            out["words"].append({"text": text, **err(error)})

    files, inventory, _ = saved_inventory()
    try:
        from equipped_cosmetic import resolve_equipped_cdo_paths, resolve_equipped_cosmetic
        pistol_cosmetic = resolve_equipped_cosmetic(inventory, rwc.base_pistol_weapon_id)
        pistol_paths = resolve_equipped_cdo_paths(inventory, rwc.base_pistol_weapon_id)
        knife_paths = resolve_equipped_cdo_paths(inventory, rmc.melee_weapon_id)
        vandal_paths = resolve_equipped_cdo_paths(inventory, "9c82e19d-4575-0200-1a81-3eacf00cf872")
    finally:
        files.close()

    # the VCTCaps24 selection of test_range_weapon_cosmetics, with catalog-shaped cdo paths
    test_cosmetic = twc.selected_base_pistol()

    def test_path(kind, source, uuid_text):
        package, obj = rwc.primary_asset_object_path(source, kind)
        return CosmeticCdoPath(kind=kind, asset_id=uuid_text, source_asset=source, package_path=package,
                               object_name=obj, packaged=True)

    test_paths = EquippedCdoPaths(
        weapon_id=test_cosmetic.weapon_id,
        skin=test_path("skin", test_cosmetic.skin_asset, test_cosmetic.skin_id),
        level=test_path("level", test_cosmetic.level_asset, test_cosmetic.level_id),
        chroma=test_path("chroma", test_cosmetic.chroma_asset, test_cosmetic.chroma_id))

    def gun_layout(weapon_id, actor=520, archetype=523, package=525, pairs=((535, 537), (539, 541), (543, 545)),
                   archetype_package="/Game/Equippables/Guns/Rifles/AK/AssaultRifle_AK",
                   pointer=(18, 19, 21), uuid=(24, 28, 41)):
        return rwc.CosmeticActorLayout(
            weapon_id=weapon_id, channel_index=20, actor_net_guid=actor, archetype_name="Default__AssaultRifle_AK_C",
            archetype_package=archetype_package, archetype_net_guid=archetype, package_net_guid=package,
            level_net_guid=5, pointer_handles=pointer, uuid_handles=uuid, asset_guid_pairs=pairs)

    vandal_id = "9c82e19d-4575-0200-1a81-3eacf00cf872"
    one = ((547, (0xd90ff390, 0x4d4dee48, 0x8d4cbc9f, 0x7b123692)),)
    two = ((547, (1, 2, 3, 4)), (551, (5, 6, 7, 8)))
    many = tuple((1001 + 2 * i, (i, i + 1, 0xFFFFFFFF, 0)) for i in range(60))

    def replaced(paths, which, **changes):
        return dataclasses.replace(paths, **{which: dataclasses.replace(getattr(paths, which), **changes)})

    opens = [
        ("pistol_saved", pistol_paths, rwc.base_pistol_cosmetic_layout, ()),
        ("pistol_test_selection", test_paths, rwc.base_pistol_cosmetic_layout, ()),
        ("knife_saved", knife_paths, rmc.starting_melee_cosmetic_layout, ()),
        ("vandal_saved_one", vandal_paths, gun_layout(vandal_id), one),
        ("vandal_saved_two", vandal_paths, gun_layout(vandal_id), two),
        ("vandal_saved_none", vandal_paths, gun_layout(vandal_id), ()),
        ("vandal_big_guids", vandal_paths, gun_layout(vandal_id, actor=100000, archetype=100001, package=100003,
                                                      pairs=((100005, 100007), (100009, 100011), (100013, 100015))), two),
        ("vandal_guard", vandal_paths, gun_layout(vandal_id), many),
        ("weapon_mismatch", pistol_paths, gun_layout(vandal_id), ()),
        ("actor_odd", vandal_paths, gun_layout(vandal_id, actor=521), ()),
        ("actor_zero", vandal_paths, gun_layout(vandal_id, actor=0), ()),
        ("static_even", vandal_paths, gun_layout(vandal_id, archetype=524), ()),
        ("static_duplicate", vandal_paths, gun_layout(vandal_id, pairs=((535, 537), (535, 541), (543, 545))), ()),
        ("static_is_actor", vandal_paths, gun_layout(vandal_id, actor=520, package=521), ()),
        ("handle_zero", vandal_paths, gun_layout(vandal_id, uuid=(0, 28, 41)), ()),
        ("archetype_outside", vandal_paths, gun_layout(vandal_id, archetype_package="/Game/Gear/X"), ()),
        ("unmeasured_handles", vandal_paths, gun_layout(vandal_id, pointer=(18, 19, 22)), one),
        ("unmeasured_no_attachments", vandal_paths, gun_layout(vandal_id, pointer=(18, 19, 22)), ()),
        ("swapped_handles", vandal_paths, gun_layout(vandal_id, pointer=(21, 19, 18), uuid=(41, 28, 24)), ()),
        ("unpackaged", replaced(vandal_paths, "level", packaged=False), gun_layout(vandal_id), ()),
        ("wrong_kind", replaced(vandal_paths, "chroma", kind="level"), gun_layout(vandal_id), ()),
        ("package_outside", replaced(vandal_paths, "skin", package_path="/Game/Other/X"), gun_layout(vandal_id), ()),
        ("object_not_default", replaced(vandal_paths, "skin", object_name="X_C"), gun_layout(vandal_id), ()),
        ("asset_id_upper", replaced(vandal_paths, "skin", asset_id=vandal_paths.skin.asset_id.upper()),
         gun_layout(vandal_id), ()),
        ("asset_id_bad", replaced(vandal_paths, "skin", asset_id="nope"), gun_layout(vandal_id), ()),
        ("package_unicode", replaced(vandal_paths, "skin", package_path="/Game/Equippables/Ü"), gun_layout(vandal_id), ()),
    ]
    for name, paths, layout, attachments in opens:
        out["opens"].append({"name": name, "paths": paths_dict(paths), "layout": layout_dict(layout),
                             "attachments": [[g, list(w)] for g, w in attachments],
                             **attempt(rwc.build_catalog_cosmetic_actor_open, paths, layout, attachments)})

    def pistol_case(name, cosmetic, paths):
        out["pistol_initial"].append({"name": name, "cosmetic": cosmetic_dict(cosmetic), "paths": paths_dict(paths),
                                      **attempt(rwc.build_base_pistol_initial_cosmetic_open, cosmetic, paths)})

    pistol_case("current_saved", pistol_cosmetic, pistol_paths)
    pistol_case("test_selection", test_cosmetic, test_paths)
    pistol_case("weapon_mismatch", dataclasses.replace(test_cosmetic, weapon_id=vandal_id), test_paths)
    pistol_case("paths_weapon_mismatch", test_cosmetic, dataclasses.replace(test_paths, weapon_id=vandal_id))
    pistol_case("source_outside", dataclasses.replace(test_cosmetic, chroma_asset="Equippables/Guns/X.uasset"), test_paths)
    pistol_case("unpackaged", test_cosmetic, replaced(test_paths, "level", packaged=False))
    pistol_case("asset_id_differs", dataclasses.replace(test_cosmetic, skin_id=test_cosmetic.level_id), test_paths)
    pistol_case("package_outside", test_cosmetic, replaced(test_paths, "chroma", package_path="/Game/Equippables/X"))
    pistol_case("wrong_kind", test_cosmetic, replaced(test_paths, "skin", kind="chroma"))
    pistol_case("source_differs", test_cosmetic, replaced(test_paths, "skin",
                                                          source_asset=test_cosmetic.level_asset))
    pistol_case("object_not_default", test_cosmetic, replaced(test_paths, "skin", object_name="Plain_C"))
    long_name = "Default__" + "A" * 220 + "_C"
    pistol_case("guard", test_cosmetic, replaced(replaced(replaced(test_paths, "skin", object_name=long_name),
                                                          "level", object_name=long_name),
                                                 "chroma", object_name=long_name))

    out["plain"] = {
        "base_pistol": attempt(rsb.build_dynamic_actor_reference,
                               ("Default__BasePistol_C", "/Game/Equippables/Guns/Sidearms/BasePistol/BasePistol"),
                               94, 95, 97, 5, None),
        "melee": attempt(rsb.build_starting_equippable_open, 1),
        "knife_saved_open": attempt(rmc.build_starting_melee_cosmetic_open, knife_paths),
    }
    return out


# ---------------------------------------------------------------- skins

SKIN_T = "20000000-0000-4000-8000-000000000001"
LEVEL_T = "20000000-0000-4000-8000-000000000002"
CHROMA_T = "20000000-0000-4000-8000-000000000003"
PISTOL_ID = "29a0cfab-485b-f5d5-779a-b59f85e204a8"
KNIFE_ID = "2f59173c-4bed-b6c3-2191-dea9b58be9c7"
VANDAL_ID = "9c82e19d-4575-0200-1a81-3eacf00cf872"


def iid(number):
    # a letter in every id, so the upper-case cases really change the text
    return f"3000000a-0000-4000-8000-{number:012x}"


def make_entry(name, weapon_id, number, root):
    """after test_equipped_cosmetic.make_entry: default level/chroma plus one variant each."""
    skin_id, level, level2, chroma, chroma2 = (iid(number + k) for k in range(5))
    return {
        "name": name, "weapon_id": weapon_id, "skin_id": skin_id, "skin_level_id": level, "chroma_id": chroma,
        "entitlements": [{"type_id": SKIN_T, "item_id": skin_id}, {"type_id": LEVEL_T, "item_id": level},
                         {"type_id": LEVEL_T, "item_id": level2}, {"type_id": CHROMA_T, "item_id": chroma2},
                         {"type_id": CHROMA_T, "item_id": chroma}],
        "source": {"weapon_asset": f"{root}/WeaponPrimaryAsset.uasset",
                   "skin_asset": f"{root}/{name}/{name}_PrimaryAsset.uasset",
                   "level_asset": f"{root}/{name}/Levels/{name}_Lv1_PrimaryAsset.uasset",
                   "chroma_asset": f"{root}/{name}/Chromas/{name}_Standard_PrimaryAsset.uasset",
                   "unlock_assets": {
                       "levels": [f"{root}/{name}/Levels/{name}_Lv1_PrimaryAsset.uasset",
                                  f"{root}/{name}/Levels/{name}_Lv2_PrimaryAsset.uasset"],
                       "chromas": [f"{root}/{name}/Chromas/{name}_Variant_PrimaryAsset.uasset",
                                   f"{root}/{name}/Chromas/{name}_Standard_PrimaryAsset.uasset"]}},
    }


PISTOL_ROOT = "Equippables/Guns/Sidearms/BasePistol"
KNIFE_ROOT = "Equippables/Melee"
AK_ROOT = "Equippables/Guns/Rifles/AK"


def base_catalog():
    return {"schema_version": 1, "skins": [
        make_entry("BasePistol_Test", PISTOL_ID, 0x10, PISTOL_ROOT),
        make_entry("Melee_Test", KNIFE_ID, 0x20, KNIFE_ROOT),
        make_entry("AK_Test", VANDAL_ID, 0x30, AK_ROOT),
        make_entry("AK_Standard", VANDAL_ID, 0x40, AK_ROOT),
        make_entry("BasePistol_Standard", PISTOL_ID, 0x50, PISTOL_ROOT),
        make_entry("AK_Second_Standard", VANDAL_ID, 0x60, AK_ROOT),
    ]}


def gun(weapon_id, skin, level, chroma, upper=False):
    if upper:
        return {"ID": weapon_id, "SkinID": skin, "SkinLevelID": level, "ChromaID": chroma}
    return {"iD": weapon_id, "skinId": skin, "skinLevelId": level, "chromaId": chroma,
            "charmInstanceId": None, "charmId": None, "charmLevelId": None, "attachments": []}


def base_state():
    owned = [iid(0x10), iid(0x20), iid(0x30), iid(0x40), iid(0x50), iid(0x60)]
    return {"schema_version": 1, "player_id": PLAYER, "version": 3, "owned": owned,
            "equipped": {PISTOL_ID: iid(0x10), KNIFE_ID: iid(0x20), VANDAL_ID: iid(0x30)},
            "loadout": {"subject": PLAYER, "version": 3, "guns": [
                gun(PISTOL_ID, iid(0x10), iid(0x12), iid(0x13)),
                gun(KNIFE_ID, iid(0x20), iid(0x21), iid(0x24)),
                gun(VANDAL_ID, iid(0x30), iid(0x31), iid(0x34))], "sprays": []}}


def skin_cases():
    """(name, catalog, state) edge cases; each is a mutation of the synthetic base."""
    cases = []

    def case(name, mutate_catalog=None, mutate_state=None, state=True):
        catalog = base_catalog()
        st = base_state() if state else None
        if mutate_catalog:
            mutate_catalog(catalog)
        if mutate_state and st is not None:
            mutate_state(st)  # in place; the lambdas' return values are discarded
        cases.append((name, catalog, st))

    def skins(c):
        return c["skins"]

    def src(c, index):
        return c["skins"][index]["source"]

    def guns(s):
        return s["loadout"]["guns"]

    case("baseline")
    case("no_state", state=False)
    case("upper_loadout_keys", mutate_state=lambda s: s["loadout"].update(
        {"guns": [gun(PISTOL_ID.upper(), iid(0x10).upper(), iid(0x12).upper(), iid(0x13).upper(), upper=True)]}))
    case("guns_upper_list", mutate_state=lambda s: s.update({"loadout": {"Subject": PLAYER, "Version": 3,
                                                                          "Guns": guns(s)}}))
    case("both_key_spellings_agree", mutate_state=lambda s: guns(s)[0].update({"SkinID": iid(0x10).upper()}))
    case("both_key_spellings_conflict", mutate_state=lambda s: guns(s)[0].update({"SkinLevelID": iid(0x11)}))
    case("loadout_missing", mutate_state=lambda s: s.pop("loadout"))
    case("loadout_null", mutate_state=lambda s: s.update({"loadout": None}))
    case("weapon_not_in_loadout", mutate_state=lambda s: guns(s).pop(0))
    case("weapon_twice_in_loadout", mutate_state=lambda s: guns(s).append(copy.deepcopy(guns(s)[0])))
    case("saved_skin_differs", mutate_state=lambda s: guns(s)[0].update({"skinId": iid(0x50)}))
    case("level_not_granted", mutate_state=lambda s: guns(s)[0].update({"skinLevelId": iid(0x14)}))
    case("chroma_is_level", mutate_state=lambda s: guns(s)[0].update({"chromaId": iid(0x11)}))
    case("level_field_missing", mutate_state=lambda s: guns(s)[0].pop("skinLevelId"))
    case("level_field_null", mutate_state=lambda s: guns(s)[0].update({"skinLevelId": None}))
    case("level_field_bad", mutate_state=lambda s: guns(s)[0].update({"skinLevelId": "nope"}))
    case("skin_field_missing", mutate_state=lambda s: guns(s)[0].pop("skinId"))
    case("gun_id_upper_key_wins_lower", mutate_state=lambda s: guns(s)[0].update({"ID": VANDAL_ID}))
    case("gun_id_not_str", mutate_state=lambda s: guns(s).append({"iD": 5, "skinId": iid(0x10)}))
    case("unlock_levels_short", lambda c: src(c, 0)["unlock_assets"]["levels"].pop())
    case("unlock_levels_missing", lambda c: src(c, 0)["unlock_assets"].pop("levels"))
    case("unlock_missing", lambda c: src(c, 0).pop("unlock_assets"))
    case("source_missing", lambda c: skins(c)[0].pop("source"))
    case("source_not_dict", lambda c: skins(c)[0].update({"source": []}))
    case("default_level_asset_mismatch", lambda c: src(c, 0).update(
        {"level_asset": src(c, 0)["unlock_assets"]["levels"][1]}))
    case("level_asset_backslash", lambda c: src(c, 0)["unlock_assets"]["levels"].__setitem__(
        1, "Equippables\\Guns\\X_PrimaryAsset.uasset"))
    case("level_asset_dotdot", lambda c: src(c, 0)["unlock_assets"]["levels"].__setitem__(
        1, "Equippables/Guns/../X_PrimaryAsset.uasset"))
    case("level_asset_absolute", lambda c: src(c, 0)["unlock_assets"]["levels"].__setitem__(
        1, "/Equippables/Guns/X_PrimaryAsset.uasset"))
    case("level_asset_suffix", lambda c: src(c, 0)["unlock_assets"]["levels"].__setitem__(
        1, "Equippables/Guns/X_PrimaryAsset.uexp"))
    case("level_asset_only_suffix", lambda c: src(c, 0)["unlock_assets"]["levels"].__setitem__(
        1, "Equippables/.uasset"))
    case("level_asset_double_slash", lambda c: src(c, 0)["unlock_assets"]["levels"].__setitem__(
        1, "Equippables//Guns/X_PrimaryAsset.uasset"))
    case("level_asset_dot_component", lambda c: src(c, 0)["unlock_assets"]["levels"].__setitem__(
        1, "Equippables/./Guns/X_PrimaryAsset.uasset"))
    case("level_asset_trailing_slash", lambda c: src(c, 0)["unlock_assets"]["levels"].__setitem__(
        1, "Equippables/Guns/X_PrimaryAsset.uasset/"))
    case("level_asset_space", lambda c: src(c, 0)["unlock_assets"]["levels"].__setitem__(
        1, "Equippables/Guns/X PrimaryAsset.uasset"))
    case("level_asset_not_str", lambda c: src(c, 0)["unlock_assets"]["levels"].__setitem__(1, 7))
    case("level_asset_lowercase_root", lambda c: src(c, 0)["unlock_assets"]["levels"].__setitem__(
        1, "equippables/Guns/X_PrimaryAsset.uasset"))
    case("skin_asset_invalid", lambda c: src(c, 0).update({"skin_asset": "Equippables/Guns/X.uexp"}))
    case("weapon_asset_missing", lambda c: src(c, 0).pop("weapon_asset"))
    case("skin_asset_bad_component", lambda c: src(c, 0).update({"skin_asset": "Equippables/Guns/X.Y.uasset"}))
    case("level_unpackaged_default", lambda c: src(c, 0).update({"level_asset_packaged": False}),
         lambda s: guns(s)[0].update({"skinLevelId": iid(0x11)}))
    case("level_unpackaged_variant_selected", lambda c: src(c, 0).update({"level_asset_packaged": False}))
    case("level_unpackaged_zero", lambda c: src(c, 0).update({"level_asset_packaged": 0}),
         lambda s: guns(s)[0].update({"skinLevelId": iid(0x11)}))
    case("knife_unpackaged", lambda c: src(c, 1).update({"level_asset_packaged": False}),
         lambda s: guns(s)[1].update({"skinLevelId": iid(0x21)}))
    case("standard_unpackaged", lambda c: src(c, 3).update({"level_asset_packaged": False}))
    case("no_standard", lambda c: [e.update({"name": e["name"] + "X"}) for e in skins(c)])
    case("unequipped", mutate_state=lambda s: s["equipped"].clear())
    case("equipped_upper_key", mutate_state=lambda s: s.update(
        {"equipped": {PISTOL_ID.upper(): iid(0x10), KNIFE_ID: iid(0x20)}}))
    case("equipped_upper_value", mutate_state=lambda s: s["equipped"].update({PISTOL_ID: iid(0x10).upper()}))
    case("owned_upper", mutate_state=lambda s: s.update({"owned": [o.upper() for o in s["owned"]]}))
    case("equipped_not_owned", mutate_state=lambda s: s["owned"].remove(iid(0x10)))
    case("equipped_wrong_weapon", mutate_state=lambda s: s["equipped"].update({PISTOL_ID: iid(0x30)}))
    case("equipped_bad_weapon_id", mutate_state=lambda s: s["equipped"].update({"pistol": iid(0x10)}))
    case("owned_unknown", mutate_state=lambda s: s["owned"].append(iid(0x99)))
    case("owned_not_list", mutate_state=lambda s: s.update({"owned": {}}))
    case("owned_not_str", mutate_state=lambda s: s["owned"].append(5))
    case("equipped_not_dict", mutate_state=lambda s: s.update({"equipped": []}))
    case("equipped_value_not_str", mutate_state=lambda s: s["equipped"].update({PISTOL_ID: None}))
    case("schema_2", mutate_state=lambda s: s.update({"schema_version": 2}))
    case("schema_true", mutate_state=lambda s: s.update({"schema_version": True}))
    case("schema_float", mutate_state=lambda s: s.update({"schema_version": 1.0}))
    case("schema_string", mutate_state=lambda s: s.update({"schema_version": "1"}))
    case("player_differs", mutate_state=lambda s: s.update({"player_id": iid(1)}))
    case("version_string", mutate_state=lambda s: s.update({"version": " 46 "}))
    case("version_unicode_digits", mutate_state=lambda s: s.update({"version": "　٤٦"}))
    case("version_float", mutate_state=lambda s: s.update({"version": 46.9}))
    case("version_null", mutate_state=lambda s: s.update({"version": None}))
    case("version_zero", mutate_state=lambda s: s.update({"version": 0}))
    case("version_missing", mutate_state=lambda s: s.pop("version"))
    case("loadout_not_dict", mutate_state=lambda s: s.update({"loadout": []}))
    case("loadout_guns_not_list", mutate_state=lambda s: s["loadout"].update({"guns": {}}))
    case("loadout_gun_not_dict", mutate_state=lambda s: guns(s).append("x"))
    case("loadout_129_guns", mutate_state=lambda s: guns(s).extend([{"iD": iid(0x900 + k)} for k in range(126)]))
    case("loadout_128_guns", mutate_state=lambda s: guns(s).extend([{"iD": iid(0x900 + k)} for k in range(125)]))
    case("catalog_schema_2", lambda c: c.update({"schema_version": 2}))
    case("catalog_skins_not_list", lambda c: c.update({"skins": {}}))
    case("catalog_entry_not_dict", lambda c: skins(c).append([]))
    case("catalog_bad_id", lambda c: skins(c)[0].update({"chroma_id": "x"}))
    case("catalog_upper_ids", lambda c: skins(c)[0].update({"skin_id": iid(0x10).upper(),
                                                            "weapon_id": PISTOL_ID.upper()}))
    case("catalog_empty_name", lambda c: skins(c)[0].update({"name": ""}))
    case("catalog_entitlement_bad", lambda c: skins(c)[0]["entitlements"].append({"type_id": "x", "item_id": iid(1)}))
    case("catalog_entitlements_missing", lambda c: skins(c)[0].pop("entitlements"))
    case("catalog_default_two_types", lambda c: skins(c)[0]["entitlements"].append(
        {"type_id": CHROMA_T, "item_id": iid(0x11)}))
    case("catalog_types_overlap", lambda c: skins(c)[0]["entitlements"].__setitem__(
        0, {"type_id": LEVEL_T, "item_id": iid(0x10)}))
    case("catalog_duplicate_skin", lambda c: skins(c).append(copy.deepcopy(skins(c)[0])))
    case("legacy_migration", lambda c: skins(c)[0].update({
        "legacy_skin_ids": [iid(0x70)], "legacy_level_ids": {iid(0x71): iid(0x12)},
        "legacy_chroma_ids": {iid(0x72): iid(0x14)}}),
        lambda s: (s["owned"].__setitem__(0, iid(0x70)), s["equipped"].update({PISTOL_ID: iid(0x70)}),
                   guns(s)[0].update({"skinId": iid(0x70), "skinLevelId": iid(0x71), "chromaId": iid(0x72)})))
    case("legacy_level_only", lambda c: skins(c)[0].update({"legacy_level_ids": {iid(0x71): iid(0x12)}}),
         lambda s: guns(s)[0].update({"skinLevelId": iid(0x71).upper()}))
    case("legacy_apply_resets_ungranted", lambda c: skins(c)[0].update({"legacy_skin_ids": [iid(0x70)]}),
         lambda s: (s["owned"].__setitem__(0, iid(0x70)), s["equipped"].update({PISTOL_ID: iid(0x70)}),
                    guns(s)[0].update({"skinId": iid(0x70), "skinLevelId": iid(0x99)})))
    case("legacy_apply_upper_keys", lambda c: skins(c)[0].update({"legacy_skin_ids": [iid(0x70)]}),
         lambda s: (s["owned"].__setitem__(0, iid(0x70)), s["equipped"].update({PISTOL_ID: iid(0x70)}),
                    s["loadout"].update({"Version": 3, "Guns": [gun(PISTOL_ID, iid(0x70), iid(0x12), iid(0x13),
                                                                    upper=True)]}),
                    s["loadout"].pop("guns"), s["loadout"].pop("version")))
    case("legacy_both_spellings", lambda c: skins(c)[0].update({"legacy_skin_ids": [iid(0x70)]}),
         lambda s: (s["owned"].__setitem__(0, iid(0x70)), s["equipped"].update({PISTOL_ID: iid(0x70)}),
                    guns(s)[0].update({"skinId": iid(0x70), "SkinID": iid(0x70)})))
    case("legacy_no_loadout", lambda c: skins(c)[0].update({"legacy_skin_ids": [iid(0x70)]}),
         lambda s: (s["owned"].__setitem__(0, iid(0x70)), s.update({"loadout": None})))
    case("legacy_skin_is_own_id", lambda c: skins(c)[0].update({"legacy_skin_ids": [iid(0x10)]}))
    case("legacy_skin_duplicate", lambda c: (skins(c)[0].update({"legacy_skin_ids": [iid(0x70)]}),
                                              skins(c)[1].update({"legacy_skin_ids": [iid(0x70).upper()]})))
    case("legacy_skin_collides", lambda c: skins(c)[0].update({"legacy_skin_ids": [iid(0x20)]}))
    case("legacy_skin_not_list", lambda c: skins(c)[0].update({"legacy_skin_ids": iid(0x70)}))
    case("legacy_level_not_dict", lambda c: skins(c)[0].update({"legacy_level_ids": [iid(0x71)]}))
    case("legacy_level_to_ungranted", lambda c: skins(c)[0].update({"legacy_level_ids": {iid(0x71): iid(0x13)}}))
    case("legacy_level_self", lambda c: skins(c)[0].update({"legacy_level_ids": {iid(0x12): iid(0x12)}}))
    case("legacy_level_collides", lambda c: skins(c)[0].update({"legacy_level_ids": {iid(0x22): iid(0x12)}}))
    case("legacy_level_conflict", lambda c: (skins(c)[0].update({"legacy_level_ids": {iid(0x71): iid(0x12)}}),
                                              skins(c)[1].update({"legacy_level_ids": {iid(0x71): iid(0x22)}})))
    case("legacy_level_same_twice", lambda c: (skins(c)[0].update({"legacy_level_ids": {iid(0x71): iid(0x12)}}),
                                                skins(c)[4].update({"legacy_level_ids": {iid(0x71): iid(0x12)}})))
    case("legacy_chroma_upper", lambda c: skins(c)[0].update({"legacy_chroma_ids": {iid(0x72).upper(): iid(0x14).upper()}}),
         lambda s: guns(s)[0].update({"chromaId": iid(0x72)}))
    case("legacy_level_bad_uuid", lambda c: skins(c)[0].update({"legacy_level_ids": {"x": iid(0x12)}}))
    return cases


def query_inventory(inventory, files, weapon_ids):
    import range_gun_loader as rgl
    import range_melee_cosmetics as rmc
    import range_udp_handshake as ruh
    import range_weapon_cosmetics as rwc
    import range_server_bunch as rsb
    from equipped_cosmetic import resolve_equipped_cdo_paths, resolve_equipped_cosmetic

    queries = []
    for weapon_id in weapon_ids:
        q = {"weapon_id": weapon_id,
             "cosmetic": value(lambda: cosmetic_dict(resolve_equipped_cosmetic(inventory, weapon_id))),
             "cdo_paths": value(lambda: paths_dict(resolve_equipped_cdo_paths(inventory, weapon_id)))}
        q["saved"] = with_inventory(inventory, lambda: value(
            lambda: paths_dict(rgl.select_saved_cdo_paths(PLAYER, weapon_id))))
        q["standard"] = with_inventory(inventory, lambda: value(
            lambda: paths_dict(rgl.standard_cdo_paths(PLAYER, weapon_id))))
        queries.append(q)

    # A21 (ruh:2989-3007): a fresh inventory from the files, then cosmetic / unshipped / unselected
    catalog, state = files.paths()
    try:
        selection = ruh.select_base_pistol_cosmetic_loadout(catalog, state, PLAYER)
        applied = selection is not None and all(p.packaged for p in (selection[1].skin, selection[1].level,
                                                                     selection[1].chroma))
        if applied:
            body = attempt(rwc.build_base_pistol_initial_cosmetic_open, *selection)
            kind = "cosmetic"
        else:
            body = attempt(rsb.build_dynamic_actor_reference,
                           ("Default__BasePistol_C", "/Game/Equippables/Guns/Sidearms/BasePistol/BasePistol"),
                           94, 95, 97, 5, None)
            kind = "unshipped" if selection is not None else "unselected"
        pistol = {"kind": kind, **body}
    except Exception as error:
        pistol = err(error)

    # B18 i=1 (ruh:4024-4044): saved knife or the plain open; any error falls back
    melee_body = rsb.build_starting_equippable_open(1)
    try:
        paths = with_inventory(inventory, lambda: rmc.select_saved_melee_cdo_paths(PLAYER))
        if paths is not None:
            melee_body = rmc.build_starting_melee_cosmetic_open(paths)
            melee_kind = "cosmetic"
        else:
            melee_kind = "unselected"
    except Exception:
        melee_kind = "error"
    return queries, pistol, {"kind": melee_kind, **hb(melee_body)}


def inventory_dict(inventory):
    return {"owned": sorted(inventory.owned), "equipped": sorted([k, v] for k, v in inventory.equipped.items()),
            "loadout": inventory.loadout, "version": inventory.version}


def compact(data):
    return json.dumps(data, separators=(",", ":"))


def json_diff(base, new, path, ops):
    """patch ops turning base into new: ["del", path], ["set", path, v] (in place, or appended
    for a new key), ["push", path, v]. dict order follows python insertion order."""
    if type(base) is type(new) and isinstance(base, dict):
        for key in base:
            if key not in new:
                ops.append(["del", path + [key]])
        for key in base:
            if key in new:
                json_diff(base[key], new[key], path + [key], ops)
        for key in new:
            if key not in base:
                ops.append(["set", path + [key], new[key]])
    elif type(base) is type(new) and isinstance(base, list) and len(new) >= len(base):
        for index in range(len(base)):
            json_diff(base[index], new[index], path + [index], ops)
        for item in new[len(base):]:
            ops.append(["push", path, item])
    elif type(base) is not type(new) or base != new:
        ops.append(["set", path, new])
    return ops


def json_patch(doc, ops):
    doc = copy.deepcopy(doc)
    for op in ops:
        kind, path = op[0], op[1]
        if kind == "set" and not path:
            doc = copy.deepcopy(op[2])
            continue
        parent = doc
        for key in (path if kind == "push" else path[:-1]):
            parent = parent[key]
        if kind == "push":
            parent.append(copy.deepcopy(op[2]))
        elif kind == "del":
            del parent[path[-1]]
        else:
            parent[path[-1]] = copy.deepcopy(op[2])
    return doc


def patch_for(base, new):
    ops = json_diff(base, new, [], [])
    if compact(json_patch(base, ops)) != compact(new):
        # a key order the diff cannot express (keys replaced): send the whole document
        ops = [["set", [], new]]
    if compact(json_patch(base, ops)) != compact(new):
        raise AssertionError("patch does not rebuild the mutated document")
    return ops


def section_skins():
    import range_store as rst

    catalog_text, state_text = real_skin_files()
    catalog = json.loads(catalog_text)
    state = json.loads(state_text)
    bodies = []

    def body_ref(result):
        """open bodies repeat across cases: keep each distinct hex once and point at it."""
        if "hex" not in result:
            return result
        if result["hex"] not in bodies:
            bodies.append(result["hex"])
        return {"kind": result["kind"], "body": bodies.index(result["hex"]), "bit_count": result["bit_count"]}

    out = {"player": PLAYER, "state_text": state_text,
           "catalog": {"bytes": len((backend_dir() / "skin_catalog.json").read_bytes()), "skins": len(catalog["skins"])},
           "weapons": [], "cases": [], "bodies": bodies}

    # every weapon in the catalog (first-appearance order), then ids that are not weapons
    weapon_ids = list(dict.fromkeys(e["weapon_id"] for e in catalog["skins"]))
    weapon_ids += [w.upper() for w in weapon_ids[:2]] + [rst.store_items["HeavyArmorItem_C"].weapon_id, "nope",
                                                         "29A0CFAB485BF5D5779AB59F85E204A8"]

    full = SkinFiles(catalog_text, state_text)
    try:
        inventory, rewritten = full.inventory()
        if isinstance(inventory, Exception):
            raise inventory
        out["state_rewritten"] = rewritten
        out["inventory"] = compact(inventory_dict(inventory))
        queries, pistol, melee = query_inventory(inventory, full, weapon_ids)
        out["weapons"] = [compact(q) for q in queries]
        out["base_pistol_open"] = body_ref(pistol)
        out["melee_open"] = body_ref(melee)
    finally:
        full.close()

    # a catalog subset that resolves identically (checked below): every equipped skin, every
    # *_Standard entry and every entry with legacy ids or level_asset_packaged, in file order
    equipped = set(state["equipped"].values())
    keep = [e for e in catalog["skins"]
            if e["skin_id"] in equipped or e["name"].endswith("_Standard") or "legacy_skin_ids" in e
            or "legacy_level_ids" in e or "legacy_chroma_ids" in e or "level_asset_packaged" in e["source"]]
    subset_catalog = compact({"schema_version": 1, "skins": keep})
    kept = {e["skin_id"] for e in keep}
    subset_state = compact(dict(state, owned=[o for o in state["owned"] if o in kept]))
    sub = SkinFiles(subset_catalog, subset_state)
    try:
        inventory, rewritten = sub.inventory()
        if isinstance(inventory, Exception):
            raise inventory
        queries, pistol, melee = query_inventory(inventory, sub, weapon_ids)
        if ([compact(q) for q in queries] != out["weapons"] or body_ref(pistol) != out["base_pistol_open"]
                or body_ref(melee) != out["melee_open"]):
            raise AssertionError("catalog subset does not resolve like the full catalog")
        out["subset"] = {"catalog_text": subset_catalog, "state_text": subset_state, "state_rewritten": rewritten,
                         "inventory": compact(inventory_dict(inventory))}
    finally:
        sub.close()

    # synthetic edge cases: each is the base catalog / state plus a patch (json_diff ops); python
    # reads the patched documents as text, rust patches its parsed base the same way
    out["base_catalog_text"] = compact(base_catalog())
    out["base_state_text"] = compact(base_state())
    query_table = []
    out["queries"] = query_table

    def query_ref(query):
        text = compact(query)
        if text not in query_table:
            query_table.append(text)
        return query_table.index(text)

    for name, cat, st in skin_cases():
        files = SkinFiles(compact(cat), None if st is None else compact(st))
        try:
            inventory, rewritten = files.inventory()
            row = {"name": name, "catalog_patch": patch_for(base_catalog(), cat)}
            if st is None:
                row["state_absent"] = True
            else:
                row["state_patch"] = patch_for(base_state(), st)
            if isinstance(inventory, Exception):
                row["load"] = err(inventory)
            else:
                row["load"] = compact({"rewritten": rewritten, **inventory_dict(inventory)})
                weapons = [PISTOL_ID, KNIFE_ID, VANDAL_ID]
                if name == "baseline":
                    weapons += [PISTOL_ID.upper(), "nope", KNIFE_ID.upper()]
                queries, pistol, melee = query_inventory(inventory, files, weapons)
                row.update({"queries": [query_ref(q) for q in queries], "base_pistol_open": body_ref(pistol),
                            "melee_open": body_ref(melee)})
            out["cases"].append(row)
        finally:
            files.close()

    # catalog_cdo_path on the synthetic catalog: every kind against every id of the entry
    from equipped_cosmetic import catalog_cdo_path
    cat = base_catalog()
    cat["skins"][0]["source"]["level_asset_packaged"] = False
    out["cdo_path_catalog"] = compact(cat)
    rows = []
    for e in cat["skins"][:2]:
        ids = [x["item_id"] for x in e["entitlements"]] + [iid(0x99), "nope", e["skin_id"].upper()]
        for kind in ("skin", "level", "chroma", "other"):
            for asset_id in ids:
                rows.append(compact({"skin_id": e["skin_id"], "kind": kind, "asset_id": asset_id,
                                     **value(lambda: cdo_dict(catalog_cdo_path(e, kind, asset_id)))}))
    out["cdo_paths"] = rows
    return out


# ---------------------------------------------------------------- whole catalog

def component_ids(entry, default_key):
    """ids sharing the default's entitlement type, in entitlement order (equipped_cosmetic.py:84-87)."""
    default_id = entry[default_key].lower()
    type_id = next(x["type_id"].lower() for x in entry["entitlements"] if x["item_id"].lower() == default_id)
    return [x["item_id"].lower() for x in entry["entitlements"] if x["type_id"].lower() == type_id]


def section_catalog_sweep():
    """every skin a player can equip, as production would see it: catalog_cdo_path for every
    skin / level / chroma id of every catalog entry, and the open of every level x chroma
    combination on its weapon (bought gun on a fresh allocator, the knife's b18 open, the
    classic's a21 open). one text line per result, digested per weapon with zlib.crc32 over
    the utf-8 lines joined by newlines, so the fixture stays small."""
    import zlib

    import range_gun_loader as rgl
    import range_melee_cosmetics as rmc
    import range_store as rst
    import range_weapon_cosmetics as rwc
    from equipped_cosmetic import EquippedCdoPaths, EquippedCosmetic, catalog_cdo_path

    catalog_text, _ = real_skin_files()
    catalog = json.loads(catalog_text)
    items = {item.weapon_id: item for item in rst._items}
    weapons = {}

    def lines_for(weapon_id):
        return weapons.setdefault(weapon_id, {"paths": [], "gun": [], "knife": [], "a21": []})

    def body_line(prefix, fn, *args):
        try:
            body, bits = fn(*args)
            return f"{prefix}|{bits}|{body.hex()}", bits
        except Exception as error:
            return f"{prefix}|err|{error}", None

    for entry in catalog["skins"]:
        weapon_id = entry["weapon_id"].lower()
        out = lines_for(weapon_id)
        resolved = {}
        for kind, ids in (("skin", [entry["skin_id"]]), ("level", component_ids(entry, "skin_level_id")),
                          ("chroma", component_ids(entry, "chroma_id"))):
            for asset_id in ids:
                try:
                    path = catalog_cdo_path(entry, kind, asset_id)
                    resolved[(kind, asset_id)] = path
                    out["paths"].append(f"{entry['skin_id']}|{kind}|{asset_id}|{int(path.packaged)}|{path.source_asset}"
                                        f"|{path.package_path}|{path.object_name}")
                except Exception as error:
                    out["paths"].append(f"{entry['skin_id']}|{kind}|{asset_id}|err|{error}")
        skin = resolved.get(("skin", entry["skin_id"]))
        for level_id in component_ids(entry, "skin_level_id"):
            for chroma_id in component_ids(entry, "chroma_id"):
                level, chroma = resolved.get(("level", level_id)), resolved.get(("chroma", chroma_id))
                if skin is None or level is None or chroma is None:
                    continue
                paths = EquippedCdoPaths(weapon_id=weapon_id, skin=skin, level=level, chroma=chroma)
                prefix = f"{entry['skin_id']}|{level_id}|{chroma_id}"
                item = items.get(weapon_id)
                if item is not None and item.is_gun:
                    load = rgl.plan_gun_load(CLASS_PATHS[item.class_name], item, rgl.PurchaseAllocator())
                    out["gun"].append(body_line(prefix, rgl.gun_open_body, load, paths))
                if weapon_id == rmc.melee_weapon_id:
                    out["knife"].append(body_line(prefix, rmc.build_starting_melee_cosmetic_open, paths))
                if weapon_id == rwc.base_pistol_weapon_id:
                    cosmetic = EquippedCosmetic(
                        weapon_id=weapon_id, skin_id=skin.asset_id, level_id=level.asset_id,
                        chroma_id=chroma.asset_id, name=entry["name"], weapon_asset=entry["source"]["weapon_asset"],
                        skin_asset=skin.source_asset, level_asset=level.source_asset,
                        chroma_asset=chroma.source_asset)
                    out["a21"].append(body_line(prefix, rwc.build_base_pistol_initial_cosmetic_open, cosmetic, paths))

    def digest(lines):
        return zlib.crc32("\n".join(lines).encode("utf-8"))

    result = {"catalog_bytes": len((backend_dir() / "skin_catalog.json").read_bytes()),
              "class_paths": [[item.weapon_id, CLASS_PATHS[item.class_name]] for item in rst._items
                              if item.is_gun],
              "weapons": []}
    for weapon_id, out in weapons.items():
        row = {"weapon_id": weapon_id, "paths": [len(out["paths"]), digest(out["paths"])]}
        for kind in ("gun", "knife", "a21"):
            if out[kind]:
                sizes = [bits for _, bits in out[kind] if bits is not None]
                row[kind] = [len(out[kind]), digest([line for line, _ in out[kind]]),
                             max(sizes) if sizes else None, len(out[kind]) - len(sizes)]
        result["weapons"].append(row)
    return result


SECTIONS = {
    "guns_catalog_sweep": section_catalog_sweep,
    "guns_store": section_store,
    "guns_alloc": section_alloc,
    "guns_equippables": section_equippables,
    "guns_mirror": section_mirror,
    "guns_loader": section_loader,
    "guns_attachments": section_attachments,
    "guns_cosmetics": section_cosmetics,
    "guns_skins": section_skins,
}
