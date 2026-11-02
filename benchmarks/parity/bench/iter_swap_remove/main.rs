fn main(){ let n:i64=std::env::args().nth(1).unwrap().parse().unwrap();
    const MOD:i64=1000000007; let m:i64=1000;
    let xs:Vec<i64>=(0..m).collect(); let mut s:i64=0;
    for r in 0..n { let mut c=xs.clone(); let idx=(r % m) as usize; c.swap_remove(idx); s=(s + c.len() as i64) % MOD; }
    println!("{}", s); }
