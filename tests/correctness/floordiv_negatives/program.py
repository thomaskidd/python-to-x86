def main() -> int:
    # Pins the Python floor-div / floor-mod semantics for mixed-sign
    # operands. CPython's `-7 // 2 == -4` and `-7 % 2 == 1`; LLVM's
    # raw sdiv/srem would give `-3` and `-1`. The compiler must emit
    # the correction blocks so this matches.
    return (-7 // 2) * 100 + (-7 % 2)
