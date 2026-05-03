from __future__ import annotations
def main(a: int, b: int) -> int:
    p: list[int] = [a, b]
    q: list[int] = [b, a]
    r: list[int] = p + q
    return r[0] + r[1] * 10 + r[2] * 100 + r[3] * 1000 + len(r)
