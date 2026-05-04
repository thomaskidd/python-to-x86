from __future__ import annotations
def main(n: int) -> int:
    # Aliasing: a and b refer to the SAME heap list. .append() through
    # one is visible via the other (Python semantics).
    a: list[int] = [1, 2]
    b: list[int] = a
    a.append(n)
    return len(a) * 1000 + len(b) * 100 + b[2]
