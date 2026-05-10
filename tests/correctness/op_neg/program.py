from __future__ import annotations
class Vec:
    x: int
    y: int
    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y
    def __neg__(self) -> Vec:
        return Vec(-self.x, -self.y)

def main(a: int, b: int) -> int:
    p: Vec = Vec(a, b)
    q: Vec = -p
    return q.x * 1000 + q.y
