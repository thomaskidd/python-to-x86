from __future__ import annotations
def main(a: int, b: int) -> int:
    xs: list[int] = [a, b, 0, a + b, 0, a - b, b - a]
    total: int = 0
    for x in xs:
        if x == 0:
            continue
        if x < -20:
            break
        total = total + x
    return total
