from __future__ import annotations
def main(n: int) -> int:
    total: int = 0
    for sq in (i * i for i in range(n)):
        if sq % 2 == 0:
            continue
        total = total + sq
    return total
