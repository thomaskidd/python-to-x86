def main(a: int, b: int) -> int:
    # Euclidean algorithm. Reassigns both vars inside the loop.
    while b != 0:
        t: int = b
        b = a % b
        a = t
    return a
