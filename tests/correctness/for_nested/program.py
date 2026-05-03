def main(n: int) -> int:
    total: int = 0
    for i in range(n):
        for j in range(n):
            total = total + i * j
    return total
