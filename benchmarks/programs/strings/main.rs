// strings — append the decimal text of 0..n-1, each followed by a space, then count the spaces and hash the bytes. Rust's
// standard library has no allocation-free integer append, so the digits are written by hand, as main.c writes them.

// Appends v's decimal digits to out, v >= 0.
fn put_digits(out: &mut Vec<u8>, mut v: i64) {
    let mut tmp = [0u8; 20];
    let mut k = 0;
    loop {
        tmp[k] = b'0' + (v % 10) as u8;
        k += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    for j in (0..k).rev() {
        out.push(tmp[j]);
    }
}

fn main() {
    let n: i64 = std::env::args().nth(1).unwrap().parse().unwrap();
    let mut result: Vec<u8> = Vec::with_capacity(n as usize * 6);
    for i in 0..n {
        put_digits(&mut result, i);
        result.push(b' ');
    }
    let spaces = result.iter().filter(|&&b| b == b' ').count();
    let mut h: i64 = 0;
    for &b in &result {
        h = (h * 31 + b as i64) & 268435455;
    }
    println!("{} {}", spaces, h);
}
