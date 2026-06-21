from __future__ import annotations
def main(a: int, b: int) -> int:
    xs: list[int] = [a, b, a + b]
    ys: list[int] = [b, a, a - b]
    total: int = 0
    for x, y in zip(xs, ys):
        total = total + x * y
    return total
