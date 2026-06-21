from __future__ import annotations
def main(n: int) -> int:
    total: int = 0
    for v in (i * i for i in range(n)):
        total = total + v
    return total
