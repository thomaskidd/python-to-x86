def main(a: int, b: int) -> int:
    # Mix of and / or / xor / not.
    return (a & b) * 1000 + (a | b) - (a ^ b) + (~a)
