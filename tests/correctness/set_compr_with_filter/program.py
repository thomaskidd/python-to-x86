from __future__ import annotations
def main(n: int) -> int:
    evens: set[int] = {i for i in range(n) if i % 2 == 0}
    return len(evens)
