from __future__ import annotations
from abc import ABC, abstractmethod

class A(ABC):
    @abstractmethod
    def f(self) -> int: ...

class B(A):
    # B doesn't implement f → B is still abstract.
    @abstractmethod
    def g(self) -> int: ...

class C(B):
    n: int
    def __init__(self, n: int):
        self.n = n
    def f(self) -> int:
        return self.n + 1
    def g(self) -> int:
        return self.n * 2

def main(n: int) -> int:
    c: C = C(n)
    return c.f() * 10000 + c.g()
