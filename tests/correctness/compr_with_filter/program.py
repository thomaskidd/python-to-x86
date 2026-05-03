from __future__ import annotations
def main(n: int) -> int:
    odds: list[int] = [i for i in range(n) if i % 2 == 1]
    total: int = 0
    for x in odds:
        total = total + x
    return total
