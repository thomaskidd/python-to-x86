from __future__ import annotations
def main(k: int) -> int:
    d: dict[int, int] = {0: 100, 1: 200, 2: 300, 3: 400, 4: 500}
    # Modulo to ensure k is in range.
    safe_k: int = k % 5
    if safe_k < 0:
        safe_k = safe_k + 5
    return d[safe_k]
