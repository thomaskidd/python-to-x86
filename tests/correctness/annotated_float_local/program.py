def main(n: int) -> int:
    # Half of n via true div, then floor it back for an int comparison.
    half: float = n / 2
    # Compare float to integer-valued literal; both promoted to F64.
    if half * 2.0 >= n:
        return 1
    else:
        return 0
