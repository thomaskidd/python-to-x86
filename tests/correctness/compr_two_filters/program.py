from __future__ import annotations
def main(n: int) -> int:
    xs: list[int] = [i for i in range(n) if i % 2 == 0 if i % 3 == 0]
    total: int = 0
    for x in xs:
        total = total + x
    return total
