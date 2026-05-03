def main(a: int, n: int) -> int:
    # Left and right shift. The strategy bounds n in [0, 60] so we
    # avoid LLVM's UB on shift count >= bit width and Python's
    # ValueError on negative counts.
    return (a << n) ^ (a >> n)
