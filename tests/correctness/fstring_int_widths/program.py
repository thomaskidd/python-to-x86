from pyx86.types import i32

def main(a: int, b: int) -> str:
    a32: i32 = a
    return f"a32={a32}, b={b}"
