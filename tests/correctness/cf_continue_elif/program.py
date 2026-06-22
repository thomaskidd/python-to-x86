from __future__ import annotations
def main(n: int) -> int:
    total: int = 0
    for i in range(n):
        if i % 3 == 0:
            total = total + 1
        elif i % 3 == 1:
            continue
        else:
            total = total + 10
        total = total + i
    return total
