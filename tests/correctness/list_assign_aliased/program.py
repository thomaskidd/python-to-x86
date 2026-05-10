from __future__ import annotations
def main(a: int, b: int) -> int:
    xs: list[int] = [10, 20, 30]
    ys: list[int] = xs
    xs[0] = a
    ys[1] = b
    # ys aliases xs — both writes are visible in both names.
    return xs[0] * 1000 + xs[1] * 10 + ys[0] + ys[2]
