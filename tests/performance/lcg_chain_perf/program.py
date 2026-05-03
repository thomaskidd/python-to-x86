def main(n: int) -> int:
    # Linear congruential generator. The data dependency from one
    # iteration to the next prevents LLVM from folding the loop.
    a: int = 1
    i: int = 0
    while i < n:
        a = a * 6364136223846793005 + 1442695040888963407
        i = i + 1
    return a
