from __future__ import annotations
def main(n: int) -> int:
    squares: dict[int, int] = {i: i * i for i in range(n)}
    total: int = 0
    for i in range(n):
        total = total + squares[i]
    return total + len(squares)
