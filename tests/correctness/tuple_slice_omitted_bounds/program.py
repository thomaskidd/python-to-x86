from __future__ import annotations
def main(a: int, b: int) -> int:
    t: tuple[int, int] = (a, b)
    # t[:] is just a copy.
    u: tuple[int, int] = t[:]
    return u[0] * 100 + u[1]
