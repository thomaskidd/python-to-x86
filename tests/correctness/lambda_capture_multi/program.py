from __future__ import annotations
from typing import Callable

def apply(f: Callable[[int], int], x: int) -> int:
    return f(x)

def main(a: int, b: int) -> int:
    name: str = "ignored"  # used to test that non-captured names don't end up in env
    p: int = a
    q: int = b
    g: Callable[[int], int] = lambda x: x * p + q
    return apply(g, a + b)
