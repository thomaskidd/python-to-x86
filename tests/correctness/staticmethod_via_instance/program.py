from __future__ import annotations

class M:
    n: int
    def __init__(self, n: int):
        self.n = n
    @staticmethod
    def square(x: int) -> int:
        return x * x

def main(n: int) -> int:
    m: M = M(n)
    # Call staticmethod via instance — same result as M.square(n).
    return m.square(n)
