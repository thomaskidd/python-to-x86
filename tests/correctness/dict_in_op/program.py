from __future__ import annotations
def main(k: int) -> int:
    d: dict[int, int] = {10: 1, 20: 2, 30: 3}
    if k in d:
        return d[k] * 1000 + 1
    elif k not in d:
        return -1
    else:
        return 0
