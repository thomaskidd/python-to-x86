// Match the Python `floor mod` semantics so signed inputs behave
// identically. With non-negative inputs (the bench's range) this
// degenerates to the standard rem.
fn floor_mod(a: i64, b: i64) -> i64 {
    let r = a % b;
    if (r != 0) && ((r ^ b) < 0) { r + b } else { r }
}

fn py_main(mut a: i64, mut b: i64) -> i64 {
    while b != 0 {
        let t: i64 = b;
        b = floor_mod(a, b);
        a = t;
    }
    a
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: i64 = args[0].parse().unwrap();
    let b: i64 = args[1].parse().unwrap();
    println!("{}", py_main(a, b));
}
