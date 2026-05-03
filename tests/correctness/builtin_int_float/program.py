def main(n: int) -> int:
    # Round-trip int → float → int via builtins.
    f: float = float(n)
    g: float = f * 2.0 + 1.0
    return int(g)
