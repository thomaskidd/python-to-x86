from __future__ import annotations
def main(n: int) -> int:
    sq: set[int] = {i * i for i in range(n)}
    count: int = 0
    for i in range(n):
        if i * i in sq:
            count = count + 1
    return count + len(sq)
