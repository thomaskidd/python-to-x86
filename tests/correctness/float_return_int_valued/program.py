def main(n: int) -> float:
    # Promote n to float via arithmetic with float literal; result
    # is integer-valued, so the printer's %.17g + .0 matches Python repr.
    return n * 2.0
