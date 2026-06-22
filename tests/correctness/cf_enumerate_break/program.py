from __future__ import annotations
def main(a: int, b: int) -> int:
    xs: list[int] = [a, b, a + b, a - b, a * 2, b * 2]
    total: int = 0
    for i, x in enumerate(xs):
        if x < 0:
            break
        total = total + i + x
    return total
