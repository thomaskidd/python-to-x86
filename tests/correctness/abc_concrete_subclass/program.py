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

def main(s: int) -> int:
    sq: Square = Square(s)
    return sq.area()
