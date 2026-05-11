from __future__ import annotations
from abc import ABC, abstractmethod

class Op(ABC):
    @abstractmethod
    def apply(self, x: int) -> int: ...

class Doubler(Op):
    def apply(self, x: int) -> int:
        return x * 2

def main(x: int) -> int:
    op: Op = Doubler()
    return op.apply(x)
