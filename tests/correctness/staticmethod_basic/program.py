from __future__ import annotations

class M:
    @staticmethod
    def add(a: int, b: int) -> int:
        return a + b

def main(a: int, b: int) -> int:
    return M.add(a, b)
