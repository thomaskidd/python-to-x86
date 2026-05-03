def main(start: int, stop: int) -> int:
    total: int = 0
    for i in range(start, stop):
        total = total + i
    return total
