from __future__ import annotations
def main(a: int, b: int) -> int:
    xs: list[int] = [a, b, a, a + b, b]
    s: set[int] = {x for x in xs}
    return len(s)
