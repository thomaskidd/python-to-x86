from __future__ import annotations
def main(a: int) -> int:
    xs: list[int] = [0]
    xs[0] = a
    xs[0] = a + 1
    xs[0] = a * 2
    xs[0] = a - 7
    return xs[0]
