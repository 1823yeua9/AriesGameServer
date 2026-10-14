"""rep builders: actor opens, rep-layout payloads, rpc payloads, complete bodies
(spec 02 sections 3 and 6.1-6.4). every case calls the python builder offline;
a builder that raises records {"error": <exception name>} and rust must fail too.
"""

import math

GUARD_ERRORS = (ValueError, IndexError, KeyError, UnicodeEncodeError)


def run(fn, *args, **kwargs):
    try:
        body, bits = fn(*args, **kwargs)
    except GUARD_ERRORS as error:
        return {"error": type(error).__name__}
    return {"hex": body.hex(), "bit_count": bits}


def number(value):
    """json has no nan/inf: send those as strings."""
    if isinstance(value, float) and not math.isfinite(value):
        return "nan" if math.isnan(value) else ("inf" if value > 0 else "-inf")
    return value


def vector(values):
    return [number(v) for v in values]


WORLD = ("AresWorldSettings", "PersistentLevel", "Range", "/Game/Maps/Poveglia/Range")
WUSHU_GUID = (0xADD6443A, 0x41BDE414, 0xF6ADE58D, 0x267F4E95)
SPAWN = (3581.634, -12019.322, 1294.153)


def section_rep_builders():
    import range_server_bunch as rsb
    import range_udp_handshake as hs
    from range_move_correction import build_correction_container, build_correction_rpc_payload
    from range_server_bunch import BitWriter

    out = {}

    # ---- section 3: actor opens and exports ----
    cases = []
    # fstring guard edges: 240 ascii chars is the longest, nul and non-ascii raise (rsb:67-72)
    for chain, actor in ((WORLD, 3), (WORLD, 201), (WORLD[:2], 3), (WORLD + ("A", "B", "C", "D"), 5),
                         (WORLD[:1], 3), (WORLD + ("A", "B", "C", "D", "E"), 3), (WORLD, 4), (WORLD, 1),
                         (WORLD, 0xFFFFFFFD), (WORLD, 0xFFFFFFF9), (("", "x"), 3),
                         (("A" * 240, "/Game/B"), 3), (("A" * 241, "/Game/B"), 3), (("A\x00B", "/Game/B"), 3),
                         (("Café", "/Game/B"), 3), (("x", "y"), 0x7F), (("x", "y"), 0x81)):
        cases.append({"chain": list(chain), "actor": actor, **run(rsb.build_exported_static_actor_reference, chain, actor)})
    out["static_reference"] = cases

    named = (
        ("world_settings", lambda: rsb.build_exported_static_actor_reference(WORLD)),
        ("game_state", rsb.build_dynamic_range_game_state_reference),
        ("controller", rsb.build_dynamic_range_controller_reference),
        ("player_state", rsb.build_dynamic_range_player_state_reference),
        ("player_info", rsb.build_dynamic_player_info_reference),
        ("pawn_spawn", lambda: rsb.build_dynamic_wushu_pawn_reference(SPAWN)),
        ("pawn_none", lambda: rsb.build_dynamic_wushu_pawn_reference(None)),
        ("pawn_origin", lambda: rsb.build_dynamic_wushu_pawn_reference((0.0, -0.0, 0.04))),
        # hs:3002-3007, the plain open
        ("base_pistol", lambda: rsb.build_dynamic_actor_reference(
            archetype_chain=("Default__BasePistol_C", "/Game/Equippables/Guns/Sidearms/BasePistol/BasePistol"),
            actor_net_guid=94, archetype_net_guid=95, package_net_guid=97, level_net_guid=5, spawn_location=None)),
    )
    out["named_reference"] = [{"name": name, **run(fn)} for name, fn in named]

    # the pawn open's spawn vector at its int32 / rounding / finiteness edges (rsb:78-96)
    locations = (SPAWN, (0.0, 0.0, 0.0), (-0.0, -0.0, -0.0), (0.05, -0.05, 0.15), (-0.25, 0.25, 0.35),
                 (214748364.7, -214748364.8, 0.0), (214748364.8, 0.0, 0.0), (0.0, -214748364.86, 0.0),
                 (1e300, 0.0, 0.0), (float("nan"), 0.0, 0.0), (0.0, float("inf"), 0.0), (0.0, 0.0, float("-inf")))
    out["pawn_location"] = [{"location": vector(loc), **run(rsb.build_dynamic_wushu_pawn_reference, loc)}
                            for loc in locations]

    out["base_team_state"] = [{"actor": a, **run(rsb.build_dynamic_base_team_state_reference, a)}
                              for a in (100, 102, 104, 98, 0)]

    cases = []
    for args in (("AresHUD_C", "/Game/UI/InGame/HUD/AresHUD", 109, 111),
                 ("ShootingRangeGameMode_C", "/Game/GameModes/ShootingRange/ShootingRangeGameMode", 261, 263),
                 ("ShootingRangeHUDConfig_C", "/Game/UI/InGame/HUD/GameStateHUDConfigs/ShootingRangeHUDConfig", 265, 267),
                 ("X_C", "/Game/X", 3, 5), ("X_C", "/Game/X", 0xFFFFFFFF, 0xFFFFFFFD),
                 ("", "/Game/X", 109, 111), ("X_C", "/Script/ShooterGame", 109, 111), ("X_C", "/Game/X", 110, 111),
                 ("X_C", "/Game/X", 109, 112), ("X_C", "/Game/X", 109, 109), ("X_C", "/Game/X", 1, 111),
                 ("C" * 240, "/Game/X", 109, 111), ("C" * 241, "/Game/X", 109, 111), ("X_C", "/Game/" + "P" * 234, 109, 111),
                 ("X_C", "/Game/" + "P" * 235, 109, 111), ("X_C", "/Game/", 109, 111), ("X_é", "/Game/X", 109, 111)):
        cases.append({"class_name": args[0], "package": args[1], "class_guid": args[2], "package_guid": args[3],
                      **run(rsb.build_class_netguid_export, *args)})
    out["class_export"] = cases

    out["starting_open"] = [{"index": i, **run(rsb.build_starting_equippable_open, i)} for i in range(8)]
    cases = []
    for i in range(8):
        try:
            cases.append({"index": i, "guids": list(rsb.starting_equippable_guids(i))})
        except ValueError:
            cases.append({"index": i, "error": "ValueError"})
    out["starting_guids"] = cases
    out["starting_table"] = [{"archetype": a, "package": p, "slot": s} for a, p, s in rsb.wushu_starting_equippables]
    names = ["BasePistol_C", "Equippable_Unarmed_C", "Ability_Melee_Base_C", "Ability_Wushu_4_Smoke_C",
             "Ability_Wushu_E_Dash_C", "Ability_Wushu_Q_CycloneBoost_C", "Ability_Wushu_X_Dagger_Production_C",
             "Ability_Wushu_Passive_Glide_C", "Default__BasePistol_C", "AssaultRifle_AK_C", ""]
    out["guid_for_class"] = [{"class_name": n, "guid": rsb.equippable_guid_for_class(n)} for n in names]

    # ---- section 6.1: rep-layout payloads ----
    cases = []
    for guid, base in ((WUSHU_GUID, 201), (WUSHU_GUID, 1), (WUSHU_GUID, 125), (WUSHU_GUID, 0xFFFC), ((0, 0, 0, 0), 201),
                       ((0xFFFFFFFF,) * 4, 201), (WUSHU_GUID, 0), (WUSHU_GUID, 0xFFFD)):
        cases.append({"guid": list(guid), "base": base, **run(rsb.build_desired_class_payload, *guid, base_handle=base)})
    out["desired_class"] = cases

    cases = []
    for handle, guids in ((34, (100, 102)), (17, (6,)), (17, ()), (300, (1, 127, 128, 0xFFFF)),
                          (1, tuple(range(1, 257))), (0xFFFF, (5,)), (1, tuple(range(1, 258))),
                          (0, (6,)), (0x10000, (6,)), (17, (0,)), (17, (0x10000,))):
        cases.append({"handle": handle, "guids": list(guids), **run(rsb.build_object_array_property_payload, handle, guids)})
    out["object_array"] = cases

    cases = []
    for handle, guids in ((31, (WUSHU_GUID,)), (31, ()), (31, (WUSHU_GUID, (1, 2, 3, 4), (0xFFFFFFFF,) * 4)),
                          (200, ((0, 0, 0, 0),) * 40), (1, ((7, 7, 7, 7),) * 256), (1, ((7, 7, 7, 7),) * 257),
                          (0, (WUSHU_GUID,)), (0x10000, (WUSHU_GUID,))):
        cases.append({"handle": handle, "guids": [list(g) for g in guids],
                      **run(rsb.build_guid_array_property_payload, handle, guids)})
    out["guid_array"] = cases

    def field(handle, ops):
        w = BitWriter()
        for value, width in ops:
            if width is None:
                w.append_packed_uint(value)
            else:
                w.append_bits(value, width)
        return (handle, w.to_bytes(), w.bit_count)

    cases = []
    for fields in ((field(15, [(6, None)]), field(16, [(10, None)])),
                   (field(16, [(104, None)]), field(17, [(1, 3)]), field(18, [(1, 3)])),
                   (field(1, [(0, 0)]),),
                   (field(127, [(0xFFFFFFFF, 32)]), field(128, [(1, 1)]), field(0xFFFFFFFF, [(300, None)])),
                   (), (field(16, [(1, 3)]), field(16, [(1, 3)])), (field(0, [(1, 3)]),),
                   (field(18, [(1, 3)]), field(17, [(1, 3)]))):
        cases.append({"fields": [[h, b.hex(), n] for h, b, n in fields],
                      **run(rsb.build_ordered_property_payload, tuple(fields))})
    out["ordered"] = cases

    out["phase"] = [{"value": v, **run(rsb.build_phase_property_payload, v)} for v in range(10)]
    out["match_state"] = [{"name": n, **run(rsb.build_match_state_property_payload, n)}
                          for n in ("InProgress", "WaitingToStart", "inprogress")]
    subjects = ("7c9f41a4-65e2-4d81-a8d5-f879c3e5ab61", "00000000-0000-0000-0000-000000000000",
                "ffffffff-ffff-ffff-ffff-ffffffffffff", "7C9F41A4-65E2-4D81-A8D5-F879C3E5AB61",
                "{7c9f41a4-65e2-4d81-a8d5-f879c3e5ab61}", "7c9f41a465e24d81a8d5f879c3e5ab61",
                "urn:uuid:7c9f41a4-65e2-4d81-a8d5-f879c3e5ab61", "7c9f41a4-65e2-4d81-a8d5-f879c3e5ab6",
                "7c9f41a4-65e24-d81-a8d5-f879c3e5ab61", "7c9f41a4-65e2-4d81-a8d5-f879c3e5ab6g", "")
    out["unique_id"] = [{"subject": s, **run(rsb.build_player_unique_id_property_payload, s)} for s in subjects]
    out["gameplay_start"] = [{"start_time": number(t), **run(rsb.build_gameplay_start_property_payload, t)}
                             for t in (0.0, -0.0, 1.0, -1e-45, float("nan"), float("inf"))]
    out["shop"] = [{"handles": list(h), **run(rsb.build_range_shop_settings_payload, h)}
                   for h in ((44, 48, 49), (44,), (49, 44, 44), (44, 45, 46, 47, 48, 49), (49,), (), (43,), (44, 50))]
    out["pistol_role"] = [{"width": w, **run(rsb.build_base_pistol_autonomous_role_payload, w)} for w in (3, 2, 1, 4, 8)]
    # rsb:1438-1441 guards; (1, 2) and (1, 3) fail in append_bits, not in the guard
    out["pawn_role"] = [{"width": w, "value": v, **run(rsb.build_pawn_autonomous_role_payload, w, v)}
                        for w, v in ((3, 2), (3, 3), (8, 2), (8, 0), (1, 1), (2, 3), (1, 2), (1, 3), (0, 0), (9, 2),
                                     (3, 4), (8, 4))]

    # the inline leaves the generic rust helpers stand for
    def int_leaf(handle, value):
        # rsb:849-853 (also rsb:1019-1023 with handle 3)
        rep = BitWriter()
        rep.append_bits(0, 1)
        rep.append_packed_uint(handle)
        rep.append_bits(value, 32)
        rep.append_packed_uint(0)
        return rep.to_bytes(), rep.bit_count

    def bits_leaf(handle, value, width):
        # hs:3601-3606 shape
        rep = BitWriter()
        rep.append_bits(0, 1)
        rep.append_packed_uint(handle)
        rep.append_bits(value, width)
        rep.append_packed_uint(0)
        return rep.to_bytes(), rep.bit_count

    def null_leaf(handle):
        # hs:2012-2016
        hr_null = BitWriter()
        hr_null.append_bits(0, 1)
        hr_null.append_packed_uint(handle)
        hr_null.append_packed_uint(0)
        hr_null.append_packed_uint(0)
        return hr_null.to_bytes(), hr_null.bit_count

    out["int_property"] = [{"handle": h, "value": v, **run(int_leaf, h, v)}
                           for h, v in ((197, 8), (3, 9000), (1, 0), (300, 0xFFFFFFFF), (0xFFFF, 0x80000000))]
    out["bits_property"] = [{"handle": h, "value": v, "width": w, **run(bits_leaf, h, v, w)}
                            for h, v, w in ((3, 1, 8), (67, 4, 8), (4, 2, 3), (1, 0, 0), (200, 1, 1),
                                            (5, 0xFFFFFFFFFFFFFFFF, 64), (3, 256, 8))]
    out["null_object"] = [{"handle": h, **run(null_leaf, h)} for h in (194, 1, 127, 128, 0xFFFF)]

    # ---- section 6.3: rpc payloads ----
    out["client_restart"] = [run(rsb.build_client_restart_rpc_payload)]
    cases = []
    for field_index, class_max, guid in ((52, 178, 109), (0, 1, 1), (177, 178, 0xFFFFFFFF), (127, 178, 109),
                                         (128, 178, 109), (52, 0xFFFFFFFF, 109), (178, 178, 109), (52, 178, 0), (5, 3, 109),
                                         (0xFFFFFFFE, 0xFFFFFFFF, 1), (0, 0, 109)):
        cases.append({"field": field_index, "max": class_max, "guid": guid,
                      **run(rsb.build_client_set_hud_payload, field_index, class_max, guid)})
    out["set_hud"] = cases

    cases = []
    for latest, index, field_index, class_max in ((26, 0, 41, 53), (0xFFFFFFFF, 1, 41, 53), (0xFFFFFFFF, 5542, 41, 53),
                                                  (0xFFFFFFFF, 0x12345678, 41, 53), (1, 0, 41, 53), (162, 0, 41, 53),
                                                  (0, 0, 0, 1), (7, 9, 52, 53), (7, 9, 53, 53), (7, 9, 200, 178)):
        cases.append({"latest": latest, "index": index, "field": field_index, "max": class_max,
                      **run(rsb.build_input_ack_rpc_payload, latest, index, field_index, class_max)})
    out["input_ack"] = cases

    cases = []
    for payload, bits in ((b"", 0), (bytes.fromhex("9bac00"), 17), (bytes.fromhex("01"), 1), (bytes(range(16)), 128),
                          (bytes(range(200)), 1597)):
        cases.append({"plain": payload.hex(), "bits": bits, **run(rsb.build_packed_bits_struct, payload, bits)})
    out["packed_bits"] = cases

    cases = []
    many = tuple(f"/Game/Maps/L{i}" for i in range(17))
    for names, field_index, class_max, loaded, visible in (
            (rsb.range_gameplay_sublevels, 66, 178, True, True), (rsb.range_gameplay_sublevels[:1], 66, 178, True, True),
            (rsb.range_gameplay_sublevels, 66, 178, False, True), (rsb.range_gameplay_sublevels, 66, 178, True, False),
            (many[:16], 66, 178, True, True), (many, 66, 178, True, True), ((), 66, 178, True, True),
            (("/Script/Engine",), 66, 178, True, True), (rsb.range_gameplay_sublevels, 0, 1, True, True),
            (rsb.range_gameplay_sublevels, 179, 178, True, True), (rsb.range_gameplay_sublevels, 178, 178, True, True),
            (("/Game/",), 66, 178, True, True), (("/Game/" + "L" * 234,), 66, 178, False, False),
            (("/Game/" + "L" * 235,), 66, 178, True, True), (("/Game/Café",), 66, 178, True, True),
            (("/Game/A", "/Script/B"), 66, 178, True, True)):
        cases.append({"names": list(names), "field": field_index, "max": class_max, "loaded": loaded, "visible": visible,
                      **run(rsb.build_client_update_multiple_levels_streaming_status_payload, names, field_index, class_max,
                            loaded, visible)})
    out["streaming"] = cases

    no_param = ((26, 178), (170, 178), (0, 0), (178, 178), (179, 178), (127, 178), (128, 178), (0xFFFFFFFF, 0xFFFFFFFF))
    out["flush"] = [{"field": f, "max": m, **run(rsb.build_client_flush_level_streaming_payload, f, m)} for f, m in no_param]
    out["range_settings"] = [{"field": f, "max": m, **run(rsb.build_client_update_range_settings_payload, f, m)}
                             for f, m in no_param]

    moves = (0, 5, 127, 128, 201, 3908, 16383, 16384, 20000, 2097152, 0xFFFFFFFF)
    vectors = ((SPAWN, (0.0, 0.0, 0.0)), ((0.0, -0.0, 1e-300), (1.5, -2.25, 600.0)),
               ((float("nan"), 0.0, 0.0), (0.0, 0.0, 0.0)), (SPAWN, (0.0, float("inf"), 0.0)))
    cases = []
    for move in moves:
        for location, velocity in vectors:
            cases.append({"move": move, "location": vector(location), "velocity": vector(velocity),
                          **run(build_correction_container, move, location, velocity)})
    out["correction_container"] = cases
    cases = []
    for move in moves:
        for location, velocity in vectors[:2]:
            for field_index, class_max in ((25, 53), (0, 1), (53, 53)):
                cases.append({"move": move, "location": vector(location), "velocity": vector(velocity),
                              "field": field_index, "max": class_max,
                              **run(build_correction_rpc_payload, move, location, field_index, class_max, velocity)})
    out["correction_rpc"] = cases

    cases = []
    for payload, bits in (rsb.build_client_update_range_settings_payload(),
                          rsb.build_client_flush_level_streaming_payload(),
                          rsb.build_client_update_multiple_levels_streaming_status_payload(rsb.range_gameplay_sublevels),
                          rsb.build_client_set_hud_payload(52, 178, 109), rsb.build_client_restart_rpc_payload(),
                          (bytes.fromhex("0000"), 9), (bytes.fromhex("00"), 8)):
        cases.append({"plain": payload.hex(), "bits": bits, **run(rsb.build_controller_rpc_body, payload, bits)})
    out["controller_rpc_body"] = cases

    # ---- section 6.2: complete bodies ----
    out["actor_int_body"] = [{"actor": a, "handle": h, "value": v, **run(rsb.build_actor_int_property_body, a, h, v)}
                             for a, h, v in ((6, 197, 1), (6, 197, 8), (6, 197, 49), (6, 197, 0), (6, 197, 0x7FFFFFFF),
                                             (2, 300, 12345), (6, 197, -1), (6, 0, 5))]
    out["game_state_bool_body"] = [{"handle": h, "value": v, **run(rsb.build_game_state_bool_body, h, v)}
                                   for h, v in ((46, True), (46, False), (18, True), (45, True), (63, False), (1, True),
                                                (64, True), (0, True))]
    cases = []
    for args in ((2,) + rsb.range_game_mode_class, (2,) + rsb.range_hud_config_class,
                 (6, "X_C", "/Game/X", 0xFFFF, 0xFFFD, 0xFFFF), (2, "X_C", "/Game/X", 0x10001, 5, 15),
                 (2, "X_C", "/Game/X", 261, 263, 0), (2, "X_C", "/Game/X", 261, 263, 0x10000), (2, "X_C", "/Script/X", 261, 263, 15)):
        cases.append({"actor": args[0], "class_name": args[1], "package": args[2], "class_guid": args[3],
                      "package_guid": args[4], "handle": args[5], **run(rsb.build_actor_class_property_body, *args)})
    out["class_property_body"] = cases
    out["money_body"] = [{"money": m, "component": c, **run(rsb.build_money_component_body, m, c)}
                         for m, c in ((9000, 254), (0, 254), (0x7FFFFFFF, 254), (9000, 300), (-1, 254))]
    cases = []
    for args in ((100, 104, "BlueTeam", 1), (102, 106, "RedTeam", 2), (100, 104, "BlueTeam", None), (102, 106, "RedTeam", None),
                 (100, 106, "RedTeam", 2), (104, 104, "BlueTeam", 1), (100, 108, "BlueTeam", 1), (100, 104, "GreenTeam", 1),
                 (100, 104, "BlueTeam", 3), (100, 104, "BlueTeam", 0)):
        cases.append({"actor": args[0], "component": args[1], "name": args[2], "role": args[3],
                      **run(rsb.build_team_state_component_update, *args)})
    out["team_component"] = cases
    out["state_link"] = [{"name": n, "guid": g, **run(hs.build_equippable_state_link_body, n, g)}
                         for n, g in (("InactiveState", 98), ("EquipState", 99), ("InactiveState", 99), ("EquipState", 98),
                                      ("IdleState", 98), ("InactiveState", 100))]

    # the python module constants the rust consts copy, read from the modules
    # themselves so a rust value can never drift from its source
    import range_melee_equippable as rme
    out["constants"] = {
        "range_shop_setting_handles": list(rsb.range_shop_setting_handles),
        "starting_equippable_first_channel": rsb.starting_equippable_first_channel,
        "starting_equippable_first_guid": rsb.starting_equippable_first_guid,
        "inventory_net_guid": rsb.inventory_net_guid,
        "item_slot_class_net_guid": rsb.item_slot_class_net_guid,
        "first_item_slot_net_guid": rsb.first_item_slot_net_guid,
        "actor_owner_handle": rsb.actor_owner_handle,
        "item_slot_contents_handle": rsb.item_slot_contents_handle,
        "base_pistol_actor_guid": rsb.base_pistol_actor_guid,
        "base_pistol_state_machine_guid": rsb.base_pistol_state_machine_guid,
        "base_pistol_inactive_state_guid": rsb.base_pistol_state_guids["InactiveState"],
        "auth_current_state_handle": rsb.auth_current_state_handle,
        "player_state_ultimate_points_handle": rsb.player_state_ultimate_points_handle,
        "range_game_mode_class": list(rsb.range_game_mode_class),
        "range_hud_config_class": list(rsb.range_hud_config_class),
        "range_gameplay_sublevels": list(rsb.range_gameplay_sublevels),
        "client_update_multiple_levels_streaming_status_field": rsb.client_update_multiple_levels_streaming_status_field,
        "client_flush_level_streaming_field": rsb.client_flush_level_streaming_field,
        "shooting_range_controller_class_max": rsb.shooting_range_controller_class_max,
        "client_update_range_settings_field": rsb.client_update_range_settings_field,
        "game_info_infinite_ammo_handle": rsb.game_info_infinite_ammo_handle,
        "money_component_guid": rsb.money_component_guid,
        "money_handle": rsb.money_handle,
        "range_money": rsb.range_money,
        "melee_channel": rme.melee_channel,
        "melee_actor_guid": rme.melee_actor_guid,
        "melee_state_machine_guid": rme.melee_state_machine_guid,
        "melee_inactive_state_guid": rme.melee_state_guids["InactiveState"],
        "melee_auth_current_state_handle": rme.melee_auth_current_state_handle,
    }
    return out


SECTIONS = {"rep_builders": section_rep_builders}
