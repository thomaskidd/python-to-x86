from __future__ import annotations
def swap(p: tuple[int, int]) -> tuple[int, int]:
    return (p[1], p[0])

def main(a: int, b: int) -> int:
    t: tuple[int, int] = swap((a, b))
    return t[0] * 1000 + t[1]
