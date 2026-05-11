from __future__ import annotations
def main(j: int) -> int:
    xs: list[int] = [1, 2, 3, 4, 5]
    ys: list[int] = xs[:j]
    total: int = 0
    k: int = 0
    while k < len(ys):
        total = total + ys[k]
        k = k + 1
    return total
