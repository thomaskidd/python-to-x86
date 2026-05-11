from __future__ import annotations
from typing import Callable

def reduce(f: Callable[[int, int], int], xs: list[int], init: int) -> int:
    acc: int = init
    i: int = 0
    while i < len(xs):
        acc = f(acc, xs[i])
        i = i + 1
    return acc

def main(n: int) -> int:
    xs: list[int] = []
    i: int = 0
    while i < n:
        xs.append(i + 1)
        i = i + 1
    # Sum xs with reduce + lambda.
    return reduce(lambda a, b: a + b, xs, 0)
