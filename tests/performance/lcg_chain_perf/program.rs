fn py_main(n: i64) -> i64 {
    let mut a: i64 = 1;
    let mut i: i64 = 0;
    while i < n {
        // Wrapping arithmetic — Python is arbitrary-precision, but
        // pyx86 uses i64 wrap-on-overflow. Match that explicitly so
        // the differential check passes.
        a = a.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        i = i + 1;
    }
    a
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n: i64 = args[0].parse().unwrap();
    println!("{}", py_main(n));
}
