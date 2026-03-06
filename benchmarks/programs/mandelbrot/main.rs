use std::env;
fn main() {
    let n: i32 = env::args().nth(1).unwrap().parse().unwrap();
    let mut count = 0;
    for y in 0..n { for x in 0..n {
        let cr = 2.0 * x as f64 / n as f64 - 1.5;
        let ci = 2.0 * y as f64 / n as f64 - 1.0;
        let (mut zr, mut zi) = (0.0, 0.0);
        let mut inside = true;
        for _ in 0..50 {
            let t = zr*zr - zi*zi + cr; zi = 2.0*zr*zi + ci; zr = t;
            if zr*zr + zi*zi > 4.0 { inside = false; break; }
        }
        if inside { count += 1; }
    }}
    println!("{}", count);
}
