from __future__ import annotations
def main(a: int, b: int) -> int:
    lst: list[int] = [a, b, a + b, a * b]
    return lst[0] + lst[1] + lst[2] + lst[3] + len(lst)
