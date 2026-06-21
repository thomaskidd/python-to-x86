from __future__ import annotations
def main(a: int, b: int) -> int:
    xs: list[int] = [a, b, a + b, a - b]
    total: int = 0
    for v in (x * 2 for x in xs if x > 0):
        total = total + v
    return total
