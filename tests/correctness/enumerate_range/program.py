from __future__ import annotations
def main(n: int) -> int:
    total: int = 0
    for i, v in enumerate(range(0, n, 2)):
        total = total + i * 100 + v
    return total
