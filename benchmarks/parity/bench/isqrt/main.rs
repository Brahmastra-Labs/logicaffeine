fn isqrt(n:i64)->i64 { if n<2 { return n; } let mut x=n; let mut y=(x+1)/2; while y<x { x=y; y=(x+n/x)/2; } x }
fn main(){ let n:i64=std::env::args().nth(1).unwrap().parse().unwrap();
	let mut s:i64=0; let mut i=1i64; while i<=n { s+=isqrt(i); i+=1; }
	println!("{}", s); }
