from __future__ import annotations
def main(a: int, b: int, c: int) -> int:
    lst: list[int] = [a, b, c, a * 2, b * 2, c * 2]
    total: int = 0
    for x in lst:
        total = total + x
    return total
