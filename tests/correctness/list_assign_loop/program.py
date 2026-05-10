from __future__ import annotations
def main(n: int) -> int:
    xs: list[int] = []
    i: int = 0
    while i < n:
        xs.append(0)
        i = i + 1
    j: int = 0
    while j < n:
        xs[j] = j * j
        j = j + 1
    # Sum the squares back.
    total: int = 0
    k: int = 0
    while k < n:
        total = total + xs[k]
        k = k + 1
    return total
