fn main() {
    let n: i64 = std::env::args().nth(1).unwrap().parse().unwrap();
    let mut result = String::with_capacity(n as usize * 6);
    for i in 0..n {
        result.push_str(&i.to_string());
        result.push(' ');
    }
    let spaces = result.chars().filter(|&c| c == ' ').count();
    println!("{}", spaces);
}
