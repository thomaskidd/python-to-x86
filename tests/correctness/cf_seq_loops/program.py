from __future__ import annotations
def main(n: int) -> int:
    total: int = 0
    for i in range(n):
        total = total + i
    for i in range(n):
        if i % 2 == 0:
            continue
        total = total + i * 2
    return total
