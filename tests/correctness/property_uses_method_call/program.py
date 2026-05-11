from __future__ import annotations

class Vec:
    x: int
    y: int
    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y
    def sum(self) -> int:
        return self.x + self.y
    @property
    def doubled_sum(self) -> int:
        # Property body calls another (regular) method on self.
        return self.sum() * 2

def main(a: int, b: int) -> int:
    v: Vec = Vec(a, b)
    return v.doubled_sum
