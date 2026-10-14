"""cpython random.Random draws for the rust port (bots use Random(1306))."""

import random


def section_pyrandom():
    out = []
    for seed in (1306, 0, 1, 2**32 - 1, 2**32, 123456789012345):
        r = random.Random(seed)
        out.append({"seed": seed, "u32": [r.getrandbits(32) for _ in range(5)],
                    "uniform": [random.Random(seed).uniform(-30.0, 30.0)] + [r.uniform(700.0, 1800.0) for _ in range(700)]})
    return out


SECTIONS = {"pyrandom": section_pyrandom}
