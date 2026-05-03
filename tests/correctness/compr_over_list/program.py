from __future__ import annotations
def main(a: int, b: int, c: int) -> int:
    src: list[int] = [a, b, c, a + b, b + c, a + c]
    doubled: list[int] = [x * 2 for x in src if x > 0]
    total: int = 0
    for y in doubled:
        total = total + y
    return total + len(doubled) * 10000
