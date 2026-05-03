from math import sqrt
def main(n: int) -> float:
    # Returns sqrt of (n*n) which equals abs(n) when n >= 0; for our
    # input range the result is integer-valued so the printer matches
    # CPython's repr.
    x: float = float(n * n)
    return sqrt(x)
