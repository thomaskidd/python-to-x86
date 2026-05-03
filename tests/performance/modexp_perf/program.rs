fn floor_mod(a: i64, b: i64) -> i64 {
    let r = a % b;
    if (r != 0) && ((r ^ b) < 0) { r + b } else { r }
}

fn floor_div(a: i64, b: i64) -> i64 {
    let q = a / b;
    let r = a % b;
    if (r != 0) && ((a ^ b) < 0) { q - 1 } else { q }
}

fn py_main(mut base: i64, mut exp: i64) -> i64 {
    let m: i64 = 1000000007;
    let mut total: i64 = 0;
    let mut i: i64 = 0;
    while i < 1000000 {
        let mut b: i64 = base;
        let mut e: i64 = exp;
        let mut result: i64 = 1;
        while e > 0 {
            if floor_mod(e, 2) == 1 {
                result = floor_mod(result * b, m);
            }
            b = floor_mod(b * b, m);
            e = floor_div(e, 2);
        }
        total = total + result;
        base = floor_mod(base * 17 + 31, m);
        exp = floor_mod(exp + 1, 64);
        i = i + 1;
    }
    floor_mod(total, m)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let base: i64 = args[0].parse().unwrap();
    let exp: i64 = args[1].parse().unwrap();
    println!("{}", py_main(base, exp));
}
