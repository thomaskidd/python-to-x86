from __future__ import annotations
def main(n: int) -> int:
    return int(all(i > 0 for i in range(0))) + n
