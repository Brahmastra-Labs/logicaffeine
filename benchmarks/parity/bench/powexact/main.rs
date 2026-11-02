fn powexact(base:i64, exp:i64)->i64{
    let (mut acc, mut b, mut e)=(1i64, base, exp);
    while e>0 { if e%2==1 { acc*=b; } e/=2; b*=b; }
    acc
}
fn main(){
    let n:i64=std::env::args().nth(1).unwrap().parse().unwrap();
    let mut s:i64=0; let mut i=1i64;
    while i<=n { s+=powexact((i%5)+1, i%13); i+=1; }
    println!("{}", s);
}
