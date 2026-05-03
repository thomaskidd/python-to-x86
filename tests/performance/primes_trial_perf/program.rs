// Match Python's floor-mod semantics for the differential check.
// All operands are non-negative in this benchmark, so this
// degenerates to plain `%`, but we keep the helper to mirror
// what pyx86 emits.
fn floor_mod(a: i64, b: i64) -> i64 {
    let r = a % b;
    if (r != 0) && ((r ^ b) < 0) { r + b } else { r }
}

fn py_main(n: i64) -> i64 {
    let mut count: i64 = 0;
    let mut i: i64 = 2;
    while i < n {
        let mut is_prime: i64 = 1;
        let mut d: i64 = 2;
        while d * d <= i {
            if floor_mod(i, d) == 0 {
                is_prime = 0;
                break;
            }
            d = d + 1;
        }
        if is_prime != 0 {
            count = count + 1;
        }
        i = i + 1;
    }
    count
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n: i64 = args[0].parse().unwrap();
    println!("{}", py_main(n));
}
