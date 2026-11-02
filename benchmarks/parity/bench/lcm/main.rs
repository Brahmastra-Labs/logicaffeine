fn gcd(mut a:i64, mut b:i64)->i64 { if a<0 {a=-a;} if b<0 {b=-b;} while b>0 { let t=b; b=a%b; a=t; } a }
fn lcm(a:i64,b:i64)->i64 { if a==0||b==0 { return 0; } let g=gcd(a,b); let p=(a*b).abs(); p/g }
fn main(){ let n:i64=std::env::args().nth(1).unwrap().parse().unwrap();
	let mut s:i64=0; let mut i=1i64; while i<=n { s+=lcm(i,60); i+=1; }
	println!("{}", s); }
