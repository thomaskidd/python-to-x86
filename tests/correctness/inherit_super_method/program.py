from __future__ import annotations
class A:
    n: int
    def __init__(self, n: int):
        self.n = n
    def compute(self) -> int:
        return self.n * 2

class B(A):
    def compute(self) -> int:
        # Call parent's compute, then add the original n.
        return super().compute() + self.n

def main(n: int) -> int:
    b: B = B(n)
    return b.compute()
