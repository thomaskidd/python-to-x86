from __future__ import annotations
def main(n: int) -> int:
    return int(any(i * i > 100 for i in range(n)))
