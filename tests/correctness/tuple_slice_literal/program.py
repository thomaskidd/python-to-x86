from __future__ import annotations
def main(a: int, b: int) -> int:
    t: tuple[int, int, int, int] = (a, b, a + b, a - b)
    # Slice with literal bounds: result is a (int, int).
    pair: tuple[int, int] = t[1:3]
    return pair[0] * 100 + pair[1]
