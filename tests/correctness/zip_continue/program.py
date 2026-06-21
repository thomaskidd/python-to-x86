from __future__ import annotations
def main(a: int, b: int) -> int:
    xs: list[int] = [a, 0, b, 0, a + b]
    ys: list[int] = [1, 2, 3, 4, 5]
    total: int = 0
    for x, y in zip(xs, ys):
        if x == 0:
            continue
        total = total + x * y
    return total
