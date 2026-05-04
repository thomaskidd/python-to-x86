from __future__ import annotations
def main(a: int, b: int) -> int:
    lst: list[int] = [a]
    lst.append(b)
    lst.append(a + b)
    total: int = 0
    for x in lst:
        total = total + x
    return total
