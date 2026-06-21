from __future__ import annotations
def main(n: int) -> int:
    total: int = 0
    for i, v in enumerate(range(n)):
        if v % 3 == 0:
            continue
        total = total + i
    return total
