def main(n: int) -> int:
    total: int = 0
    for i in range(0, n, 3):
        total = total + i
    return total
