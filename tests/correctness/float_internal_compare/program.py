def main(a: int) -> int:
    # Compute a float result, compare it, return int.
    # 1.0 + 2.0 = 3.0 always; comparison to 0.0 is well-defined.
    x: float = 1.0 + 2.0
    if x > 0.0:
        return a
    else:
        return -a
