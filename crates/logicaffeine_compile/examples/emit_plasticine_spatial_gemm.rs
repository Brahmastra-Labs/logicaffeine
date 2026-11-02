//! Write a LOGOS matmul as Spatial's GEMM regression app (`class Logos_gemm_MxNxK extends SpatialTest`) into a
//! directory — the public toolchain's `regression/` dir, so `gate.sh Logos_gemm_MxNxK` runs OUR GEMM through
//! Spatial → PIR → plastiroute → Tungsten. Usage:
//!   cargo run -p logicaffeine-compile --example emit_plasticine_spatial_gemm -- <out_dir> M N K [ts its ip]

use logicaffeine_compile::codegen_sva::plasticine_report::{logos_gemm_program, lower_gemm_source};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        eprintln!("usage: emit_plasticine_spatial_gemm <out_dir> M N K [ts its ip]");
        std::process::exit(2);
    }
    let out = &args[0];
    let num = |i: usize, d: usize| {
        args.get(i)
            .map(|s| s.parse::<usize>().expect("integer"))
            .unwrap_or(d)
    };
    let (m, n, k) = (num(1, 640), num(2, 640), num(3, 640));
    let (ts, its, ip) = (num(4, 64), num(5, 64), num(6, 16));
    let source = logos_gemm_program(m, n, k);
    let (m, n, k, class, spatial) = lower_gemm_source(&source, ts, its, ip).unwrap_or_else(|e| {
        eprintln!("refused: {e}");
        std::process::exit(1);
    });
    std::fs::create_dir_all(out).expect("out dir");
    let path = std::path::Path::new(out).join(format!("{class}.scala"));
    std::fs::write(&path, &spatial).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    std::fs::write(
        std::path::Path::new(out).join(format!("{class}.lg")),
        &source,
    )
    .unwrap();
    println!(
        "{class} → {} ({m}x{n}x{k}, ts {ts} its {its} ip {ip})",
        path.display()
    );
    println!("  python3 scripts/accel/plasticine-gate-ledger.py add --kind gemm --app {class} --app-source \"emitted from LOGOS by spatial_emit::emit_spatial_gemm_test ({class}.lg, Spatial's GEMM structure)\" --gen-dir target/plasticine-toolchain/spatial/gen/Tst_pirTest/{class} --shape {m}x{n}x{k} --params \"ts={ts} its={its} ip={ip} loop_ii=1 loop_jj=1 loop_kk=1 loop_i=1 loop_j=1\" --toolchain \"…\"");
}
