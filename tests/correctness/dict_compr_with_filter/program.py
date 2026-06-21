from __future__ import annotations
def main(n: int) -> int:
    evens: dict[int, int] = {i: i * 10 for i in range(n) if i % 2 == 0}
    total: int = 0
    for i in range(n):
        if i % 2 == 0:
            total = total + evens[i]
    return total + len(evens)
