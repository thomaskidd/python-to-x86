from __future__ import annotations
def main(n: int) -> int:
    d: dict[int, int] = {i: i for i in range(0)}
    return len(d) + n
