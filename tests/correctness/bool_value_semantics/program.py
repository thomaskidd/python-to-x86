def main(a: int, b: int) -> int:
    # Python and/or return one of the operands, not just a bool:
    #   5 and 7 == 7
    #   5 or  7 == 5
    #   0 and 7 == 0
    #   0 or  7 == 7
    # Differential test pins this against CPython.
    x: int = a and b
    y: int = a or b
    return x * 100 + y
