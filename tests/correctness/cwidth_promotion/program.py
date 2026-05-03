from pyx86.types import i32, i16

def main(a: int) -> int:
    short: i16 = a
    longer: i32 = short
    # short + 1 → max(i16, i64-of-1) = i64; result is i64.
    return longer + a
