from __future__ import annotations
from typing import Callable

def make_adder(n: int) -> Callable[[int], int]:
    return lambda x: x + n

def main(a: int, b: int) -> int:
    add: Callable[[int], int] = make_adder(a)
    return add(b)
