fn gcd(mut a:i64, mut b:i64)->i64 { while b>0 { let t=b; b=a%b; a=t; } a }
fn main(){ let n:i64=std::env::args().nth(1).unwrap().parse().unwrap();
	let mut s:i64=0; let mut i=1i64;
	while i<=n { let mut j=i; while j<=n { s+=gcd(i,j); j+=1; } i+=1; }
	println!("{}", s); }
