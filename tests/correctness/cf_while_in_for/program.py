from __future__ import annotations
def main(n: int) -> int:
    total: int = 0
    for i in range(n):
        j: int = 0
        while j < n:
            if j > i:
                break
            total = total + 1
            j = j + 1
        if i % 2 == 0:
            continue
        total = total + 100
    return total
