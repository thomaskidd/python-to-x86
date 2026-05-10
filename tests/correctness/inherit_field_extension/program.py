from __future__ import annotations
class A:
    x: int
    def __init__(self, x: int):
        self.x = x

class B(A):
    y: int
    def __init__(self, x: int, y: int):
        super().__init__(x)
        self.y = y

def main(a: int, b: int) -> int:
    o: B = B(a, b)
    return o.x * 1000 + o.y
