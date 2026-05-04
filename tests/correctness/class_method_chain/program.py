from __future__ import annotations
class Acc:
    total: int

    def __init__(self) -> None:
        self.total = 0

    def add(self, x: int) -> int:
        self.total = self.total + x
        return self.total

    def double(self) -> int:
        self.total = self.total * 2
        return self.total

def main(a: int, b: int) -> int:
    acc: Acc = Acc()
    acc.add(a)
    acc.add(b)
    acc.double()
    return acc.total
