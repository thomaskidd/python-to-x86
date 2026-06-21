from __future__ import annotations
def main(a: int, b: int) -> int:
    xs: list[int] = [a, b, a + b, a - b]
    d: dict[int, int] = {x: x + 1 for x in xs}
    total: int = 0
    for x in xs:
        total = total + d[x]
    return total + len(d)
