from __future__ import annotations
def main(a: int, b: int) -> int:
    xs: list[int] = [a, b, a + b, 7, 9, 11]
    ys: list[int] = [1, 2, 3]
    total: int = 0
    for x, y in zip(xs, ys):
        total = total + x + y
    return total
