from __future__ import annotations
def main(a: int, b: int) -> int:
    xs: list[int] = [a, 0, b, 0, a + b, 0]
    total: int = 0
    for x in xs:
        if x == 0:
            continue
        total = total + x
    return total
