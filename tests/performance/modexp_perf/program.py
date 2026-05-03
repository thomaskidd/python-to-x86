def main(base: int, exp: int) -> int:
    # base^exp mod 1_000_000_007 by binary exponentiation, repeated
    # in an outer LCG-driven loop so the work scales beyond one
    # binary-exp call.
    M: int = 1000000007
    total: int = 0
    i: int = 0
    while i < 1000000:
        # Update base / exp via a deterministic chain so each
        # iteration is different and LLVM can't pre-compute.
        b: int = base
        e: int = exp
        result: int = 1
        while e > 0:
            if e % 2 == 1:
                result = (result * b) % M
            b = (b * b) % M
            e = e // 2
        total = total + result
        # Mutate base / exp for the next outer iteration.
        base = (base * 17 + 31) % M
        exp = (exp + 1) % 64
        i = i + 1
    return total % M
