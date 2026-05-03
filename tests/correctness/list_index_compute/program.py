from __future__ import annotations
def main(n: int) -> int:
    lst: list[int] = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100]
    # Index modulo length to make any input safe.
    i: int = n % len(lst)
    if i < 0:
        i = i + len(lst)
    return lst[i]
