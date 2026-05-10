from __future__ import annotations
class Cmp:
    v: int
    def __init__(self, v: int):
        self.v = v
    def __le__(self, other: Cmp) -> bool:
        return self.v <= other.v
    def __gt__(self, other: Cmp) -> bool:
        return self.v > other.v
    def __ge__(self, other: Cmp) -> bool:
        return self.v >= other.v

def main(a: int, b: int) -> int:
    p: Cmp = Cmp(a)
    q: Cmp = Cmp(b)
    out: int = 0
    if p <= q:
        out = out + 100
    if p > q:
        out = out + 10
    if p >= q:
        out = out + 1
    return out
