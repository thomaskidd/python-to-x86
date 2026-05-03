from __future__ import annotations
def main(n: int) -> int:
    # Build a list by repeated concat — emulates append.
    acc: list[int] = []
    i: int = 0
    while i < n:
        acc = acc + [i * i]
        i = i + 1
    total: int = 0
    for x in acc:
        total = total + x
    return total
