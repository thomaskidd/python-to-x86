fn py_main(n: i64) -> i64 {
    let mut a: i64 = 0;
    let mut b: i64 = 1;
    let mut i: i64 = 0;
    while i < n {
        let t: i64 = a + b;
        a = b;
        b = t;
        i = i + 1;
    }
    a
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n: i64 = args[0].parse().unwrap();
    println!("{}", py_main(n));
}
