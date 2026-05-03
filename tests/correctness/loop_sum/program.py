def main(n: int) -> int:
    i: int = 0
    total: int = 0
    while i < n:
        total = total + i
        i = i + 1
    return total
