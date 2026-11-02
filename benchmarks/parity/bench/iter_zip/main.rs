fn main(){ let n:i64=std::env::args().nth(1).unwrap().parse().unwrap();
    const MOD:i64=1000000007; let m:i64=1000;
    let xs:Vec<i64>=(0..m).collect(); let ys:Vec<i64>=(0..m).map(|k| k+m).collect(); let mut s:i64=0;
    for _ in 0..n { for (a,b) in xs.iter().zip(ys.iter()) { s=(s + *a + *b) % MOD; } }
    println!("{}", s); }
