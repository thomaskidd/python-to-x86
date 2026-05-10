from __future__ import annotations
def main(k: int, v: int) -> int:
    d: dict[int, int] = {}
    d[k] = v
    return d[k] + len(d)
