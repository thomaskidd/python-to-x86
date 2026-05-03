from pyx86.types import i32

def main(a: int) -> int:
    # Internally use i32; result fits in i64 after sign-extending back.
    x: i32 = a
    y: i32 = x * x
    return y
