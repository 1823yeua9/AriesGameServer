"""End-to-end launcher/platform supervision, without launching or altering a game."""
import ctypes
import http.client
import json
import subprocess
import sys
import tempfile
import time
from pathlib import Path

root = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(root.parent / "local_backend"))
from test_backend import make_pki


def alive(pid):
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenProcess.argtypes = [ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
    kernel.OpenProcess.restype = ctypes.c_void_p
    kernel.CloseHandle.argtypes = [ctypes.c_void_p]
    kernel.WaitForSingleObject.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
    handle = kernel.OpenProcess(0x100000, False, pid)
    if not handle:
        return False
    try:
        return kernel.WaitForSingleObject(handle, 0) == 258
    finally:
        kernel.CloseHandle(handle)


def run():
    loader = root / "loader/x64/Release/loader.exe"
    server = root / "server/platform/target/debug/aries-platform.exe"
    with tempfile.TemporaryDirectory(prefix="aries-loader-test-") as temporary:
        directory = Path(temporary)
        pki = make_pki(str(directory))
        config_path = directory / "settings.val"
        subprocess.run([str(loader), "--write-config", str(config_path)], check=True)
        config = json.loads(config_path.read_text())
        config.update({"port": "0", "admin_port": "0", "server_path": str(server), "cert_path": pki["cert"], "key_path": pki["key"], "skin_catalog": str(root.parent / "local_backend/skin_catalog.json"), "skin_state": "account/skins.json", "game_settings_path": "account/persistence", "runtime_path": "runs"})
        config_path.write_text(json.dumps(config))
        command = [str(loader), "--config", str(config_path), "--server-only"]
        process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        state = None
        try:
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                paths = list((directory / "runs").glob("*/backend_state.json"))
                if paths:
                    state = json.loads(paths[0].read_text())
                    break
                if process.poll() is not None:
                    raise AssertionError(process.stdout.read())
                time.sleep(.05)
            assert state and alive(state["pid"]), "platform was not started"
            connection = http.client.HTTPConnection("127.0.0.1", state["admin_port"], timeout=5)
            connection.request("GET", "/health", headers={"Authorization": "Bearer " + state["admin_token"]})
            response = connection.getresponse()
            assert response.status == 200 and json.loads(response.read())["pid"] == state["pid"]
            connection.close()
            duplicate = subprocess.run(command, capture_output=True, text=True, timeout=5)
            assert duplicate.returncode != 0 and "another Aries loader" in duplicate.stdout + duplicate.stderr
            assert alive(state["pid"]), "duplicate launch damaged existing child"
        finally:
            process.terminate()
            process.communicate(timeout=10)
            if state:
                deadline = time.monotonic() + 5
                while alive(state["pid"]) and time.monotonic() < deadline:
                    time.sleep(.05)
                assert not alive(state["pid"]), "orphaned platform child after loader exit"
        config["server_path"] = str(directory / "missing.exe")
        config_path.write_text(json.dumps(config))
        failed = subprocess.run(command, capture_output=True, text=True, timeout=5)
        assert failed.returncode != 0 and "missing platform executable" in failed.stderr
        print("PASS: launcher readiness/auth, ephemeral loopback ports, duplicate refusal, crash cleanup and missing-artifact preflight")


if __name__ == "__main__":
    run()
