from __future__ import annotations
def main(n: int) -> int:
    total: int = 0
    for i in range(n):
        if i % 3 == 0:
            continue
        if i > 20:
            break
        total = total + i
    return total
