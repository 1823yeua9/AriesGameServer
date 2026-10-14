"""inline bring-up bodies from range_udp_handshake.run_probe, re-run offline.

each stage function below repeats the exact statements run_probe executes to
build the bunch body (hs = range_udp_handshake.py line refs), with the
production arguments (docs/port/03_bringup_a.md section 7, 04_bringup_b.md
section 8 and 9). no socket, no live process.
"""

DESIRED_CLASS = (0xADD6443A, 0x41BDE414, 0xF6ADE58D, 0x267F4E95)  # hs:4753, parsed hs:720-723
MOVE_CORRECTION_LOCATION = (3581.634, -12019.322, 1294.153)  # launch.ps1:176


def hb(pair):
    body, bits = pair
    return {"hex": body.hex(), "bit_count": bits}


def actor_rep(plain, seed):
    """the 3-statement rep block every link stage repeats (e.g. hs:2415-2419)."""
    from range_server_bunch import append_actor_rep_layout_payload, encode_short_actor_rep_payload

    plain_payload, payload_bits = plain
    encoded_payload = encode_short_actor_rep_payload(plain_payload, payload_bits, 0, seed)
    return append_actor_rep_layout_payload(b"", 0, encoded_payload, payload_bits)


def actor_rpc(rpc, seed):
    """the rpc block (hs:3360-3364, 4496-4499)."""
    from range_server_bunch import append_actor_rpc_content_block, encode_short_actor_rep_payload

    rpc_payload, payload_bits = rpc
    encoded_payload = encode_short_actor_rep_payload(rpc_payload, payload_bits, 0, seed)
    return append_actor_rpc_content_block(b"", 0, encoded_payload, payload_bits)


# ---- part A ----

def a01_world_settings_open():
    from range_server_bunch import build_exported_static_actor_reference
    # hs:2196-2197
    return build_exported_static_actor_reference(
        ("AresWorldSettings", "PersistentLevel", "Range", "/Game/Maps/Poveglia/Range"))


def a06_pawn_open():
    from range_server_bunch import build_dynamic_wushu_pawn_reference
    # hs:2357-2362, pawn_spawn_location_probe on
    pawn_spawn_location = (3581.634, -12019.322, 1294.153)
    return build_dynamic_wushu_pawn_reference(pawn_spawn_location)


def a17_inventory_open():
    from range_server_bunch import BitWriter, encode_short_actor_rep_payload
    # hs:2706-2729
    INV_SUBOBJ_GUID = 60
    header = BitWriter()
    header.append_bits(0, 1)
    header.append_bits(1, 32)
    header.append_packed_uint(INV_SUBOBJ_GUID)
    header.append_bits(5, 8)
    header.append_packed_uint(10)
    header.append_bits(0, 8)
    header.append_fstring_ascii("InventoryComponent")
    header.append_bits(0, 32)
    header.append_bits(1, 1)
    header.append_bits(0, 1)
    header.append_packed_uint(INV_SUBOBJ_GUID)
    header.append_bits(1, 1)
    rep_bits_writer = BitWriter()
    rep_bits_writer.append_bits(0, 1)
    rep_bits_writer.append_packed_uint(0)
    rep_bytes = rep_bits_writer.to_bytes()
    rep_bits = rep_bits_writer.bit_count
    enc = encode_short_actor_rep_payload(rep_bytes, rep_bits, 0, 10)
    header.append_packed_uint(rep_bits)
    header.append_payload(enc, rep_bits)
    return header.to_bytes(), header.bit_count


def a18_item_slot_create():
    from range_server_bunch import BitWriter, encode_short_actor_rep_payload
    # hs:2767-2806 (slot 0)
    CLASS_GUID = 61
    PACKAGE_GUID = 63
    SLOT_GUID = 62
    header = BitWriter()
    header.append_bits(0, 1)
    header.append_bits(1, 32)
    header.append_packed_uint(CLASS_GUID)
    header.append_bits(5, 8)
    header.append_packed_uint(PACKAGE_GUID)
    header.append_bits(5, 8)
    header.append_packed_uint(0)
    header.append_fstring_ascii("/Script/ShooterGame")
    header.append_bits(0, 32)
    header.append_fstring_ascii("ItemSlot")
    header.append_bits(0, 32)
    header.append_bits(1, 1)
    header.append_bits(0, 1)
    header.append_packed_uint(SLOT_GUID)
    header.append_bits(0, 1)
    header.append_bits(0, 1)
    header.append_packed_uint(CLASS_GUID)
    header.append_bits(0, 1)
    header.append_packed_uint(60)
    rep_writer = BitWriter()
    rep_writer.append_bits(0, 1)
    rep_writer.append_packed_uint(0)
    rep_bytes = rep_writer.to_bytes()
    rep_bits = rep_writer.bit_count
    enc = encode_short_actor_rep_payload(rep_bytes, rep_bits, 0, 10)
    header.append_packed_uint(rep_bits)
    header.append_payload(enc, rep_bits)
    return header.to_bytes(), header.bit_count


def a19_item_slot_link():
    from range_server_bunch import BitWriter, build_object_property_payload, encode_short_actor_rep_payload
    # hs:2840-2851 (slot 0)
    header = BitWriter()
    header.append_bits(1, 1)
    header.append_bits(0, 1)
    header.append_packed_uint(60)
    header.append_bits(1, 1)
    rep_bytes, rep_bits = build_object_property_payload(3, 62)
    enc = encode_short_actor_rep_payload(rep_bytes, rep_bits, 0, 10)
    header.append_packed_uint(rep_bits)
    header.append_payload(enc, rep_bits)
    return header.to_bytes(), header.bit_count


def a20_scale_pair(slot_i):
    from range_server_bunch import BitWriter, build_object_property_payload, encode_short_actor_rep_payload
    # hs:2889-2947 (slots 1..15)
    slot_guid = 62 + 2 * slot_i
    wire_handle = 3 + slot_i
    create_hdr = BitWriter()
    create_hdr.append_bits(0, 1)
    create_hdr.append_bits(1, 32)
    create_hdr.append_packed_uint(61)
    create_hdr.append_bits(5, 8)
    create_hdr.append_packed_uint(63)
    create_hdr.append_bits(5, 8)
    create_hdr.append_packed_uint(0)
    create_hdr.append_fstring_ascii("/Script/ShooterGame")
    create_hdr.append_bits(0, 32)
    create_hdr.append_fstring_ascii("ItemSlot")
    create_hdr.append_bits(0, 32)
    create_hdr.append_bits(1, 1)
    create_hdr.append_bits(0, 1)
    create_hdr.append_packed_uint(slot_guid)
    create_hdr.append_bits(0, 1)
    create_hdr.append_bits(0, 1)
    create_hdr.append_packed_uint(61)
    create_hdr.append_bits(0, 1)
    create_hdr.append_packed_uint(60)
    empty_rep = BitWriter()
    empty_rep.append_bits(0, 1)
    empty_rep.append_packed_uint(0)
    erep_enc = encode_short_actor_rep_payload(empty_rep.to_bytes(), empty_rep.bit_count, 0, 10)
    create_hdr.append_packed_uint(empty_rep.bit_count)
    create_hdr.append_payload(erep_enc, empty_rep.bit_count)

    link_hdr = BitWriter()
    link_hdr.append_bits(1, 1)
    link_hdr.append_bits(0, 1)
    link_hdr.append_packed_uint(60)
    link_hdr.append_bits(1, 1)
    lp_bytes, lp_bits = build_object_property_payload(wire_handle, slot_guid)
    lp_enc = encode_short_actor_rep_payload(lp_bytes, lp_bits, 0, 10)
    link_hdr.append_packed_uint(lp_bits)
    link_hdr.append_payload(lp_enc, lp_bits)
    return (create_hdr.to_bytes(), create_hdr.bit_count), (link_hdr.to_bytes(), link_hdr.bit_count)


def a21_base_pistol_open_plain():
    from range_server_bunch import build_dynamic_actor_reference
    # hs:3002-3007
    return build_dynamic_actor_reference(
        archetype_chain=("Default__BasePistol_C",
                         "/Game/Equippables/Guns/Sidearms/BasePistol/BasePistol"),
        actor_net_guid=94, archetype_net_guid=95,
        package_net_guid=97, level_net_guid=5,
        spawn_location=None)


def a22_pistol_slot_contents():
    from range_server_bunch import BitWriter, build_object_property_payload, encode_short_actor_rep_payload
    # hs:3062-3076
    hdr = BitWriter()
    hdr.append_bits(1, 1)
    hdr.append_bits(0, 1)
    hdr.append_packed_uint(64)
    hdr.append_bits(0, 1)
    hdr.append_bits(0, 1)
    hdr.append_packed_uint(61)
    hdr.append_bits(0, 1)
    hdr.append_packed_uint(60)
    rep_bytes, rep_bits = build_object_property_payload(1, 94)
    enc = encode_short_actor_rep_payload(rep_bytes, rep_bits, 0, 10)
    hdr.append_packed_uint(rep_bits)
    hdr.append_payload(enc, rep_bits)
    return hdr.to_bytes(), hdr.bit_count


def a23_pistol_owner():
    from range_server_bunch import BitWriter, build_object_property_payload, encode_short_actor_rep_payload
    # hs:3107-3113
    rep_bytes_f, rep_bits_f = build_object_property_payload(12, 10)
    enc_f = encode_short_actor_rep_payload(rep_bytes_f, rep_bits_f, 0, 94)
    hdr_f = BitWriter()
    hdr_f.append_bits(1, 1)
    hdr_f.append_bits(1, 1)
    hdr_f.append_packed_uint(rep_bits_f)
    hdr_f.append_payload(enc_f, rep_bits_f)
    return hdr_f.to_bytes(), hdr_f.bit_count


def a24_pistol_role():
    from range_server_bunch import BitWriter, build_base_pistol_autonomous_role_payload, encode_short_actor_rep_payload
    # hs:3149-3156
    role_payload, role_bits = build_base_pistol_autonomous_role_payload()
    enc_g = encode_short_actor_rep_payload(role_payload, role_bits, 0, 94)
    hdr_g = BitWriter()
    hdr_g.append_bits(1, 1)
    hdr_g.append_bits(1, 1)
    hdr_g.append_packed_uint(role_bits)
    hdr_g.append_payload(enc_g, role_bits)
    return hdr_g.to_bytes(), hdr_g.bit_count


def a25_pistol_state_machine_open():
    from range_server_bunch import BitWriter, encode_short_actor_rep_payload
    # hs:3188-3208
    SM_SUBOBJ_GUID = 96
    hdr_h = BitWriter()
    hdr_h.append_bits(0, 1)
    hdr_h.append_bits(1, 32)
    hdr_h.append_packed_uint(SM_SUBOBJ_GUID)
    hdr_h.append_bits(5, 8)
    hdr_h.append_packed_uint(94)
    hdr_h.append_bits(0, 8)
    hdr_h.append_fstring_ascii("EquippableStateMachine")
    hdr_h.append_bits(0, 32)
    hdr_h.append_bits(1, 1)
    hdr_h.append_bits(0, 1)
    hdr_h.append_packed_uint(SM_SUBOBJ_GUID)
    hdr_h.append_bits(1, 1)
    rep_h = BitWriter()
    rep_h.append_bits(0, 1)
    rep_h.append_packed_uint(0)
    enc_h = encode_short_actor_rep_payload(rep_h.to_bytes(), rep_h.bit_count, 0, 94)
    hdr_h.append_packed_uint(rep_h.bit_count)
    hdr_h.append_payload(enc_h, rep_h.bit_count)
    return hdr_h.to_bytes(), hdr_h.bit_count


def a26_inactive_state_link():
    from range_udp_handshake import build_equippable_state_link_body
    # hs:3239-3241
    inactive_state_guid = 98
    return build_equippable_state_link_body("InactiveState", inactive_state_guid)


def a27_auth_correct(correction_idx):
    from range_server_bunch import BitWriter, encode_short_actor_rep_payload
    # hs:3315-3333
    rep = BitWriter()
    rep.append_bits(0, 1)
    rep.append_packed_uint(25)
    rep.append_packed_uint(94)
    rep.append_packed_uint(26)
    rep.append_bits(correction_idx, 32)
    rep.append_packed_uint(0)
    rep_bytes = rep.to_bytes()
    rep_bits = rep.bit_count
    enc = encode_short_actor_rep_payload(rep_bytes, rep_bits, 0, 10)
    hdr = BitWriter()
    hdr.append_bits(1, 1)
    hdr.append_bits(0, 1)
    hdr.append_packed_uint(60)
    hdr.append_bits(1, 1)
    hdr.append_packed_uint(rep_bits)
    hdr.append_payload(enc, rep_bits)
    return hdr.to_bytes(), hdr.bit_count


# ---- part B ----

def b5_team_link(team_index):
    from range_server_bunch import build_team_state_component_update
    # hs:3480-3484
    component_name = "BlueTeam" if team_index == 0 else "RedTeam"
    return build_team_state_component_update(
        100 + 2 * team_index, 104 + 2 * team_index,
        component_name, 1 + team_index)


def b7_bomb_team_open():
    from range_server_bunch import BitWriter, encode_short_actor_rep_payload
    # hs:3548-3568
    BOMB_TEAM_GUID = 108
    hdr_bt = BitWriter()
    hdr_bt.append_bits(0, 1)
    hdr_bt.append_bits(1, 32)
    hdr_bt.append_packed_uint(BOMB_TEAM_GUID)
    hdr_bt.append_bits(5, 8)
    hdr_bt.append_packed_uint(6)
    hdr_bt.append_bits(0, 8)
    hdr_bt.append_fstring_ascii("BombTeam")
    hdr_bt.append_bits(0, 32)
    hdr_bt.append_bits(1, 1)
    hdr_bt.append_bits(0, 1)
    hdr_bt.append_packed_uint(BOMB_TEAM_GUID)
    hdr_bt.append_bits(1, 1)
    rep_bt = BitWriter()
    rep_bt.append_bits(0, 1)
    rep_bt.append_packed_uint(0)
    enc_bt = encode_short_actor_rep_payload(rep_bt.to_bytes(), rep_bt.bit_count, 0, 6)
    hdr_bt.append_packed_uint(rep_bt.bit_count)
    hdr_bt.append_payload(enc_bt, rep_bt.bit_count)
    return hdr_bt.to_bytes(), hdr_bt.bit_count


def b8_bomb_team_number(bomb_team_number_value, bomb_team_number_bits):
    from range_server_bunch import BitWriter, encode_short_actor_rep_payload
    # hs:3601-3615
    rep_bt2 = BitWriter()
    rep_bt2.append_bits(0, 1)
    rep_bt2.append_packed_uint(3)
    rep_bt2.append_bits(bomb_team_number_value & ((1 << bomb_team_number_bits) - 1),
                        bomb_team_number_bits)
    rep_bt2.append_packed_uint(0)
    enc_bt2 = encode_short_actor_rep_payload(rep_bt2.to_bytes(), rep_bt2.bit_count, 0, 6)
    hdr_bt2 = BitWriter()
    hdr_bt2.append_bits(1, 1)
    hdr_bt2.append_bits(0, 1)
    hdr_bt2.append_packed_uint(108)
    hdr_bt2.append_bits(1, 1)
    hdr_bt2.append_packed_uint(rep_bt2.bit_count)
    hdr_bt2.append_payload(enc_bt2, rep_bt2.bit_count)
    return hdr_bt2.to_bytes(), hdr_bt2.bit_count


def b13_hud_set(field_index, class_max_index):
    from range_server_bunch import (append_actor_rpc_content_block, build_class_netguid_export,
                                    build_client_set_hud_payload, encode_short_actor_rep_payload)
    # hs:3789-3797; hud_rpc = {"field_index", "class_max_index"} resolved live
    hud_rpc = {"field_index": field_index, "class_max_index": class_max_index}
    hud_export, hud_export_bits = build_class_netguid_export(
        "AresHUD_C", "/Game/UI/InGame/HUD/AresHUD", 109, 111)
    hud_rpc_payload, hud_rpc_bits = build_client_set_hud_payload(
        hud_rpc["field_index"], hud_rpc["class_max_index"], 109)
    hud_encoded = encode_short_actor_rep_payload(hud_rpc_payload, hud_rpc_bits, 0, 4)
    return append_actor_rpc_content_block(hud_export, hud_export_bits, hud_encoded, hud_rpc_bits)


def b18_starting_equippable(equip_i):
    from range_server_bunch import (build_channel_actor_owner_block, build_item_slot_contents_block,
                                    build_starting_equippable_open, starting_equippable_guids,
                                    wushu_starting_equippables)
    # hs:4020-4024, 4069-4070 (default open; cosmetic and tuning opens are other owners')
    equip_guid = starting_equippable_guids(equip_i)[0]
    equip_slot = wushu_starting_equippables[equip_i][2]
    open_body, open_bits = build_starting_equippable_open(equip_i)
    owner_body, owner_bits = build_channel_actor_owner_block(equip_guid, 10)
    contents_body, contents_bits = build_item_slot_contents_block(equip_slot, equip_guid)
    return (open_body, open_bits), (owner_body, owner_bits), (contents_body, contents_bits)


def b22_viewtarget():
    from range_server_bunch import BitWriter, build_object_property_payload, encode_short_actor_rep_payload
    # hs:4281-4313 with viewtarget_name "ViewTarget", viewtarget_handle "3", viewtarget_transform on
    viewtarget_name = "ViewTarget"
    viewtarget_handle = "3"
    viewtarget_sent_sources = set()
    SUBOBJ_GUID = 50
    header = BitWriter()
    header.append_bits(0, 1)
    header.append_bits(1, 32)
    header.append_packed_uint(SUBOBJ_GUID)
    header.append_bits(5, 8)
    header.append_packed_uint(4)
    header.append_bits(0, 8)
    _vt_candidates = [n.strip() for n in viewtarget_name.split(",") if n.strip()]
    _vt_name = _vt_candidates[len(viewtarget_sent_sources) % len(_vt_candidates)]
    header.append_fstring_ascii(_vt_name)
    header.append_bits(0, 32)
    header.append_bits(1, 1)
    header.append_bits(0, 1)
    header.append_packed_uint(SUBOBJ_GUID)
    header.append_bits(1, 1)
    _vt_handles = [int(h) for h in str(viewtarget_handle).split(",") if h.strip()]
    _vt_handle = _vt_handles[len(viewtarget_sent_sources) % len(_vt_handles)]
    rep_bytes, rep_bits = build_object_property_payload(_vt_handle, 10)
    enc = encode_short_actor_rep_payload(rep_bytes, rep_bits, 0, 4)
    header.append_packed_uint(rep_bits)
    header.append_payload(enc, rep_bits)
    return header.to_bytes(), header.bit_count


# ---- move / input acks ----

def m_correction(move_number, location):
    from range_move_correction import build_correction_rpc_payload
    # hs:4483-4499 (move_ack_index 25, move_ack_class_max 53)
    rpc_payload, rpc_bits = build_correction_rpc_payload(move_number, location, 25, 53)
    return actor_rpc((rpc_payload, rpc_bits), 10)


def m_move_ack(move_number):
    from range_server_bunch import build_input_ack_rpc_payload
    # hs:4493-4499
    rpc_payload, rpc_bits = build_input_ack_rpc_payload(0xFFFFFFFF, move_number, 41, 53)
    return actor_rpc((rpc_payload, rpc_bits), 10)


def i_input_ack(latest_id):
    from range_server_bunch import build_input_ack_rpc_payload
    # hs:1541-1546
    rpc_payload, rpc_bits = build_input_ack_rpc_payload(latest_id, 0, 41, 53)
    return actor_rpc((rpc_payload, rpc_bits), 10)


# ---- per-packet bodies (part C) ----

def c_range_class_properties():
    from range_server_bunch import build_actor_class_property_body, range_game_mode_class, range_hud_config_class
    # hs:1789-1792
    return [build_actor_class_property_body(2, class_name, package, class_guid, package_guid, handle)
            for class_name, package, class_guid, package_guid, handle in (range_game_mode_class, range_hud_config_class)]


def c_hud_rebuild(hud_field, hud_max):
    from range_server_bunch import append_actor_rpc_content_block, build_client_set_hud_payload, encode_short_actor_rep_payload
    # hs:1793-1796
    hud_payload, hud_bits = build_client_set_hud_payload(hud_field, hud_max, 109)
    hud_encoded = encode_short_actor_rep_payload(hud_payload, hud_bits, 0, 4)
    return append_actor_rpc_content_block(b"", 0, hud_encoded, hud_bits)


def c_streaming():
    from range_server_bunch import (append_actor_rpc_content_block, build_client_flush_level_streaming_payload,
                                    build_client_update_multiple_levels_streaming_status_payload,
                                    encode_short_actor_rep_payload, range_gameplay_sublevels)
    # hs:1813-1820
    out = []
    for streaming_payload, streaming_bits in (
            build_client_update_multiple_levels_streaming_status_payload(range_gameplay_sublevels),
            build_client_flush_level_streaming_payload()):
        streaming_encoded = encode_short_actor_rep_payload(streaming_payload, streaming_bits, 0, 4)
        out.append(append_actor_rpc_content_block(b"", 0, streaming_encoded, streaming_bits))
    return out


def c_ultimate_points(ult_required):
    from range_server_bunch import build_actor_int_property_body, player_state_ultimate_points_handle
    # hs:1842-1843
    return build_actor_int_property_body(6, player_state_ultimate_points_handle, ult_required)


def c_range_settings_request():
    from range_server_bunch import build_client_update_range_settings_payload, build_controller_rpc_body
    # hs:1922-1923
    return build_controller_rpc_body(*build_client_update_range_settings_payload())


def c_infinite_ammo(infinite_ammo):
    from range_server_bunch import build_game_state_bool_body, game_info_infinite_ammo_handle
    # hs:1943-1944
    return build_game_state_bool_body(game_info_infinite_ammo_handle, infinite_ammo)


def c_hud_refresh():
    from range_server_bunch import (BitWriter as _HrWriter, append_actor_rep_layout_payload,
                                    build_object_property_payload, encode_short_actor_rep_payload)
    # hs:2012-2021
    hr_null = _HrWriter()
    hr_null.append_bits(0, 1)
    hr_null.append_packed_uint(194)
    hr_null.append_packed_uint(0)
    hr_null.append_packed_uint(0)
    out = []
    for hr_payload, hr_bits in ((hr_null.to_bytes(), hr_null.bit_count),
                                build_object_property_payload(194, 10)):
        hr_encoded = encode_short_actor_rep_payload(hr_payload, hr_bits, 0, 6)
        out.append(append_actor_rep_layout_payload(b"", 0, hr_encoded, hr_bits))
    return out


def section_rep_setup():
    import range_server_bunch as rsb
    from backend import offline_player_id
    from range_melee_equippable import (build_melee_auth_state_body, build_melee_autonomous_role_body,
                                        build_melee_state_machine_open)

    out = {}
    # part A, single-body stages
    out["a01_world_settings_open"] = [hb(a01_world_settings_open())]
    out["a02_game_state_open"] = [hb(rsb.build_dynamic_range_game_state_reference())]  # hs:2223
    out["a03_controller_open"] = [hb(rsb.build_dynamic_range_controller_reference())]  # hs:2291
    out["a04_player_state_open"] = [hb(rsb.build_dynamic_range_player_state_reference())]  # hs:2313
    out["a05_player_info_open"] = [hb(rsb.build_dynamic_player_info_reference())]  # hs:2335
    out["a06_pawn_open"] = [hb(a06_pawn_open())]
    # hs:2386-2391 (game_state_bool_probe_value 1; 0 for the edge)
    out["a07_game_state_bool"] = [{"value": bool(v), **hb(actor_rep(rsb.build_bool_property_payload(18, bool(v)), 2))}
                                  for v in (1, 0)]
    out["a08_controller_player_state"] = [hb(actor_rep(rsb.build_object_property_payload(15, 6), 4))]  # hs:2415
    out["a09_controller_pawn"] = [hb(actor_rep(rsb.build_object_property_payload(16, 10), 4))]  # hs:2441
    out["a10_player_state_player_info"] = [hb(actor_rep(rsb.build_object_property_payload(30, 8), 6))]  # hs:2471
    out["a11_spawned_character"] = [hb(actor_rep(rsb.build_object_property_payload(194, 10), 6))]  # hs:2508
    out["a11_possessed_character"] = [hb(actor_rep(rsb.build_object_property_payload(195, 10), 6))]  # hs:2508
    # hs:2545-2551
    out["a12_desired_class"] = [{"guid": list(g), **hb(actor_rep(rsb.build_desired_class_payload(*g, base_handle=201), 6))}
                                for g in (DESIRED_CLASS, (1, 2, 3, 4))]
    out["a13_player_info_controller"] = [hb(actor_rep(rsb.build_object_property_payload(15, 4), 8))]  # hs:2586
    out["a14_pawn_player_state"] = [hb(actor_rep(rsb.build_object_property_payload(15, 6), 10))]  # hs:2614
    out["a15_pawn_controller"] = [hb(actor_rep(rsb.build_object_property_payload(16, 4), 10))]  # hs:2640
    out["a16_pawn_role"] = [hb(actor_rep(rsb.build_pawn_autonomous_role_payload(role_value=2), 10))]  # hs:2667
    out["a17_inventory_open"] = [hb(a17_inventory_open())]
    creates = [{"slot": 0, **hb(a18_item_slot_create())}]
    links = [{"slot": 0, **hb(a19_item_slot_link())}]
    for slot_i in range(1, 16):
        create, link = a20_scale_pair(slot_i)
        creates.append({"slot": slot_i, **hb(create)})
        links.append({"slot": slot_i, **hb(link)})
    out["a18_a20_item_slot_create"] = creates
    out["a19_a20_item_slot_link"] = links
    out["a21_base_pistol_open"] = [hb(a21_base_pistol_open_plain())]
    out["a22_pistol_slot_contents"] = [hb(a22_pistol_slot_contents())]
    out["a23_pistol_owner"] = [hb(a23_pistol_owner())]
    out["a24_pistol_role"] = [hb(a24_pistol_role())]
    out["a25_pistol_state_machine_open"] = [hb(a25_pistol_state_machine_open())]
    out["a26_inactive_state_link"] = [hb(a26_inactive_state_link())]
    out["a27_auth_correct"] = [{"index": i, **hb(a27_auth_correct(i))} for i in (1, 2, 3, 4, 5, 6, 300, 0xFFFFFFFF)]

    # part B
    out["b1_client_restart"] = [hb(actor_rpc(rsb.build_client_restart_rpc_payload(), 4))]  # hs:3360
    out["b2_game_state_phase"] = [{"phase": p, **hb(actor_rep(rsb.build_phase_property_payload(p), 2))}
                                  for p in range(8)]  # hs:3387, production 4
    out["b3_team_state_open"] = [{"team": i, **hb(rsb.build_dynamic_base_team_state_reference(100 + 2 * i))}
                                 for i in (0, 1)]  # hs:3419-3421
    out["b4_team_state_array"] = [hb(actor_rep(rsb.build_object_array_property_payload(34, (100, 102)), 2))]  # hs:3449
    out["b5_team_state_link"] = [{"team": i, **hb(b5_team_link(i))} for i in (0, 1)]
    out["b6_player_state_team"] = [hb(actor_rep(rsb.build_object_property_payload(27, 100), 6))]  # hs:3512
    out["b7_bomb_team_open"] = [hb(b7_bomb_team_open())]
    out["b8_bomb_team_number"] = [{"value": v, "bits": n, **hb(b8_bomb_team_number(v, n))}
                                  for v, n in ((1, 8), (0, 8), (2, 8), (0x1FF, 8), (5, 3), (1, 1), (3, 0), (0xFFFFFFFF, 32),
                                               (0xFFFFFFFF, 64), (7, 16))]
    # hs:3646-3652
    out["b9_accessible_character"] = [{"guid": list(g), **hb(actor_rep(rsb.build_guid_array_property_payload(31, (g,)), 6))}
                                      for g in (DESIRED_CLASS, (0, 0, 0, 0))]
    # hs:3684-3689
    out["b10_unique_id"] = [{"subject": s, **hb(actor_rep(rsb.build_player_unique_id_property_payload(s), 6))}
                            for s in (offline_player_id, "00000000-0000-0000-0000-000000000000")]
    out["b11_money"] = [hb(rsb.build_money_component_body())]  # hs:3718
    out["b12_pawn_owner"] = [hb(actor_rep(rsb.build_object_property_payload(12, 4), 10))]  # hs:3748
    out["b13_hud_set"] = [{"field": f, "max": m, **hb(b13_hud_set(f, m))} for f, m in ((52, 178), (0, 1), (127, 178), (128, 178))]
    out["b14_match_state"] = [hb(actor_rep(rsb.build_match_state_property_payload(), 2))]  # hs:3876
    out["b15_gameplay_start"] = [hb(actor_rep(rsb.build_gameplay_start_property_payload(), 2))]  # hs:3912
    out["b16_player_array"] = [hb(actor_rep(rsb.build_object_array_property_payload(17, (6,)), 2))]  # hs:3946
    out["b17_range_shop"] = [hb(actor_rep(rsb.build_range_shop_settings_payload(), 2))]  # hs:3983
    cases = []
    for i in range(7):
        open_pair, owner_pair, contents_pair = b18_starting_equippable(i)
        cases.append({"index": i, "open": hb(open_pair), "owner": hb(owner_pair), "contents": hb(contents_pair)})
    out["b18_starting_equippable"] = cases
    out["b19_knife_role"] = [hb(build_melee_autonomous_role_body())]  # hs:4114
    out["b20_knife_state_machine"] = [hb(build_melee_state_machine_open())]  # hs:4144
    out["b21_knife_inactive_state"] = [hb(build_melee_auth_state_body("InactiveState"))]  # hs:4174
    out["b22_viewtarget"] = [hb(b22_viewtarget())]

    # move / input acks
    moves = (1, 5, 100, 127, 128, 201, 3908, 5542, 16383, 16384, 20000, 2097152, 0x12345678, 0xFFFFFFFF)
    out["m_correction"] = [{"move": m, "location": list(loc), **hb(m_correction(m, loc))}
                           for m in moves for loc in (MOVE_CORRECTION_LOCATION, (0.0, -0.0, 12.5))]
    out["m_move_ack"] = [{"move": m, **hb(m_move_ack(m))} for m in moves + (0,)]
    out["i_input_ack"] = [{"event": e, **hb(i_input_ack(e))} for e in (0, 1, 11, 26, 162, 0x7FFFFFFF, 0xFFFFFFFF)]

    # per-packet bodies
    game_mode, hud_config = c_range_class_properties()
    out["c_range_class_property"] = [{"which": "game_mode", **hb(game_mode)}, {"which": "hud_config", **hb(hud_config)}]
    out["c_hud_rebuild"] = [{"field": f, "max": m, **hb(c_hud_rebuild(f, m))} for f, m in ((52, 178), (128, 178))]
    streaming, flush = c_streaming()
    out["c_levels_streaming"] = [hb(streaming)]
    out["c_flush_level_streaming"] = [hb(flush)]
    out["c_ultimate_points"] = [{"points": p, **hb(c_ultimate_points(p))} for p in (1, 8, 49)]
    out["c_range_settings_request"] = [hb(c_range_settings_request())]
    out["c_infinite_ammo"] = [{"enabled": v, **hb(c_infinite_ammo(v))} for v in (True, False)]
    null_body, pawn_body = c_hud_refresh()
    out["c_hud_refresh_null"] = [hb(null_body)]
    out["c_hud_refresh_pawn"] = [hb(pawn_body)]
    return out


SECTIONS = {"rep_setup": section_rep_setup}
