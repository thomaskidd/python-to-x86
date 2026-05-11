from __future__ import annotations
from typing import Callable

def main(x: int) -> int:
    f: Callable[[int], int] = lambda v: v + 1
    return f(x)
