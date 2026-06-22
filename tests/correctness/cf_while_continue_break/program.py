from __future__ import annotations
def main(n: int) -> int:
    total: int = 0
    i: int = 0
    while i < n:
        i = i + 1
        if i % 2 == 0:
            continue
        if i > 25:
            break
        total = total + i
    return total
