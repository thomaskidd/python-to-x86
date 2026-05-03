from __future__ import annotations
def make(x: int) -> tuple[int, int, int]:
    return (x, x * 2, x * 3)

def main(n: int) -> int:
    t: tuple[int, int, int] = make(n)
    return t[-1] - t[0]  # negative index supported (compile-time resolved)
