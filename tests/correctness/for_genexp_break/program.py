from __future__ import annotations
def main(n: int) -> int:
    total: int = 0
    for v in (i * 3 for i in range(n)):
        if v > 30:
            break
        total = total + v
    return total
