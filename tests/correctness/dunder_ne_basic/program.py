from __future__ import annotations
class N:
    v: int
    def __init__(self, v: int):
        self.v = v
    def __eq__(self, other: N) -> bool:
        return self.v == other.v

def main(a: int, b: int) -> int:
    x: N = N(a)
    y: N = N(b)
    if x != y:
        return 1
    return 0
