from __future__ import annotations
def main(a: int, b: int) -> int:
    t: tuple[int, int, int] = (a, b, a + b)
    return t[0] * 100 + t[1] * 10 + t[2]
