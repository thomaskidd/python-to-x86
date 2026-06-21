from __future__ import annotations
def main(n: int) -> int:
    return sum(i for i in range(n) if i % 3 == 0)
