from __future__ import annotations
from abc import ABC, abstractmethod

class A(ABC):
    @abstractmethod
    def f(self) -> int: ...

class B(A):
    n: int
    def __init__(self, n: int):
        self.n = n
    def f(self) -> int:
        return self.n + 1

class C(B):
    # C inherits B's concrete f via the chain. Calling f through an
    # A-typed reference dispatches via vtable to B.f.
    def __init__(self, n: int):
        super().__init__(n * 10)

def call_f(a: A) -> int:
    return a.f()

def main(n: int) -> int:
    c: C = C(n)
    return call_f(c)
