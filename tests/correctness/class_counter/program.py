from __future__ import annotations
class Counter:
    count: int

    def __init__(self, start: int):
        self.count = start

    def increment(self) -> int:
        self.count = self.count + 1
        return self.count

def main(start: int) -> int:
    c: Counter = Counter(start)
    c.increment()
    c.increment()
    c.increment()
    return c.count
