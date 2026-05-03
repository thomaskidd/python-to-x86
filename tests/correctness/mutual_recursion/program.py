def is_even(n: int) -> int:
    if n == 0:
        return 1
    return is_odd(n - 1)

def is_odd(n: int) -> int:
    if n == 0:
        return 0
    return is_even(n - 1)

def main(n: int) -> int:
    return is_even(n)
