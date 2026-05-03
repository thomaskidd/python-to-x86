def main(n: int) -> int:
    # Sum only odd values in [0, n).
    i: int = 0
    total: int = 0
    while i < n:
        if i % 2 == 0:
            i = i + 1
            continue
        total = total + i
        i = i + 1
    return total
