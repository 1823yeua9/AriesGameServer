"""Write golden fixtures from the python responder for the rust tests.

Runs the original python builders offline (no sockets, no live process) and
dumps the results into server/fixtures/<section>.json. Sections live in
tools/golden/<area>.py, each exporting SECTIONS = {name: fn}; fn returns any
json value. Add a new module per area instead of editing this file.

usage: python export_golden.py [section ...]   (default: every section)
env ARIES_PY_BACKEND overrides the python source dir.
"""

import importlib
import json
import os
import sys
from pathlib import Path

here = Path(__file__).resolve().parent
fixtures = here.parent / "fixtures"
# the python project root (holds local_backend/ and range_authority_probe/). worktrees
# live elsewhere, so fall back to the fixed project path when the relative one is missing.
project = Path(os.environ.get("ARIES_PROJECT_ROOT", here.parents[2]))
if not (project / "local_backend").is_dir():
    project = Path(r"C:/Users/Cayden/Desktop/valorant proj")
backend = Path(os.environ.get("ARIES_PY_BACKEND", project / "local_backend"))
os.environ["ARIES_PROJECT_ROOT"] = str(project)
sys.path.insert(0, str(backend))
sys.path.insert(0, str(here))
sys.dont_write_bytecode = True


def all_sections():
    sections = {}
    for path in sorted((here / "golden").glob("*.py")):
        if path.stem.startswith("_"):
            continue
        module = importlib.import_module(f"golden.{path.stem}")
        for name, fn in module.SECTIONS.items():
            if name in sections:
                raise ValueError(f"duplicate golden section {name}")
            sections[name] = fn
    return sections


def main(names):
    sections = all_sections()
    fixtures.mkdir(parents=True, exist_ok=True)
    for name in names or sections:
        data = sections[name]()
        path = fixtures / f"{name}.json"
        path.write_text(json.dumps(data, indent=1) + "\n", encoding="utf-8")
        print(f"{name}: {len(data)} cases -> {path}")


if __name__ == "__main__":
    main(sys.argv[1:])
