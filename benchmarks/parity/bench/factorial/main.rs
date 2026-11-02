fn fact(n:i64)->i64 { let mut r=1i64; let mut i=2i64; while i<=n { r*=i; i+=1; } r }
fn main(){ let n:i64=std::env::args().nth(1).unwrap().parse().unwrap();
	let mut s:i64=0; let mut i=1i64; while i<=n { s+=fact((i%12)+1); i+=1; }
	println!("{}", s); }
