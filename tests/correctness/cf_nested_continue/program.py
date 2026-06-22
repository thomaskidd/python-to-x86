from __future__ import annotations
def main(n: int) -> int:
    total: int = 0
    for i in range(n):
        if i % 4 == 0:
            continue
        for j in range(n):
            if (i + j) % 2 == 0:
                continue
            total = total + i * j
    return total
