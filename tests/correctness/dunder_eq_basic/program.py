from __future__ import annotations
class Pair:
    a: int
    b: int
    def __init__(self, a: int, b: int):
        self.a = a
        self.b = b
    def __eq__(self, other: Pair) -> bool:
        if self.a == other.a:
            if self.b == other.b:
                return True
        return False

def main(x: int, y: int) -> int:
    p: Pair = Pair(x, y)
    q: Pair = Pair(x, y)
    r: Pair = Pair(x + 1, y)
    result: int = 0
    if p == q:
        result = result + 10
    if p == r:
        result = result + 1
    return result
