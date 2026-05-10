from __future__ import annotations
def main(i: int) -> str:
    name: str = "item"
    ok: bool = i >= 0
    return f"{name} #{i}: ok={ok}"
