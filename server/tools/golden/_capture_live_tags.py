"""one-time, read-only capture of the running client's gameplay-tag node map
for the spec 08 tag net index fixtures (open question 1: the six
AbilityTuning net indices only exist in client memory).

it makes gameplay_tag_net_index's own reads (find_tags_manager, the node map
at manager+0x130, CommonlyReplicatedTags at manager+0x180, settings+0xBC,
node+0x50) and keeps the raw rows they consume, plus what
predict_net_indices returns live, in server/fixtures/world_tags_live_capture.json.
the golden section world_tags_live replays the rows through
predict_net_indices on a fake memory image, so the fixture never needs the
client again. the exporter skips this module (leading underscore).

usage: python _capture_live_tags.py   (exactly one VALORANT client running)
"""

import datetime
import json
import os
import struct
import sys
from pathlib import Path

here = Path(__file__).resolve().parent
project = Path(os.environ.get("ARIES_PROJECT_ROOT", here.parents[3]))
if not (project / "local_backend").is_dir():
    project = Path(r"C:/Users/Cayden/Desktop/valorant proj")
sys.path.insert(0, str(project / "local_backend"))
sys.dont_write_bytecode = True

TUNING_TAGS = ("AbilityTuning.Wushu.4.SmokeDuration", "AbilityTuning.Wushu.4.SmokeMaxCharges",
               "AbilityTuning.Wushu.E.DashVelocityMultiplier", "AbilityTuning.Wushu.E.PrimeDuration",
               "AbilityTuning.Wushu.E.MaxCharges", "AbilityTuning.Wushu.Q.MaxCharges")


def main():
    import gameplay_tag_net_index as gtni
    from launch_range import find_client
    from read_equippable_state import EquippableStateReader

    pid, image_base = find_client()
    if not pid or not image_base:
        raise SystemExit("no VALORANT client (or its image base) found")
    reader = EquippableStateReader(pid, image_base)
    try:
        handle, key = reader.handle, reader.key
        manager, settings = gtni.find_tags_manager(handle, image_base, key)
        node_map = manager + gtni.manager_node_map
        data, count, capacity = gtni.read_u64(handle, node_map), gtni.read_u32(handle, node_map + 8), gtni.read_u32(handle, node_map + 12)
        raw = gtni._read_block(handle, data, capacity * 0x28)
        nodes, undecoded = [], 0
        for index in range(capacity):
            comparison, number = struct.unpack_from("<II", raw, 0x28 * index)
            node = struct.unpack_from("<Q", raw, 0x28 * index + 0x10)[0]
            if not node:
                continue
            try:
                name = gtni.decode_name(handle, image_base, comparison, key)
            except Exception:
                undecoded += 1
                continue
            net_index = struct.unpack("<H", gtni.rws.read_bytes(handle, node + 0x50, 2))[0]
            nodes.append([name, number, net_index])
        common_data = gtni.read_u64(handle, manager + gtni.manager_common_tags)
        common_count = gtni.read_u32(handle, manager + gtni.manager_common_tags + 8)
        common = [gtni.decode_name(handle, image_base, gtni.read_u32(handle, common_data + 12 * i), key)
                  for i in range(common_count)]
        setting = struct.unpack("<i", gtni.rws.read_bytes(handle, settings + gtni.settings_first_bit_segment, 4))[0]
        live = gtni.predict_net_indices(handle, image_base, key)
    finally:
        reader.close()
    capture = {
        "source": "read-only capture of the running client by tools/golden/_capture_live_tags.py",
        "captured": datetime.datetime.now().isoformat(timespec="seconds"),
        "pid": pid, "count": count, "capacity": capacity, "undecoded": undecoded,
        "nodes": nodes, "common": common, "setting": setting,
        "python_live": {"tags": len(live.indices), "true_bit_num": live.true_bit_num,
                        "first_bit_segment": live.first_bit_segment, "constructed": live.constructed,
                        "tuning": {tag: live.indices.get(tag) for tag in TUNING_TAGS}},
    }
    out = here.parent.parent / "fixtures" / "world_tags_live_capture.json"
    out.write_text(json.dumps(capture, indent=1) + "\n", encoding="utf-8")
    print(f"{len(nodes)} of {count} tags, common {len(common)}, setting {setting}, "
          f"constructed {live.constructed}, true bits {live.true_bit_num} -> {out}")
    for tag in TUNING_TAGS:
        print(f"  {live.indices.get(tag)!s:>5} {tag}")


if __name__ == "__main__":
    main()
