from __future__ import annotations
class Base:
    a: int
    b: int
    def __init__(self, a: int, b: int):
        self.a = a
        self.b = b

class Derived(Base):
    c: int
    def __init__(self, a: int, b: int, c: int):
        super().__init__(a, b)
        self.c = c

def main(a: int, b: int) -> int:
    d: Derived = Derived(a, b, a + b)
    return d.a * 10000 + d.b * 100 + d.c
