from __future__ import annotations

class Box:
    width: int
    height: int
    def __init__(self, w: int, h: int):
        self.width = w
        self.height = h
    @property
    def area(self) -> int:
        return self.width * self.height

def main(w: int, h: int) -> int:
    b: Box = Box(w, h)
    return b.area
