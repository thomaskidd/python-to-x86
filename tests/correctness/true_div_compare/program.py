def main(a: int, b: int) -> int:
    # a / b is always float in Python (and us). Compare to a threshold.
    if b == 0:
        return -1
    q: float = a / b
    if q >= 1.0:
        return 1
    elif q <= -1.0:
        return -1
    else:
        return 0
