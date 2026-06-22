from __future__ import annotations
def main(n: int) -> int:
    for i in range(n):
        for j in range(n):
            if i * j > 20:
                return i * 100 + j
    return -1
