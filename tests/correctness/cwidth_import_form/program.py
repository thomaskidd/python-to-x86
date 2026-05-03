from pyx86.types import i8

def main(a: int) -> int:
    # i8 is signed range ±127. Pass small values.
    b: i8 = a
    c: i8 = b + 1
    return c
