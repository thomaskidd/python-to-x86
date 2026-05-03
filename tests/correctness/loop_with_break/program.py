def main(n: int) -> int:
    # Find smallest i >= 0 where i * i >= n.
    i: int = 0
    while i < 1000:
        if i * i >= n:
            break
        i = i + 1
    return i
