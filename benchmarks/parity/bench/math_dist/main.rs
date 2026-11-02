use std::env;
fn main(){let n:i64=env::args().nth(1).unwrap().parse().unwrap();let mut s:i64=0;
for i in 1..=n { let dx=i as f64/1000.0-3.0; let dy=i as f64/700.0-4.0; s += ((dx*dx+dy*dy).sqrt()*1000000.0) as i64; }
println!("{}",s);}
