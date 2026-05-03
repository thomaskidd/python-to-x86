def main(a: int) -> int:
    # Sequential ifs with early return inside each. Exercises the
    # "function ends with return after a series of guards" pattern.
    if a < 0:
        return 0
    if a > 100:
        return 100
    return a
