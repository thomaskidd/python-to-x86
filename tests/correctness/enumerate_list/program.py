from __future__ import annotations
def main(a: int, b: int) -> int:
    xs: list[int] = [a, b, a + b, a * 2]
    total: int = 0
    for i, x in enumerate(xs):
        total = total + i * x
    return total
