from __future__ import annotations
def main(a: int, b: int) -> int:
    d: dict[int, int] = {1: a, 2: b, 3: a + b}
    return d[1] * 100 + d[2] * 10 + d[3] + len(d)
