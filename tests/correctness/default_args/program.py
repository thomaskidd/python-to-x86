def offset(x: int, by: int = 10) -> int:
    return x + by

def main(x: int) -> int:
    return offset(x) + offset(x, 100)
