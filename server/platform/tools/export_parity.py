"""Record existing backend tests as request/response contracts (synthetic only)."""
import json
import sys
import unittest
from pathlib import Path
from export_contracts import source_root

sys.path.insert(0, str(source_root))
import backend
import test_backend

cases = []
original_route = backend.LocalRemotingBackend._route


def normalize(value, instance):
    if isinstance(value, dict):
        return {key: (0 if key in {"time", "timestamp"} else normalize(item, instance)) for key, item in value.items()}
    if isinstance(value, list):
        return [normalize(item, instance) for item in value]
    if isinstance(value, str):
        return value.replace(f"127.0.0.1:{instance.port}", "127.0.0.1:58089")
    return value


def record_route(self, method, path, body):
    state = {key: getattr(self, key) for key in (
        "party_state", "party_previous_state", "party_version", "session_loop_state",
        "session_version", "core_game_match_id", "core_game_player_version")}
    inventory = {key: getattr(self.skin_inventory, key) for key in ("version", "loadout")}
    # Freeze nested mutable objects before the request mutates them.
    inventory = json.loads(json.dumps(inventory))
    status, payload = original_route(self, method, path, body)
    if path in {"/riotclient/app-port", "/riotclient/v1/auth-token", "/product-integration/v1/settings-token"}:
        return status, payload
    cases.append({"method": method, "path": path, "raw_body": body.decode(), "session": state,
                  "inventory": inventory, "status": status, "payload": normalize(payload, self)})
    return status, payload


if __name__ == "__main__":
    backend.LocalRemotingBackend._route = record_route
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(test_backend.BackendTests)
    result = unittest.TextTestRunner(verbosity=1).run(suite)
    if not result.wasSuccessful():
        raise SystemExit("Reference tests failed; no fixtures exported")
    output = Path(__file__).resolve().parents[1] / "data" / "parity.json"
    output.write_text(json.dumps(cases, indent=2) + "\n", encoding="utf-8")
    print(f"Exported {len(cases)} reference requests")
