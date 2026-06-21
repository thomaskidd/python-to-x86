from __future__ import annotations
def main(n: int) -> int:
    total: int = 0
    for i in range(n):
        if i % 2 == 0:
            continue
        total = total + i
    return total
