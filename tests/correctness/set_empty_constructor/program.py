from __future__ import annotations
def main(k: int) -> int:
    s: set[int] = set()
    # length zero; arbitrary `in` check should be False.
    miss: int = 0
    if k in s:
        miss = 1
    return len(s) * 100 + miss
