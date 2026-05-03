from math import floor, ceil
def main(n: int) -> int:
    # Both floor(n + 0.4) and ceil(n + 0.4) are integer-valued floats;
    # convert back to int for the diff (avoids printer concerns).
    f: float = floor(float(n) + 0.5)
    c: float = ceil(float(n) + 0.5)
    return int(f) * 1000 + int(c)
