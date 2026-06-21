from __future__ import annotations
def main(n: int) -> int:
    return int(any(i > 5 for i in range(n) if i % 2 == 0))
