from __future__ import annotations
class Norm:
    v: int
    def __init__(self, v: int):
        self.v = v
    def __lt__(self, other: Norm) -> bool:
        return self.v < other.v

def main(a: int, b: int) -> int:
    p: Norm = Norm(a)
    q: Norm = Norm(b)
    if p < q:
        return 1
    return 0
