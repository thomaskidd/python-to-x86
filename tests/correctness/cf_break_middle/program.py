from __future__ import annotations
def main(n: int) -> int:
    total: int = 0
    for i in range(n):
        for j in range(n):
            if j == i:
                break
            for k in range(n):
                if k > j:
                    break
                total = total + 1
    return total
