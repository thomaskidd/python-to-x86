from __future__ import annotations

class Point:
    x: int
    y: int
    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y
    @staticmethod
    def origin() -> Point:
        return Point(0, 0)
    @staticmethod
    def at_offset(off: int) -> Point:
        return Point(off, off)

def main(off: int) -> int:
    o: Point = Point.origin()
    p: Point = Point.at_offset(off)
    return o.x * 10000 + o.y * 1000 + p.x * 10 + p.y
