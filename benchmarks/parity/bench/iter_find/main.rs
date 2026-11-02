fn main(){ let n:i64=std::env::args().nth(1).unwrap().parse().unwrap();
    let m:i64=1000; let xs:Vec<i64>=(0..m).collect(); let mut s:i64=0;
    for _ in 0..n { let idx=xs.iter().position(|&v| v>500).map(|p| p as i64 + 1).unwrap_or(0); s+=idx; }
    println!("{}", s); }
