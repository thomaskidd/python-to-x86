from __future__ import annotations
def main(n: int) -> int:
    rems: set[int] = {i % 5 for i in range(n)}
    return len(rems)
