from __future__ import annotations
def main(n: int) -> int:
    # Start from an empty literal (cap = 4) and insert n distinct keys.
    # Forces the dict to grow several times for n > 3.
    d: dict[int, int] = {}
    i: int = 0
    while i < n:
        d[i] = i * i
        i = i + 1
    # Read back two arbitrary keys to verify integrity after growth.
    a: int = d[n // 2]
    b: int = d[n - 1]
    return a * 1000 + b + len(d)
