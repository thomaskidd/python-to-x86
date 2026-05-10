from __future__ import annotations
def main(a: int, b: int) -> int:
    d: dict[int, int] = {7: 0}
    d[7] = a
    d[7] = a + 1
    d[7] = b
    d[7] = b * 2
    return d[7]
