from __future__ import annotations
from abc import ABC, abstractmethod

class Shape(ABC):
    name_index: int
    def __init__(self, idx: int):
        self.name_index = idx
    @abstractmethod
    def area(self) -> int: ...
    # Concrete method on an abstract base. NOT in the vtable; concrete
    # subclass calls dispatch to this directly (static).
    def describe_index(self) -> int:
        return self.name_index * 100

class Square(Shape):
    side: int
    def __init__(self, idx: int, side: int):
        super().__init__(idx)
        self.side = side
    def area(self) -> int:
        return self.side * self.side

def main(idx: int, side: int) -> int:
    sq: Square = Square(idx, side)
    # describe_index dispatches statically (it's not abstract anywhere).
    # area dispatches virtually because it's in Shape's vtable.
    return sq.area() * 10000 + sq.describe_index()
