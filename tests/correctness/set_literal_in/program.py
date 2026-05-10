from __future__ import annotations
def main(n: int) -> int:
    s: set[int] = {1, 2, 3, 5, 8, 13}
    found: int = 0
    if n in s:
        found = 1
    return found
