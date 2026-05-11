from __future__ import annotations
from abc import ABC, abstractmethod

class Shape(ABC):
    @abstractmethod
    def area(self) -> int: ...

class Square(Shape):
    side: int
    def __init__(self, side: int):
        self.side = side
    def area(self) -> int:
        return self.side * self.side

def area_of(s: Shape) -> int:
    return s.area()

def main(side: int) -> int:
    sq: Square = Square(side)
    return area_of(sq)
