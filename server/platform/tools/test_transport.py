"""Real TLS, WAMP/RMS, admin, gate and disk tests; no game or production services."""
import base64
import concurrent.futures
import http.client
import json
import socket
import ssl
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from export_contracts import source_root

sys.path.insert(0, str(source_root))
import test_backend
import synthetic_client as client
from backend import Reader, read_frame, write_frame, encode_message_pack, decode_message_pack


def request(state, method, path, body=None, authorized=True):
    connection = http.client.HTTPConnection("127.0.0.1", state["admin_port"], timeout=5)
    headers = {"Authorization": "Bearer " + state["admin_token"]} if authorized else {}
    connection.request(method, path, None if body is None else json.dumps(body), headers)
    response = connection.getresponse()
    result = response.status, json.loads(response.read())
    connection.close()
    return result


def websocket(state, ca, binary):
    tls = client.make_context(ca).wrap_socket(socket.create_connection(("127.0.0.1", state["port"]), timeout=5), server_hostname="127.0.0.1")
    protocol = "" if binary else "Sec-WebSocket-Protocol: WAMP-1.0-RiotRemoting\r\n"
    tls.sendall((f"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {base64.b64encode(bytes(range(16))).decode()}\r\nSec-WebSocket-Version: 13\r\n{protocol}Authorization: {client.basic_auth(state['auth_token'])}\r\n\r\n").encode())
    reader = Reader(tls)
    assert b"101" in reader.read_until(b"\r\n\r\n").split(b"\r\n")[0]
    decode = lambda payload: decode_message_pack(payload) if binary else json.loads(payload)
    encode = lambda value: encode_message_pack(value) if binary else json.dumps(value).encode()
    assert decode(read_frame(reader)[1])[0] == 0
    write_frame(tls, 2 if binary else 1, encode([5, "OnJsonApiEvent_riot-messaging-service_v1_messages"]), masked=True)
    write_frame(tls, 2 if binary else 1, encode([2, 1, "riotclient.ping"]), masked=True)
    assert decode(read_frame(reader)[1]) == [3, 1, "pong"]  # subscription barrier
    status, result = request(state, "POST", "/dev/range/enter")
    assert status == 200 and result["match_id"]
    event = decode(read_frame(reader)[1])
    assert event[0] == 8 and event[2]["eventType"] == "Update"
    write_frame(tls, 8, b"", masked=True)
    tls.close()


def run(binary):
    with tempfile.TemporaryDirectory(prefix="aries-platform-test-") as directory:
        directory = Path(directory)
        pki = test_backend.make_pki(str(directory))
        config = {"port": 0, "admin_port": 0, "cert_path": pki["cert"], "key_path": pki["key"], "state_path": str(directory / "state.json"), "persistence_path": str(directory / "persistence"), "skin_catalog": str(source_root / "skin_catalog.json"), "skin_state": str(directory / "skins.json"), "startup_gate": str(directory / "gate")}
        config_path = directory / "config.json"
        config_path.write_text(json.dumps(config))
        with (directory / "server.log").open("w+") as log:
            process = subprocess.Popen([str(binary), "--config", str(config_path)], stdout=log, stderr=log)
            try:
                deadline = time.monotonic() + 15
                while not Path(config["state_path"]).exists():
                    if process.poll() is not None or time.monotonic() > deadline:
                        log.seek(0)
                        raise AssertionError(log.read())
                    time.sleep(.02)
                state = json.loads(Path(config["state_path"]).read_text())
                assert state["pid"] == process.pid
                flow = client.run_flow(state["port"], pki["ca"], state["auth_token"])
                assert flow["app_port"] == (200, state["port"])
                assert flow["auth_token"] == (200, state["auth_token"])
                assert flow["settings_token"][0] == 200
                assert flow["websocket"][2][0] == [3, 1, "pong"]
                assert client.https_request(state["port"], pki["ca"], "wrong", "GET", "/riotclient/app-port")[0] == 401
                assert request(state, "GET", "/health", authorized=False)[0] == 401
                assert request(state, "GET", "/health")[1]["ready"]
                untrusted = http.client.HTTPSConnection("127.0.0.1", state["port"], context=ssl.create_default_context(), timeout=5)
                try:
                    untrusted.request("GET", "/riotclient/app-port")
                    raise AssertionError("untrusted test CA was accepted")
                except ssl.SSLCertVerificationError:
                    pass
                finally:
                    untrusted.close()
                assert request(state, "POST", "/dev/skins/grant", {"skin_id": "x" * 5000})[0] == 413
                for binary_mode in (False, True):
                    websocket(state, pki["ca"], binary_mode)
                catalog = json.loads(Path(config["skin_catalog"]).read_text())["skins"]
                skin = catalog[0]
                assert request(state, "POST", "/dev/skins/grant", {"skin_id": skin["skin_id"]})[0] == 200
                assert skin["skin_id"] in json.loads(Path(config["skin_state"]).read_text())["owned"]
                assert request(state, "POST", "/dev/skins/grant-all")[0] == 200
                assert len(json.loads(Path(config["skin_state"]).read_text())["owned"]) == len(catalog)
                gate = Path(config["startup_gate"])
                gate.touch()
                with concurrent.futures.ThreadPoolExecutor() as executor:
                    pending = executor.submit(client.https_request, state["port"], pki["ca"], state["auth_token"], "GET", "/v1/config/na")
                    time.sleep(.15)
                    assert not pending.done()
                    assert request(state, "GET", "/health")[0] == 200
                    gate.unlink()
                    assert pending.result()[0] == 200
                print("PASS: TLS auth, settings, JSON+MessagePack RMS, admin auth, catalog grants, persistence and nonblocking startup gate")
            finally:
                process.terminate()
                process.wait(timeout=10)


if __name__ == "__main__":
    root = Path(__file__).resolve().parents[1]
    run(Path(sys.argv[1]) if len(sys.argv) > 1 else root / "target/debug/aries-platform.exe")
