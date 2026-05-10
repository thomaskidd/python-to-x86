from __future__ import annotations
def main(n: int) -> int:
    s: set[int] = set()
    i: int = 0
    while i < n:
        s.add(i)
        i = i + 1
    # Re-add the same values — len should not change.
    j: int = 0
    while j < n:
        s.add(j)
        j = j + 1
    return len(s)
