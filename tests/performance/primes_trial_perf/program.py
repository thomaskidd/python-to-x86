def main(n: int) -> int:
    # Count primes in [2, n) by trial division up to sqrt(i).
    # Branch-heavy + nested while + break.
    count: int = 0
    i: int = 2
    while i < n:
        is_prime: int = 1
        d: int = 2
        while d * d <= i:
            if i % d == 0:
                is_prime = 0
                break
            d = d + 1
        if is_prime != 0:
            count = count + 1
        i = i + 1
    return count
