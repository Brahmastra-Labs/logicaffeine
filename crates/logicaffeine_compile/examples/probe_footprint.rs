//! What the footprint lock sees: the buffers of a 2048x2048x1024 GEMM on the production Wormhole grid.
use logicaffeine_compile::spatial::pipeline::compile_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;

fn gemm_program(m: usize, n: usize, k: usize) -> String {
    format!(
        r#"## Main
Let rows be {m}.
Let cols be {n}.
Let mid be {k}.
Let mutable a be a new Seq of Int.
Let mutable b be a new Seq of Int.
Let mutable c be a new Seq of Int.
Let mutable p be 0.
While p is less than rows * mid:
    Push (p % 7) - 3 to a.
    Set p to p + 1.
Set p to 0.
While p is less than mid * cols:
    Push (p % 11) - 5 to b.
    Set p to p + 1.
Let d be the matmul of a and b with rows rows cols cols mid mid.
Show d.
"#
    )
}

fn main() {
    let wh = SpatialTarget::wormhole();
    let p = compile_to_spatial(&gemm_program(2048, 2048, 1024), &wh).expect("compiles");
    let s = p.summary();
    println!(
        "plan {:?} grid {:?} per_site {:?} block {:?} nb {:?}",
        s.arrangement_label(),
        s.grid,
        s.per_site_block,
        s.block,
        s.num_blocks
    );
    let site = *p.sites().iter().next().expect("a site");
    println!("first site {site:?}, sites {}", p.sites().len());
    for b in p.buffers.iter().filter(|b| b.site == Some(site)) {
        println!("  {:<12} {:?}  bytes {}", b.name, b.kind, p.buffer_bytes(b));
    }
}
