def f(a: int, b: int = 10, c: int = 100) -> int:
    return a * 10000 + b * 100 + c

def main(x: int) -> int:
    return f(x) + f(x, c=7) + f(x, b=8, c=9) + f(c=11, a=22, b=33)
