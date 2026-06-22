from __future__ import annotations
def main(n: int) -> int:
    total: int = 0
    for i in range(n):
        inner: list[int] = [j * j for j in range(i) if j % 2 == 0]
        for x in inner:
            total = total + x
    return total
