from __future__ import annotations
def main(n: int) -> int:
    xs: list[int] = [0, 0, 0]
    xs[0] = n
    xs[1] = n * 2
    xs[2] = xs[0] + xs[1]
    return xs[0] * 100 + xs[1] * 10 + xs[2]
