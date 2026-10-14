"""End-to-end launcher/platform supervision, without launching or altering a game."""
import ctypes
import http.client
import itertools
import json
import socket
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
        help_result = subprocess.run([str(loader), "--help"], capture_output=True, text=True, timeout=5)
        assert help_result.returncode == 0 and "Aries loader" in help_result.stdout
        modes = ["--check", "--install-firewall", "--server-only"]
        invalid_commands = [[first, second] for first, second in itertools.product(modes, repeat=2)]
        invalid_commands += [["--write-config", str(config_path), mode] for mode in modes]
        invalid_commands += [[mode, "--write-config", str(config_path)] for mode in modes]
        invalid_commands += [
            ["--config"], ["--write-config"], ["--config", ""],
            ["--config", "--server-only"], ["--write-config", "--check"],
            ["--config", str(config_path), "--config", str(config_path)],
            ["--write-config", str(config_path), "--config", str(config_path)],
            ["--config", str(config_path), "--write-config", str(config_path)],
            ["--help", "--server-only"], ["--check", "--help"], ["--unknown"],
        ]
        for arguments in invalid_commands:
            rejected = subprocess.run([str(loader), *arguments], capture_output=True, text=True, timeout=5)
            assert rejected.returncode != 0, f"accepted invalid command: {arguments}"
            assert not config_path.exists(), f"invalid command wrote settings: {arguments}"
        subprocess.run([str(loader), "--write-config", str(config_path)], check=True)
        original_config = config_path.read_bytes()
        overwrite = subprocess.run([str(loader), "--write-config", str(config_path)], capture_output=True, text=True, timeout=5)
        assert overwrite.returncode != 0 and config_path.read_bytes() == original_config
        config = json.loads(config_path.read_text())
        config.update({"port": "0", "admin_port": "0", "server_path": str(server), "cert_path": pki["cert"], "key_path": pki["key"], "skin_catalog": str(root.parent / "local_backend/skin_catalog.json"), "skin_state": "account/skins.json", "game_settings_path": "account/persistence", "runtime_path": "runs"})
        config_path.write_text(json.dumps(config))
        command = [str(loader), "--config", str(config_path), "--server-only"]
        collision_config = dict(config, port="58089", admin_port="58089")
        config_path.write_text(json.dumps(collision_config))
        collision = subprocess.run(command, capture_output=True, text=True, timeout=5)
        assert collision.returncode != 0 and "ports must differ" in collision.stderr
        assert not (directory / "runs").exists(), "port preflight started a launch"
        config_path.write_text(json.dumps(config))
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
        # A port owned by an unrelated listener must fail without disturbing it.
        with socket.socket() as occupied:
            occupied.bind(("127.0.0.1", 0))
            occupied.listen()
            blocked_port = occupied.getsockname()[1]
            config_path.write_text(json.dumps(dict(config, port=str(blocked_port), runtime_path="occupied-runs")))
            blocked = subprocess.run(command, capture_output=True, text=True, timeout=20)
            assert blocked.returncode != 0 and "platform readiness" in blocked.stderr
            assert not list((directory / "occupied-runs").glob("*/backend_state.json"))
            with socket.create_connection(("127.0.0.1", blocked_port), timeout=2):
                accepted, _ = occupied.accept()
                accepted.close()
        # Supervision must notice the platform dying after readiness, not idle forever.
        config_path.write_text(json.dumps(dict(config, runtime_path="crash-runs")))
        process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        try:
            deadline = time.monotonic() + 15
            crash_state = None
            while time.monotonic() < deadline:
                paths = list((directory / "crash-runs").glob("*/backend_state.json"))
                if paths:
                    crash_state = json.loads(paths[0].read_text())
                    break
                if process.poll() is not None:
                    raise AssertionError(process.stdout.read())
                time.sleep(.05)
            assert crash_state and alive(crash_state["pid"]), "crash-test platform was not started"
            kernel = ctypes.WinDLL("kernel32", use_last_error=True)
            kernel.OpenProcess.argtypes = [ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
            kernel.OpenProcess.restype = ctypes.c_void_p
            kernel.TerminateProcess.argtypes = [ctypes.c_void_p, ctypes.c_uint]
            kernel.CloseHandle.argtypes = [ctypes.c_void_p]
            child_handle = kernel.OpenProcess(1, False, crash_state["pid"])
            assert child_handle, "cannot open owned test platform"
            try:
                assert kernel.TerminateProcess(child_handle, 23)
            finally:
                kernel.CloseHandle(child_handle)
            output, _ = process.communicate(timeout=10)
            assert process.returncode != 0 and ("platform exited unexpectedly" in output or "child exited" in output), output
        finally:
            if process.poll() is None:
                process.terminate()
                process.communicate(timeout=10)
        config["server_path"] = str(directory / "missing.exe")
        config_path.write_text(json.dumps(config))
        failed = subprocess.run(command, capture_output=True, text=True, timeout=5)
        assert failed.returncode != 0 and "missing platform executable" in failed.stderr
        print("PASS: CLI validation, non-overwrite, port preflight, launcher readiness/auth, ephemeral loopback ports, duplicate refusal, occupied-port isolation, child failure, crash cleanup and missing-artifact preflight")


if __name__ == "__main__":
    run()
