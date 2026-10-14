"""Export deterministic platform DTOs from the working Python reference.

This is a migration/build tool, never part of the Rust runtime. No running
backend, game process, credential or saved account is read or modified.
"""
import ast
import json
import sys
import threading
from pathlib import Path

project_root = Path(__file__).resolve().parents[4]
source_root = project_root / "local_backend"
sys.path.insert(0, str(source_root))
import backend
from skin_inventory import SkinInventory


def reference_backend():
    instance = backend.LocalRemotingBackend.__new__(backend.LocalRemotingBackend)
    instance.port = 58089
    instance.auth_token = "fixture-auth-token"
    instance.machine_id = "AAAAAAAAQACAAAAAAAAAAQ=="
    instance.settings = {"publisher": "local", "product_id": "valorant", "patchline_id": "live"}
    instance.skin_inventory = SkinInventory(backend.offline_player_id)
    instance.party_state_lock = threading.Lock()
    instance.party_state = instance.party_previous_state = "DEFAULT"
    instance.party_version = instance.session_version = instance.core_game_player_version = 1
    instance.session_loop_state = "MENUS"
    instance.core_game_match_id = ""
    instance.log = lambda *args, **kwargs: None
    return instance


def main():
    instance = reference_backend()
    tree = ast.parse((source_root / "backend.py").read_text(encoding="utf-8"))
    paths = set()
    for node in ast.walk(tree):
        if isinstance(node, (ast.Constant, ast.JoinedStr)):
            try:
                value = ast.literal_eval(node) if isinstance(node, ast.Constant) else eval(
                    compile(ast.Expression(node), "<source-path>", "eval"), vars(backend))
            except (ValueError, NameError, TypeError, SyntaxError):
                continue
            if isinstance(value, str) and value.startswith("/") and " " not in value:
                paths.add(value)
    dynamic = ("/core-game/", "/session/", "/parties/", "/entitlements/", "/personalization/",
               "/riot-messaging-service/v1/messages/", "/rso-auth/", "/riotclient/", "/bloomline/")
    static_routes = {}
    for path in sorted(paths):
        if any(part in path for part in dynamic):
            continue
        status, payload = instance._route("GET", path, b"")
        if status == 200:
            static_routes[path] = payload
    templates = {
        "party": backend.make_local_party(),
        "party_player": instance._party_player_snapshot(),
        "session": instance._session_snapshot(),
        "range_match": instance._range_match_snapshot("{match_id}"),
        "presence": backend.make_local_chat_presence(),
        "nameset": backend.make_local_nameset("{player_id}"),
        "agent_entitlement": backend.make_local_agent_entitlement(),
        "storefront": backend.make_local_storefront(),
        "game_config": instance._route("GET", "/v1/config/na", b"")[1],
        "queue_config": instance._route("GET", "/.a.pvp.net/parties/v1/parties/customgameconfigs", b"")[1],
    }
    templates["presence"]["time"] = 0
    output = Path(__file__).resolve().parents[1] / "data" / "contracts.json"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps({"static_routes": static_routes, "templates": templates}, indent=2) + "\n", encoding="utf-8")
    print(f"Exported {len(static_routes)} static routes and {len(templates)} DTO templates")


if __name__ == "__main__":
    main()
