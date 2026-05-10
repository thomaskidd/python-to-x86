from __future__ import annotations
from abc import ABC, abstractmethod

class Counter(ABC):
    count: int
    def __init__(self, start: int):
        self.count = start
    @abstractmethod
    def step(self) -> int: ...

class Doubling(Counter):
    def step(self) -> int:
        self.count = self.count * 2
        return self.count

def main(start: int, iters: int) -> int:
    d: Doubling = Doubling(start)
    i: int = 0
    while i < iters:
        d.step()
        i = i + 1
    return d.count
