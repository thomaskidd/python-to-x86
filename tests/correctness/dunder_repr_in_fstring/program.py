from __future__ import annotations
class Point:
    x: int
    y: int
    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y
    def __repr__(self) -> str:
        return f"({self.x}, {self.y})"

def main(a: int, b: int) -> str:
    p: Point = Point(a, b)
    return f"p={p}"
