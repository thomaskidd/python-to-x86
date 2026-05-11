from __future__ import annotations
def main(off: int) -> int:
    xs: list[int] = [1, 2, 3]
    # Slice past len → empty (clamped).
    ys: list[int] = xs[100 + off:200 + off]
    return len(ys)
