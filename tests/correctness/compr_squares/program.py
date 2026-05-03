from __future__ import annotations
def main(n: int) -> int:
    sq: list[int] = [i * i for i in range(n)]
    total: int = 0
    for x in sq:
        total = total + x
    return total
