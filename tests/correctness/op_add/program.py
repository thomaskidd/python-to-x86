from __future__ import annotations
class Vec:
    x: int
    y: int
    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y
    def __add__(self, other: Vec) -> Vec:
        return Vec(self.x + other.x, self.y + other.y)

def main(a: int, b: int) -> int:
    p: Vec = Vec(a, b)
    q: Vec = Vec(b, a)
    r: Vec = p + q
    return r.x * 1000 + r.y
