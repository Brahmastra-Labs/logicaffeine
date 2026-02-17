use std::env;
fn a(i: usize, j: usize) -> f64 { 1.0 / ((i+j)*(i+j+1)/2+i+1) as f64 }
fn mul_av(n: usize, v: &[f64], r: &mut [f64]) { for i in 0..n { r[i]=0.0; for j in 0..n { r[i]+=a(i,j)*v[j]; } } }
fn mul_atv(n: usize, v: &[f64], r: &mut [f64]) { for i in 0..n { r[i]=0.0; for j in 0..n { r[i]+=a(j,i)*v[j]; } } }
fn mul_atav(n: usize, v: &[f64], r: &mut [f64], t: &mut [f64]) { mul_av(n,v,t); mul_atv(n,t,r); }
fn main() {
    let n: usize = env::args().nth(1).unwrap().parse().unwrap();
    let mut u = vec![1.0f64; n];
    let mut v = vec![0.0f64; n];
    let mut t = vec![0.0f64; n];
    for _ in 0..10 { mul_atav(n,&u,&mut v,&mut t); mul_atav(n,&v,&mut u,&mut t); }
    let (mut vbv, mut vv) = (0.0, 0.0);
    for i in 0..n { vbv+=u[i]*v[i]; vv+=v[i]*v[i]; }
    println!("{:.9}", (vbv/vv).sqrt());
}
