def main(n: int) -> int:
    a: str = "abc"
    b: str = "abc"
    c: str = "abd"
    eq_same: int = 0
    if a == b:
        eq_same = 1
    eq_diff: int = 0
    if a == c:
        eq_diff = 1
    neq_same: int = 0
    if a != c:
        neq_same = 1
    return eq_same * 100 + eq_diff * 10 + neq_same + n
