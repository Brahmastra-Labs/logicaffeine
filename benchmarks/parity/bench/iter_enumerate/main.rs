fn main(){ let n:i64=std::env::args().nth(1).unwrap().parse().unwrap();
    const MOD:i64=1000000007; let m:i64=1000;
    let xs:Vec<i64>=(0..m).collect(); let mut s:i64=0;
    for _ in 0..n { for (i,v) in xs.iter().enumerate() { s=(s + (i as i64 + 1) + *v) % MOD; } }
    println!("{}", s); }
