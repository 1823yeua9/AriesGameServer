"""Compare the Rust router against exported Python tests, including JWT claims."""
import base64
import json
import subprocess
import sys
from pathlib import Path


def normalize(value):
    if isinstance(value, dict):
        return {key: (0 if key in {"timestamp", "time"} else normalize(item)) for key, item in value.items()}
    if isinstance(value, list):
        return [normalize(item) for item in value]
    if isinstance(value, str):
        if value.startswith("eyJ") and value.count(".") == 2:
            return {"jwt_claims": json.loads(base64.urlsafe_b64decode(value.split(".")[1] + "=="))}
        if value.startswith("{"):
            try:
                return normalize(json.loads(value))
            except json.JSONDecodeError:
                pass
    return value


if __name__ == "__main__":
    root = Path(__file__).resolve().parents[1]
    binary = Path(sys.argv[1]) if len(sys.argv)>1 else root / "target/debug/aries-platform.exe"
    cases = json.loads((root / "data/parity.json").read_text())
    process = subprocess.run([str(binary), "--replay"], input="".join(json.dumps(case)+"\n" for case in cases), text=True, capture_output=True, check=True)
    results = [json.loads(line) for line in process.stdout.splitlines()]
    if len(results)!=len(cases):
        raise SystemExit(f"Expected {len(cases)} replies, got {len(results)}")
    failed = 0
    for case, result in zip(cases, results):
        expected={"status":case["status"],"payload":case["payload"]}
        if normalize(expected)!=normalize(result):
            failed += 1
            print(case["method"],case["path"])
            print("expected",json.dumps(normalize(expected))[:2000])
            print("actual  ",json.dumps(normalize(result))[:2000])
    print(f"{len(cases)-failed}/{len(cases)} reference contracts matched")
    raise SystemExit(bool(failed))
