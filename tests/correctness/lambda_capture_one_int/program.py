from __future__ import annotations
from typing import Callable

def apply(f: Callable[[int], int], x: int) -> int:
    return f(x)

def main(a: int, b: int) -> int:
    g: Callable[[int], int] = lambda x: x + a
    return apply(g, b)
