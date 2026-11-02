fn binom(n:i64,k:i64)->i64 { if k<0 { return 0; } if k>n { return 0; } let mut kk=k; if kk>n-kk { kk=n-kk; }
	let mut acc=1i64; let mut i=1i64; while i<=kk { acc=acc*(n-kk+i)/i; i+=1; } acc }
fn main(){ let n:i64=std::env::args().nth(1).unwrap().parse().unwrap();
	let mut s:i64=0; let mut i=1i64; while i<=n { s+=binom(30, (s+i)%16); i+=1; }
	println!("{}", s); }
