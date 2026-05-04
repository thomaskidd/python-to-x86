from __future__ import annotations
class Point:
    x: int
    y: int

    def __init__(self, x: int, y: int):
        self.x = x
        self.y = y

    def manhattan(self) -> int:
        ax: int = self.x
        ay: int = self.y
        if ax < 0:
            ax = -ax
        if ay < 0:
            ay = -ay
        return ax + ay

def main(x: int, y: int) -> int:
    p: Point = Point(x, y)
    return p.manhattan()
