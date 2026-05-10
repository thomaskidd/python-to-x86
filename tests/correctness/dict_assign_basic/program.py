from __future__ import annotations
def main(a: int, b: int) -> int:
    d: dict[int, int] = {1: 0, 2: 0, 3: 0}
    d[1] = a
    d[2] = b
    d[3] = a + b
    return d[1] * 100 + d[2] * 10 + d[3]
