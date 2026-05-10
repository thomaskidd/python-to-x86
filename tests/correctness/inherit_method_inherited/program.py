from __future__ import annotations
class A:
    n: int
    def __init__(self, n: int):
        self.n = n
    def doubled(self) -> int:
        return self.n * 2

class B(A):
    pass

def main(n: int) -> int:
    b: B = B(n)
    return b.doubled()
