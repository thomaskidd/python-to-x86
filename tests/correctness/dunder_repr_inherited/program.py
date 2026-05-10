from __future__ import annotations
class Named:
    name: str
    def __init__(self, name: str):
        self.name = name
    def __repr__(self) -> str:
        return f"Named({self.name})"

class Tagged(Named):
    tag: int
    def __init__(self, name: str, tag: int):
        super().__init__(name)
        self.tag = tag
    # No __repr__ override — inherits Named's.

def main(t: int) -> str:
    obj: Tagged = Tagged("alpha", t)
    return repr(obj)
