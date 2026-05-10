from __future__ import annotations
from abc import ABC, abstractmethod

class Op(ABC):
    @abstractmethod
    def apply(self, x: int) -> int: ...
    @abstractmethod
    def name_value(self) -> int: ...

class Doubler(Op):
    def apply(self, x: int) -> int:
        return x * 2
    def name_value(self) -> int:
        return 7

def main(n: int) -> int:
    d: Doubler = Doubler()
    return d.apply(n) + d.name_value()
