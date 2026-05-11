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

class Circle(Shape):
    r: int
    def __init__(self, r: int):
        self.r = r
    def area(self) -> int:
        return self.r * self.r * 3

def main(n: int) -> int:
    shapes: list[Shape] = []
    i: int = 0
    while i < n:
        # Alternate between Square and Circle.
        if i % 2 == 0:
            shapes.append(Square(i + 1))
        else:
            shapes.append(Circle(i + 1))
        i = i + 1
    total: int = 0
    j: int = 0
    while j < n:
        total = total + shapes[j].area()
        j = j + 1
    return total
