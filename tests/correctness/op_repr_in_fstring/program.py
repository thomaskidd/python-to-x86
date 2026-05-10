from __future__ import annotations
class Pair:
    a: int
    b: int
    def __init__(self, a: int, b: int):
        self.a = a
        self.b = b
    def __add__(self, other: Pair) -> Pair:
        return Pair(self.a + other.a, self.b + other.b)
    def __repr__(self) -> str:
        return f"P({self.a}, {self.b})"

def main(x: int, y: int) -> str:
    p: Pair = Pair(x, y)
    q: Pair = Pair(y, x)
    r: Pair = p + q
    return f"sum={r}"
