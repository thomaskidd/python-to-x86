from __future__ import annotations
def main(a: int, b: int) -> int:
    xs: list[int] = [0, a, 0, b]
    return int(any(xs)) * 10 + int(all(xs))
