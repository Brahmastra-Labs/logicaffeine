use std::env;
fn main() {
    let n: usize = env::args().nth(1).unwrap().parse().unwrap();
    let mut perm1: Vec<i64> = (0..n as i64).collect();
    let mut perm = vec![0i64; n];
    let mut count = vec![0i64; n];
    let mut max_flips = 0i64;
    let mut checksum = 0i64;
    let mut perm_count = 0i64;
    let mut r = n;
    loop {
        while r > 1 { count[r - 1] = r as i64; r -= 1; }
        perm.copy_from_slice(&perm1);
        let mut flips = 0i64;
        while perm[0] != 0 {
            let k = perm[0] as usize + 1;
            for i in 0..k / 2 { perm.swap(i, k - 1 - i); }
            flips += 1;
        }
        if flips > max_flips { max_flips = flips; }
        checksum += if perm_count % 2 == 0 { flips } else { -flips };
        perm_count += 1;
        loop {
            if r == n { println!("{}\n{}", checksum, max_flips); return; }
            let p0 = perm1[0];
            for i in 0..r { perm1[i] = perm1[i + 1]; }
            perm1[r] = p0;
            count[r] -= 1;
            if count[r] > 0 { break; }
            r += 1;
        }
    }
}
