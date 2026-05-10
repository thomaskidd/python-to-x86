from __future__ import annotations
class Money:
    cents: int
    def __init__(self, cents: int):
        self.cents = cents
    def __sub__(self, other: Money) -> Money:
        return Money(self.cents - other.cents)
    def __mul__(self, other: Money) -> int:
        return self.cents * other.cents

def main(a: int, b: int) -> int:
    p: Money = Money(a)
    q: Money = Money(b)
    diff: Money = p - q
    prod: int = p * q
    return diff.cents * 10000 + prod
