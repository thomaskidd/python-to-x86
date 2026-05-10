from __future__ import annotations
class A:
    v: int
    def __init__(self, v: int):
        self.v = v
    def __eq__(self, other: A) -> bool:
        return self.v == other.v

class B(A):
    extra: int
    def __init__(self, v: int, extra: int):
        super().__init__(v)
        self.extra = extra
    def __eq__(self, other: B) -> bool:
        # Override: equal iff both v AND extra match.
        if self.v == other.v:
            if self.extra == other.extra:
                return True
        return False

def main(v: int, e: int) -> int:
    p: B = B(v, e)
    q: B = B(v, e + 1)
    result: int = 0
    if p == q:
        result = result + 1
    r: B = B(v, e)
    if p == r:
        result = result + 10
    return result
