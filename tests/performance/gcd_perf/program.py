def main(a: int, b: int) -> int:
    while b != 0:
        t: int = b
        b = a % b
        a = t
    return a
