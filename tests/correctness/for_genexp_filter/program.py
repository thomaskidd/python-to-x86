from __future__ import annotations
def main(n: int) -> int:
    total: int = 0
    for v in (i + 1 for i in range(n) if i % 3 == 0):
        total = total + v
    return total
