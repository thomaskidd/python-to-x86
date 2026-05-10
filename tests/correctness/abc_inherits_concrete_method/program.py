from __future__ import annotations
from abc import ABC, abstractmethod

class Counter(ABC):
    count: int
    def __init__(self, start: int):
        self.count = start
    # Concrete inherited method; doesn't call any abstract method, so
    # this works without vtables.
    def doubled_count(self) -> int:
        return self.count * 2
    @abstractmethod
    def step(self) -> int: ...

class Incrementer(Counter):
    def step(self) -> int:
        self.count = self.count + 1
        return self.count

def main(start: int) -> int:
    c: Incrementer = Incrementer(start)
    c.step()
    c.step()
    # Inherited concrete method works on the subclass instance.
    return c.doubled_count()
