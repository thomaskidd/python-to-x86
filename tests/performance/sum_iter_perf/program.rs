fn py_main(n: i64) -> i64 {
    let mut i: i64 = 0;
    let mut total: i64 = 0;
    while i < n {
        total = total + i;
        i = i + 1;
    }
    total
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n: i64 = args[0].parse().unwrap();
    println!("{}", py_main(n));
}
