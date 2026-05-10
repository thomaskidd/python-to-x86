from __future__ import annotations
def main(k: int) -> int:
    s: set[int] = {1, 2, 3}
    s.add(k)
    s.add(k)
    s.add(k)
    s.add(k)
    return len(s)
