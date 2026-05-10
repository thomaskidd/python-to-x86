from __future__ import annotations
class A:
    a: int
    def __init__(self, a: int):
        self.a = a
    def f(self) -> int:
        return self.a

class B(A):
    b: int
    def __init__(self, a: int, b: int):
        super().__init__(a)
        self.b = b

class C(B):
    c: int
    def __init__(self, a: int, b: int, c: int):
        super().__init__(a, b)
        self.c = c

def main(a: int) -> int:
    c: C = C(a, a + 1, a + 2)
    return c.f() * 1000 + c.a * 100 + c.b * 10 + c.c
