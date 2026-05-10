from __future__ import annotations
def main(off: int) -> str:
    s: str = "hello world"
    # Slice well past the end — Python and our compiler should both clamp
    # to len(s) and return an empty (or near-empty) substring.
    return s[100 + off:200 + off]
