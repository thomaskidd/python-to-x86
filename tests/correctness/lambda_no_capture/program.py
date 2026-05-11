from __future__ import annotations
from typing import Callable

def apply(f: Callable[[int], int], x: int) -> int:
    return f(x)

def main(x: int) -> int:
    return apply(lambda v: v * 2, x)
