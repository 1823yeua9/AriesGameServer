"""shared rep helpers (props basics, owner block, slot contents, inventory correction)."""


def hb(pair):
    body, bits = pair
    return {"hex": body.hex(), "bit_count": bits}


def section_rep_shared():
    import range_server_bunch as rsb

    out = {"bool": [], "object": [], "role": [], "owner": [], "contents": [], "correction": []}
    for h, v in ((18, True), (18, False), (46, True), (63, False)):
        out["bool"].append({"handle": h, "value": v, **hb(rsb.build_bool_property_payload(h, v))})
    for h, g in ((15, 6), (16, 10), (30, 8), (194, 10), (195, 10), (1, 200), (12, 10), (3, 62), (300, 65535)):
        out["object"].append({"handle": h, "guid": g, **hb(rsb.build_object_property_payload(h, g))})
    for width, value in ((3, 2), (3, 3), (8, 2)):
        out["role"].append({"width": width, "value": value, **hb(rsb.build_pawn_autonomous_role_payload(width, value))})
    for actor, owner in ((94, 10), (200, 10), (204, 10), (520, 10), (100000, 10)):
        out["owner"].append({"actor": actor, "owner": owner, **hb(rsb.build_channel_actor_owner_block(actor, owner))})
    for slot, item in ((1, 94), (10, 200), (2, 204), (3, 208), (0, 520), (11, 640)):
        out["contents"].append({"slot": slot, "item": item, **hb(rsb.build_item_slot_contents_block(slot, item))})
    for index, guid in ((1, 94), (5, 94), (6, None), (7, 204), (6, 200), (300, 520), (0xFFFFFFFF, 94)):
        out["correction"].append({"index": index, "guid": guid, **hb(rsb.build_inventory_correction_body(index, guid))})
    return out


SECTIONS = {"rep_shared": section_rep_shared}
