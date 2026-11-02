fn modinverse(x:i64, m:i64)->i64{
    let mut old_r=x%m; if old_r<0 { old_r+=m; }
    let (mut r, mut old_s, mut s)=(m, 1i64, 0i64);
    while r>0 {
        let q=old_r/r;
        let t_r=old_r-q*r; old_r=r; r=t_r;
        let t_s=old_s-q*s; old_s=s; s=t_s;
    }
    if old_r>1 { return 0; }
    let inv=old_s%m;
    if inv<0 { inv+m } else { inv }
}
fn main(){
    let n:i64=std::env::args().nth(1).unwrap().parse().unwrap();
    let m:i64=1000000007; let mut sum:i64=0; let mut i=1i64;
    while i<=n { sum+=modinverse(i,m); i+=1; }
    println!("{}", sum);
}
