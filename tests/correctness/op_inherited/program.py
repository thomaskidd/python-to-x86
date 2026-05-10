from __future__ import annotations
class Base:
    v: int
    def __init__(self, v: int):
        self.v = v
    def __add__(self, other: Base) -> Base:
        return Base(self.v + other.v)

class Sub(Base):
    pass

def main(a: int, b: int) -> int:
    # Sub inherits __add__ from Base. Result is Base-typed (per Base.__add__'s
    # return annotation), which is fine since Sub adds no fields and is layout-
    # compatible.
    p: Sub = Sub(a)
    q: Sub = Sub(b)
    r: Base = p + q
    return r.v
