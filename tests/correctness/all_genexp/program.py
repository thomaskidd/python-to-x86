from __future__ import annotations
def main(n: int) -> int:
    return int(all(i < n for i in range(n)))
