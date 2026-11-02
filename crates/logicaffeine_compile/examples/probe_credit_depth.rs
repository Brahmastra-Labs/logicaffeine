//! Two-sided check on the credit-loop depth rule: a part whose fetch is fast must double-buffer, and one
//! whose fetch outruns its consumer must deepen. Same program, same shape, only the declared latency differs.
use logicaffeine_compile::spatial::ir::{BufferKind, NumericsPolicy};
use logicaffeine_compile::spatial::pipeline::{compile_to_spatial, template_from_source_with};
use logicaffeine_compile::spatial::target::{CreditLatency, SpatialTarget};

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
Set p to 0.
While p is less than rows * cols:
    Push 0 to c.
    Set p to p + 1.
Let mutable i be 0.
While i is less than rows:
    Let mutable k be 0.
    While k is less than mid:
        Let mutable j be 0.
        While j is less than cols:
            Let idx be i * cols + j + 1.
            Set item idx of c to item idx of c + item (i * mid + k + 1) of a * item (k * cols + j + 1) of b.
            Set j to j + 1.
        Set k to k + 1.
    Set i to i + 1.
Let mutable checksum be 0.
Let mutable q be 1.
While q is at most rows * cols:
    Set checksum to checksum + item q of c.
    Set q to q + 1.
Show checksum.
"#
    )
}

fn depths(label: &str, credit: Option<CreditLatency>) {
    let mut t = SpatialTarget::wormhole();
    t.site.credit = credit;
    let p = compile_to_spatial(&gemm_program(2048, 1024, 512), &t).expect("compiles");
    let mut seen: Vec<(String, usize)> = Vec::new();
    for b in &p.buffers {
        if let BufferKind::Fifo { depth_pages, .. } = b.kind {
            let base = b.name.split('@').next().unwrap_or(&b.name).to_string();
            if !seen.iter().any(|(n, _)| *n == base) {
                seen.push((base, depth_pages));
            }
        }
    }
    let d: Vec<String> = seen.iter().map(|(n, d)| format!("{n}={d}")).collect();
    println!("  {label:<34} {}", d.join("  "));
    let _ = template_from_source_with(&gemm_program(8, 8, 8), &t, NumericsPolicy::Exact);
}

fn main() {
    {
        let wh = SpatialTarget::wormhole();
        let p = compile_to_spatial(&gemm_program(2048, 2048, 1024), &wh).expect("compiles");
        let s = p.summary();
        println!("footprint fixture 2048x2048x1024 default numerics: {} per_site {:?} block {:?} nb {:?}", s.arrangement_label(), s.per_site_block, s.block, s.num_blocks);
        let site = *p.sites().iter().next().unwrap();
        for b in p.buffers.iter().filter(|b| b.site == Some(site)) {
            println!(
                "   {:<12} {:?} dtype {:?} bytes {}",
                b.name,
                b.kind,
                b.dtype,
                p.buffer_bytes(b)
            );
        }
        println!();
    }
    println!("credit-loop depth, 2048x1024x512, same program, only the declared latency differs:");
    depths("undeclared (vendor default)", None);
    depths(
        "fast fetch  (8 / 4 / 200)",
        Some(CreditLatency {
            fetch_tile: 8,
            credit_return: 4,
            service_page: 200,
        }),
    );
    for (f, r, sv) in [
        (4u32, 8u32, 200u32),
        (6, 8, 200),
        (8, 8, 200),
        (12, 8, 200),
        (16, 8, 200),
        (24, 8, 200),
    ] {
        depths(
            &format!("fetch {f} return {r} service {sv}"),
            Some(CreditLatency {
                fetch_tile: f,
                credit_return: r,
                service_page: sv,
            }),
        );
    }
}
