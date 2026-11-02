use std::env;
fn main(){let n:i64=env::args().nth(1).unwrap().parse().unwrap();let mut s:i64=0;
for i in 1..=n { let x=i as f64/1000.0; let q=(x/7.0).round_ties_even(); s += ((x-q*7.0)*1000000.0) as i64; }
println!("{}",s);}
