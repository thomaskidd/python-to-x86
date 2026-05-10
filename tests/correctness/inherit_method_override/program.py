from __future__ import annotations
class A:
    n: int
    def __init__(self, n: int):
        self.n = n
    def value(self) -> int:
        return self.n

class B(A):
    def value(self) -> int:
        return self.n * 10

def main(n: int) -> int:
    a: A = A(n)
    b: B = B(n)
    return a.value() * 100 + b.value()
