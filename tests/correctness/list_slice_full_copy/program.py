from __future__ import annotations
def main(n: int) -> int:
    xs: list[int] = [1, 2, 3]
    ys: list[int] = xs[:]
    # ys is a fresh copy; mutating ys must not affect xs.
    ys[0] = n
    return xs[0] * 100 + ys[0]
