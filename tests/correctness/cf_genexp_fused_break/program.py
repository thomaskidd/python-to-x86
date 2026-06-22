from __future__ import annotations
def main(n: int) -> int:
    total: int = 0
    for v in (i * i for i in range(n) if i % 2 == 1):
        if v > 40:
            break
        total = total + v
    return total
