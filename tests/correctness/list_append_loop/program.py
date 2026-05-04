from __future__ import annotations
def main(n: int) -> int:
    lst: list[int] = []
    i: int = 0
    while i < n:
        lst.append(i * i)
        i = i + 1
    total: int = 0
    for x in lst:
        total = total + x
    return total
