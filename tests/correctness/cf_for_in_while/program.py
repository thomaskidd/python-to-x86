from __future__ import annotations
def main(n: int) -> int:
    total: int = 0
    i: int = 0
    while i < n:
        for j in range(i):
            if j % 3 == 0:
                continue
            total = total + j
        i = i + 1
    return total
