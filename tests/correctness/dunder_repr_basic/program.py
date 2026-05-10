from __future__ import annotations
class Box:
    v: int
    def __init__(self, v: int):
        self.v = v
    def __repr__(self) -> str:
        return f"Box({self.v})"

def main(v: int) -> str:
    b: Box = Box(v)
    return repr(b)
