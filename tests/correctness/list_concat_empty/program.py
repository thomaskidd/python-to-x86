from __future__ import annotations
def main(n: int) -> int:
    empty: list[int] = []
    full: list[int] = [n, n + 1, n + 2]
    r: list[int] = empty + full + empty
    total: int = 0
    for x in r:
        total = total + x
    return total + len(r)
