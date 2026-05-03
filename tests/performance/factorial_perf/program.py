def main(n: int) -> int:
    r: int = 1
    while n > 1:
        r = r * n
        n = n - 1
    return r
