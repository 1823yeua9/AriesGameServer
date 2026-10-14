"""oracle sections that need no live process: the GObjects pointer decode."""

import random
import sys
from pathlib import Path


def section_oracle_decode():
    sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "range_authority_probe"))
    from read_world_state import decode_array_pointer

    rng = random.Random(9)
    cases = []
    for _ in range(400):
        owner = bytes(rng.randrange(256) for _ in range(0x80))
        cases.append({"owner": owner.hex(), "decoded": str(decode_array_pointer(owner))})
    return cases


SECTIONS = {"oracle_decode": section_oracle_decode}
