from __future__ import annotations
def main(n: int) -> int:
    total: int = 0
    for i, v in enumerate(range(n), 100):
        total = total + i + v
    return total
