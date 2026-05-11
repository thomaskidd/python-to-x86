from __future__ import annotations
def main(i: int, j: int) -> int:
    xs: list[int] = [10, 20, 30, 40, 50, 60, 70]
    ys: list[int] = xs[i:j]
    total: int = 0
    k: int = 0
    while k < len(ys):
        total = total + ys[k]
        k = k + 1
    return total * 100 + len(ys)
