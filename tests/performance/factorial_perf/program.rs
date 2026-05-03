fn py_main(mut n: i64) -> i64 {
    let mut r: i64 = 1;
    while n > 1 {
        r = r * n;
        n = n - 1;
    }
    r
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n: i64 = args[0].parse().unwrap();
    println!("{}", py_main(n));
}
