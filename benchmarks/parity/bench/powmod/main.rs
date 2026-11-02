fn powmod(base:i64, exp:i64, m:i64)->i64 { let mut acc=1i64; let mut b=base%m; let mut e=exp;
	while e>0 { if e%2==1 { acc=(acc*b)%m; } e/=2; b=(b*b)%m; } acc }
fn main(){ let n:i64=std::env::args().nth(1).unwrap().parse().unwrap();
	let m:i64=1000000007; let mut s:i64=0; let mut i=1i64; while i<=n { s+=powmod(i,i,m); i+=1; }
	println!("{}", s); }
