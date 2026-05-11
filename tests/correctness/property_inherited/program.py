from __future__ import annotations

class Base:
    n: int
    def __init__(self, n: int):
        self.n = n
    @property
    def doubled(self) -> int:
        return self.n * 2

class Sub(Base):
    pass

def main(n: int) -> int:
    s: Sub = Sub(n)
    return s.doubled
