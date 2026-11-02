fn main(){ let n:i64=std::env::args().nth(1).unwrap().parse().unwrap();
	let mut s:i64=0; let mut i=1i64; while i<=n { s+=(64-(i as u64).leading_zeros()) as i64; i+=1; }
	println!("{}",s); }
