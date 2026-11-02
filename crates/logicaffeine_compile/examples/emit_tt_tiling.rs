//! Print the tiling the LOGOS autotuner chooses for a matmul on a chip — the config the measurement harness
//! passes to `ttnn.matmul` as "our config", so that comparison is genuinely the compiler's choice at ANY
//! shape rather than a default carried over from one. Usage:
//!   cargo run -p logicaffeine-compile --features codegen --example emit_tt_tiling -- M N K [wormhole|blackhole]
//! Prints one shell-parseable line: `LOGOS_TILING grid=GXxGY per_core=PMxPN in0_block_w=W subblock=SHxSW`.

use logicaffeine_compile::codegen_tt::matmul_lower::{
    tune_tiling_with, vendor_dispatch_blocking, ChipKnobs, ComputeKnobs,
};
use logicaffeine_compile::tensor_recognize::MatmulSpec;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 3 {
        eprintln!("usage: emit_tt_tiling M N K [wormhole|blackhole]");
        std::process::exit(2);
    }
    let n = |i: usize| a[i].parse::<usize>().expect("integer");
    let (m, nn, k) = (n(0), n(1), n(2));
    let chip = match a.get(3).map(String::as_str).unwrap_or("wormhole") {
        "blackhole" => ChipKnobs::blackhole(),
        _ => ChipKnobs::wormhole(),
    };
    let spec = MatmulSpec {
        m,
        n: nn,
        k,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    let cfg = tune_tiling_with(&spec, &chip, &ComputeKnobs::default());
    // The SAME definition the dispatch emitter uses — not a copy of it. The vendor's 2-D multicast op requires
    // the output block count along each axis to fit the grid; the autotuner satisfies that by construction.
    let (ibw, osh, osw) = vendor_dispatch_blocking(&spec, &cfg);
    debug_assert_eq!(
        nn / 32 / cfg.per_core_n,
        cfg.grid_x,
        "output blocks along x must equal the grid columns"
    );
    println!(
        "LOGOS_TILING grid={}x{} per_core={}x{} in0_block_w={} subblock={}x{}",
        cfg.grid_x, cfg.grid_y, cfg.per_core_m, cfg.per_core_n, ibw, osh, osw
    );
}
