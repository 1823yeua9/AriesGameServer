"""Compare catalog-backed inventory mutations against the Python reference."""
import json
import subprocess
import sys
import tempfile
from pathlib import Path
from export_contracts import source_root

sys.path.insert(0, str(source_root))
from skin_inventory import SkinInventory
from backend import offline_player_id


def run(binary):
    with tempfile.TemporaryDirectory(prefix="aries-inventory-parity-") as temporary:
        directory = Path(temporary)
        catalog_path = source_root / "skin_catalog.json"
        reference = SkinInventory(offline_player_id, catalog_path, directory / "python.json")
        cases = []
        expected = []

        def record(method, path, body, result):
            cases.append({"method": method, "path": path, "body": body})
            expected.append({"status": 200, "payload": result})

        loadout_path = f"/.a.pvp.net/personalization/v3/players/{offline_player_id}/playerloadout"
        record("GET", "/dev/skins", {}, reference.snapshot())
        # Choose an entry with actual non-default variants, not a synthetic test skin.
        entry = next(item for item in reference.catalog.values() if len(reference._granted_component_ids(item, "chroma_id")) > 1)
        weapon, skin = entry["weapon_id"], entry["skin_id"]
        record("POST", "/dev/skins/grant", {"skin_id": skin}, reference.grant(skin))
        record("POST", "/dev/skins/grant", {"skin_id": skin}, reference.grant(skin))
        for chroma in sorted(reference._granted_component_ids(entry, "chroma_id")):
            loadout = {"version": reference.version, "guns": [{"iD": weapon, "skinId": skin, "skinLevelId": entry["skin_level_id"], "chromaId": chroma}]}
            record("PUT", loadout_path, loadout, reference.put_loadout(loadout))
            record("GET", loadout_path, {}, reference.get_loadout())
        record("PUT", "/dev/skins/equip", {"weapon_id": weapon, "skin_id": skin}, reference.equip(weapon, skin))
        record("POST", "/dev/skins/grant-all", {}, reference.grant_all())
        for type_id, entitlements in reference.entitlements().items():
            record("GET", f"/.a.pvp.net/store/v1/entitlements/{offline_player_id}/{type_id}", {}, {"Entitlements": entitlements})
        config = {"skin_catalog": str(catalog_path), "skin_state": str(directory / "rust.json")}
        config_path = directory / "config.json"
        config_path.write_text(json.dumps(config))
        process = subprocess.run([str(binary), "--replay", "--config", str(config_path)], input="".join(json.dumps(case) + "\n" for case in cases), capture_output=True, text=True, check=True)
        actual = [json.loads(line) for line in process.stdout.splitlines()]
        assert len(actual) == len(expected)
        for index, (left, right) in enumerate(zip(expected, actual)):
            if left != right:
                left_items = left["payload"].get("Entitlements", [])
                right_items = right["payload"].get("Entitlements", [])
                difference = next(((a, b) for a, b in zip(left_items, right_items) if a != b), None)
                raise AssertionError(f"inventory parity failed at {index}: {cases[index]['path']}; counts {len(left_items)}/{len(right_items)}; first difference {difference}")
        assert json.loads((directory / "python.json").read_text()) == json.loads((directory / "rust.json").read_text())
        print(f"PASS: {len(cases)} catalog-backed inventory contracts and identical persisted account JSON")


if __name__ == "__main__":
    root = Path(__file__).resolve().parents[1]
    run(Path(sys.argv[1]) if len(sys.argv) > 1 else root / "target/debug/aries-platform.exe")
