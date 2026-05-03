def main(a: int, b: int, c: int) -> int:
    # Exercises: params interleaved with literals, parens, floor-div,
    # mod, unary minus. Range bounds in strategy.toml are tight enough
    # that no intermediate result overflows i64.
    return (a + b) * c - (a - b) // 3 + (-c) % 7
