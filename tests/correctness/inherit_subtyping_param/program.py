from __future__ import annotations
class A:
    n: int
    def __init__(self, n: int):
        self.n = n
    def value(self) -> int:
        return self.n

class B(A):
    extra: int
    def __init__(self, n: int, extra: int):
        super().__init__(n)
        self.extra = extra
    # Note: B does NOT override value(). When passed as A, the static
    # dispatch to A.value still returns self.n (B's fields are layout-
    # compatible).

def use_as_a(a: A) -> int:
    return a.value()

def main(n: int, e: int) -> int:
    b: B = B(n, e)
    return use_as_a(b)
