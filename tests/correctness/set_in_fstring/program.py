from __future__ import annotations
def main(n: int) -> str:
    s: set[int] = set()
    i: int = 0
    while i < n:
        s.add(i * 7)
        i = i + 1
    return f"size={len(s)}"
