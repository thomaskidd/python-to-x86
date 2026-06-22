from __future__ import annotations
def main(n: int) -> int:
    total: int = 0
    for i in range(n):
        j: int = 0
        while j < n:
            for k in range(j):
                if k == i:
                    continue
                total = total + 1
            if j > i:
                break
            j = j + 1
    return total
