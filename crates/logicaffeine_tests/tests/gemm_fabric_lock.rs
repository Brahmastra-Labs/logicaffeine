//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! GEMM-FABRIC LOCK — the certified reconfigurable-dataflow demonstrator's correctness certificate.
//!
//! The workload of inference is matrix multiply, and the heart of matrix multiply is the int8
//! multiply-accumulate (MAC) dot-product: `acc += a·b`. This lock proves that MAC — the exact datapath
//! we place spatially onto the fabric and run on silicon — is byte-identical to the reference at every
//! hop, which is the property no mainstream cross-vendor ML compiler guarantees:
//!
//!   Tier 1 — FABRIC EVAL ≡ VM. The dot-product `Σ k·(n-k)` lowers to the reconfigurable loop fabric
//!            (`loop_fabric_from_source`) and genuinely emits a `Mul` + `Add` tile; `eval_loop_fabric`
//!            matches the VM across an input battery.
//!   Tier 2 — PLACED GRID ≡ DATAPATH, by our own SAT. The MAC datapath `acc + a*b` placed-and-routed
//!            onto the 2-D tile array (`route_loop_body`) is proven EQUAL to the abstract datapath for
//!            ALL inputs (bit-blasted, refuted by `prove_unsat`) — the mapping preserves semantics.
//!   Tier 3 — ON THE SOFT-CORE RTL ≡ VM. The same MAC, inlined into the scalar ISA, is compiled to
//!            bytecode, streamed over the UART protocol into `nexys_soc_scalar.v` under iverilog, and
//!            read back ≡ VM — the no-board twin of the Artix-7 run.
//!   Plus — FULL int8 GEMM ≡ an independent Rust oracle on the VM (the recognizable matmul, not just a
//!          dot-product), so the named artifact is a real matrix multiply.
//!
//! Reuses the machinery proven by `fabric_op_matrix_lock` (eval ≡ VM), `verified_placer_lock` (placer
//! certified by SAT), and `nexys_bytecode_over_wire_lock` (soft-core ≡ VM). iverilog-gated: the RTL leg
//! skips cleanly (loud note) where the simulator is unavailable.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use std::path::PathBuf;
use std::process::Command;

use logicaffeine_compile::codegen_sva::bitblast::lower_bool;
use logicaffeine_compile::codegen_sva::fabric_grid::{
    eval_grid, grid_output_to_bounded, route_loop_body,
};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    emit_loop_fabric_verilog, eval_fabric, eval_loop_fabric, fabric_to_bounded,
    loop_fabric_from_source, AluOp, FabricOp, LoopCell, Src,
};
use logicaffeine_compile::codegen_sva::scalar_asm::{compile_to_scalar_words, to_hex};
use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
use logicaffeine_proof::ProofExpr;

// ── The MAC dot-product kernel, in two surface forms that compute the SAME function ─────────────────

/// Function form (for fabric lowering): `dotMac(n) = Σ_{k=1..n} k·(n-k)` — a genuine dot-product of the
/// int8-range vectors a=[1..n] and b=[n-1..0]. Multiply-accumulate, the GEMM inner kernel.
const DOTMAC_FN: &str = "## To dotMac (n: Int) -> Int:\n    Let mutable s be 0.\n    Repeat for k from 1 to n:\n        Set s to s + k * (n - k).\n    Return s.\n";

/// The reference dot-product, computed independently in Rust.
fn dotmac(n: i64) -> i64 {
    (1..=n).map(|k| k * (n - k)).sum()
}

/// Inline form for the scalar soft-core (no functions / division / collections there): the SAME MAC as
/// `dotMac(N)`, unrolled into `## Main` with a `While` counter.
fn dotmac_inline(n: i64) -> String {
    format!(
        "## Main\n    Let mutable s be 0.\n    Let mutable k be 1.\n    While k is at most {n}:\n        Set s to s + k * ({n} - k).\n        Set k to k + 1.\n    Show s.\n"
    )
}

// ── helpers (mirrors of the reference locks) ────────────────────────────────────────────────────────

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// VM result of `<defs>## Main Show <fname>(x).` (decimal int).
fn vm_call_result(src: &str, fname: &str, x: i64) -> String {
    let full = format!("{src}## Main\n    Show {fname}({x}).\n");
    let out = vm_outcome(&full);
    assert!(
        out.error.is_none(),
        "{fname}({x}): VM error {:?}",
        out.error
    );
    out.output.trim().to_string()
}

/// VM Show stream of a whole program, space-joined (matches the SoC `RESULT:` form).
fn vm_shows(src: &str) -> String {
    let out = vm_outcome(src);
    assert!(out.error.is_none(), "VM error on {src:?}: {:?}", out.error);
    out.output.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn body_has_mul(body: &[LoopCell]) -> bool {
    body.iter()
        .any(|c| matches!(c, LoopCell::Arith { op: AluOp::Mul, .. }))
}
fn body_has_add(body: &[LoopCell]) -> bool {
    body.iter()
        .any(|c| matches!(c, LoopCell::Arith { op: AluOp::Add, .. }))
}

/// `LoopCell::Arith` and `FabricOp` share `{op, a, b}` — direct transcription for the placer.
fn to_cells(body: &[FabricOp]) -> Vec<LoopCell> {
    body.iter()
        .map(|o| LoopCell::Arith {
            op: o.op,
            a: o.a,
            b: o.b,
        })
        .collect()
}

/// Prove `eval_grid(route_loop_body(body)) == eval_fabric(body)` for ALL inputs at width `w`, by our own
/// certified SAT (refute the negation). `n_in` inputs; `result` is the output cell.
fn placement_is_certified(body: &[FabricOp], n_in: usize, result: usize, w: u32) -> bool {
    let cells = to_cells(body);
    let g = route_loop_body(&cells, n_in, &[Src::Val(result)]);
    let grid = grid_output_to_bounded(&g, 0, w, &|k| format!("in{k}"));
    let datapath = fabric_to_bounded(body, result, w);
    let claim = BoundedExpr::Eq(Box::new(grid), Box::new(datapath));
    let p = lower_bool(&claim)
        .expect("the MAC placement-equivalence claim lowers to a bounded formula");
    prove_unsat(&ProofExpr::Not(Box::new(p))) == UnsatOutcome::Refuted
}

// ── the int8 GEMM (the full, recognizable matmul) ───────────────────────────────────────────────────

/// Independent Rust oracle for the int8 GEMM in `benchmarks/programs/gemm_fabric/main.lg`
/// (0-based here; the `.lg` uses 1-based `item` indexing — same arithmetic).
fn gemm_checksum(n: i64) -> i64 {
    let un = n as usize;
    let mut a = vec![0i64; un * un];
    let mut b = vec![0i64; un * un];
    let mut c = vec![0i64; un * un];
    for i in 0..n {
        for j in 0..n {
            a[(i * n + j) as usize] = (i * n + j) & 127;
            b[(i * n + j) as usize] = (j * n + i) & 127;
        }
    }
    for i in 0..n {
        for k in 0..n {
            for j in 0..n {
                c[(i * n + j) as usize] += a[(i * n + k) as usize] * b[(k * n + j) as usize];
            }
        }
    }
    c.iter().sum()
}

/// The int8 GEMM as a self-contained VM program (n inlined, no native args), Show checksum.
fn gemm_src(n: i64) -> String {
    format!(
        "## Main\nLet n be {n}.\nLet a be a new Seq of Int.\nLet b be a new Seq of Int.\nLet c be a new Seq of Int.\nLet i be 0.\nWhile i is less than n:\n    Let j be 0.\n    While j is less than n:\n        Push (i * n + j) & 127 to a.\n        Push (j * n + i) & 127 to b.\n        Push 0 to c.\n        Set j to j + 1.\n    Set i to i + 1.\nSet i to 0.\nWhile i is less than n:\n    Let k be 0.\n    While k is less than n:\n        Let j be 0.\n        While j is less than n:\n            Let idx be i * n + j + 1.\n            Set item idx of c to item idx of c + item (i * n + k + 1) of a * item (k * n + j + 1) of b.\n            Set j to j + 1.\n        Set k to k + 1.\n    Set i to i + 1.\nLet checksum be 0.\nSet i to 1.\nWhile i is at most n * n:\n    Set checksum to checksum + item i of c.\n    Set i to i + 1.\nShow checksum.\n"
    )
}

// ════════════════════════════════════════════════════════════════════════════════════════════════

/// Tier 1 — the MAC dot-product lowers to the reconfigurable loop fabric, emits a real Mul + Add tile,
/// and evaluates byte-identically to the VM across a battery.
#[test]
fn int8_mac_lowers_to_the_fabric_matching_the_vm() {
    let lf = loop_fabric_from_source(DOTMAC_FN, "dotMac");
    assert!(
        body_has_mul(&lf.body) && body_has_add(&lf.body),
        "the MAC body must genuinely emit a Mul tile and an Add tile (not a degraded lowering)"
    );

    for n in [1i64, 2, 3, 4, 8, 12, 16] {
        let got = eval_loop_fabric(&lf, &[n]);
        assert_eq!(
            got,
            dotmac(n),
            "fabric dotMac({n}) diverged from the Rust oracle"
        );
        assert_eq!(
            got.to_string(),
            vm_call_result(DOTMAC_FN, "dotMac", n),
            "fabric dotMac({n}) != VM"
        );
    }
    println!(
        "Tier 1: int8 MAC dot-product lowers to the loop fabric (Mul+Add tiles) ≡ VM ≡ oracle."
    );
}

/// Tier 2 — the MAC datapath `acc + a*b`, placed-and-routed onto the tile array, is certified EQUAL to
/// the abstract datapath for ALL inputs by our own SAT (routed fan-in of the product into the adder).
#[test]
fn int8_mac_placement_is_certified_semantics_preserving() {
    // inputs: In0 = a, In1 = b, In2 = acc.  cell0 = a*b ; cell1 = acc + cell0 ; result = cell1.
    let body = vec![
        FabricOp {
            op: AluOp::Mul,
            a: Src::In(0),
            b: Src::In(1),
        },
        FabricOp {
            op: AluOp::Add,
            a: Src::In(2),
            b: Src::Val(0),
        },
    ];
    const N_IN: usize = 3;
    const RESULT: usize = 1;

    // (a) concrete agreement — also proves the placement RUNS.
    for &(a, b, acc) in &[
        (0i64, 0i64, 0i64),
        (1, 1, 0),
        (5, 7, 3),
        (12, 9, 100),
        (127, 127, 40),
    ] {
        let placed = eval_grid(
            &route_loop_body(&to_cells(&body), N_IN, &[Src::Val(RESULT)]),
            &[a, b, acc],
        );
        let abstract_ = eval_fabric(&body, &[a, b, acc], RESULT);
        assert_eq!(
            placed, abstract_,
            "placed MAC {placed} != datapath {abstract_} at (a{a},b{b},acc{acc})"
        );
        assert_eq!(
            placed,
            acc + a * b,
            "placed MAC {placed} != acc+a*b at (a{a},b{b},acc{acc})"
        );
    }

    // (b) exhaustive certificate: placed grid ≡ datapath for ALL inputs (width 8 covers the int8 product).
    assert!(
        placement_is_certified(&body, N_IN, RESULT, 8),
        "route_loop_body must be certified semantics-preserving for the MAC datapath `acc + a*b`"
    );
    println!("Tier 2: the MAC datapath placed on the tile array is certified ≡ the abstract datapath by our SAT.");
}

/// Tier 3 — the SAME MAC, inlined into the scalar ISA, compiled to bytecode and streamed over the UART
/// protocol into `nexys_soc_scalar.v` under iverilog, reads back ≡ VM (the no-board twin of the Artix-7).
#[test]
fn int8_mac_runs_on_the_soft_core_rtl_matching_the_vm() {
    // Consistency: the inline form equals the function form on the VM.
    const N: i64 = 16;
    let inline = dotmac_inline(N);
    assert_eq!(
        vm_shows(&inline),
        dotmac(N).to_string(),
        "inline MAC != oracle on the VM"
    );
    assert_eq!(
        vm_shows(&inline),
        vm_call_result(DOTMAC_FN, "dotMac", N),
        "inline MAC != dotMac(N) on the VM"
    );

    if !common::require_tool("iverilog") {
        return;
    }

    let sil = root().join("benchmarks/silicon");
    let tmp = std::env::temp_dir().join("logos_gemm_fabric_lock");
    std::fs::create_dir_all(&tmp).expect("create temp dir");

    let sim = tmp.join("nexys_scalar_run.vvp");
    let build = Command::new("iverilog")
        .current_dir(&sil)
        .args(["-g2012", "-s", "tb", "-DSOC_MODULE=nexys_soc_scalar", "-o"])
        .arg(&sim)
        .args([
            "nexys_soc_scalar.v",
            "soft_core_scalar.v",
            "alu.v",
            "tb_nexys_run.v",
        ])
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let words = compile_to_scalar_words(&inline)
        .unwrap_or_else(|e| panic!("MAC scalar encode failed (outside the scalar ISA?): {e}"));
    assert!(
        words.len() <= 64,
        "MAC program {} words exceeds tb_nexys_run.v's 64-word memory",
        words.len()
    );

    let hex = tmp.join("mac.hex");
    std::fs::write(&hex, to_hex(&words)).expect("write hex");
    let run = Command::new("vvp")
        .current_dir(&sil)
        .arg(&sim)
        .arg(format!("+HEX={}", hex.display()))
        .arg(format!("+N={}", words.len()))
        .output()
        .expect("run vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let got = stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("RESULT:"))
        .map(|r| {
            r.split("CYCLES")
                .next()
                .unwrap_or("")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_else(|| panic!("no RESULT line from the SoC sim:\n{stdout}"));

    assert_eq!(
        got,
        vm_shows(&inline),
        "soft-core readback [{got}] != VM — MAC over the wire diverged\n{stdout}"
    );
    println!(
        "Tier 3: the int8 MAC streamed into nexys_soc_scalar.v under iverilog reads back ≡ VM."
    );
}

/// The named artifact is a REAL matrix multiply: the full int8 GEMM matches an independent Rust oracle on
/// the VM across several sizes (the recognizable matmul the demonstrator ships, not just a dot-product).
#[test]
fn int8_gemm_matches_the_oracle_on_the_vm() {
    for n in [2i64, 3, 4, 5, 6] {
        let out = vm_outcome(&gemm_src(n));
        assert!(out.error.is_none(), "GEMM(n={n}): VM error {:?}", out.error);
        assert_eq!(
            out.output.trim(),
            gemm_checksum(n).to_string(),
            "int8 GEMM(n={n}) checksum diverged from the Rust oracle"
        );
    }
    println!("int8 GEMM ≡ an independent Rust oracle on the VM for n = 2..6 — the artifact is a real matmul.");
}

// ── SPEED: spatial parallelism (the dataflow win) + sequential soft-core cycles (the baseline) ──────

fn count_muls(body: &[LoopCell]) -> usize {
    body.iter()
        .filter(|c| matches!(c, LoopCell::Arith { op: AluOp::Mul, .. }))
        .count()
}

/// A `u`-wide unrolled dot-product: each iteration accumulates `u` independent products
/// `Σ_{j<u} (k+j)·(n-k-j)`. The `u` multiplies are independent, so the spatial fabric places `u`
/// multiply tiles that all evaluate in ONE feed-forward wave — `u` MACs per clock.
fn dotmac_unrolled_fn(u: usize) -> String {
    let terms: Vec<String> = (0..u)
        .map(|j| {
            if j == 0 {
                "k * (n - k)".to_string()
            } else {
                format!("(k + {j}) * (n - k - {j})")
            }
        })
        .collect();
    format!(
        "## To dotMacU (n: Int) -> Int:\n    Let mutable s be 0.\n    Repeat for k from 1 to n:\n        Set s to s + {}.\n    Return s.\n",
        terms.join(" + ")
    )
}
fn dotmac_unrolled(u: i64, n: i64) -> i64 {
    (1..=n)
        .map(|k| (0..u).map(|j| (k + j) * (n - k - j)).sum::<i64>())
        .sum()
}

/// Parse `RESULT: <shows>  CYCLES=<n>` (the on-chip cycle counter the soft-core reports).
fn parse_result_and_cycles(stdout: &str) -> Option<(String, i64)> {
    let line = stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("RESULT:"))?;
    let (shows, after) = line.split_once("CYCLES")?;
    let cycles = after
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>();
    Some((
        shows.split_whitespace().collect::<Vec<_>>().join(" "),
        cycles.parse().ok()?,
    ))
}

/// SPEED (spatial) — the dataflow win: an unrolled int8 MAC places `u` multiply tiles that evaluate in a
/// single feed-forward wave (`u` MACs/clock), proven ≡ VM, and emits synthesizable datapath RTL with a
/// hardware multiply. The multiply-count scales 1→8 with the unroll (spatial parallelism, not sequential).
#[test]
fn int8_mac_spatial_parallelism_scales_and_emits_datapath_rtl() {
    for u in [1usize, 2, 4, 8] {
        let src = dotmac_unrolled_fn(u);
        let lf = loop_fabric_from_source(&src, "dotMacU");
        let muls = count_muls(&lf.body);
        assert_eq!(
            muls, u,
            "unroll×{u} must place {u} multiply tiles (spatial MACs/clock), got {muls}"
        );

        // ≡ VM ≡ oracle across a battery.
        for n in [1i64, 4, 8, 16] {
            let got = eval_loop_fabric(&lf, &[n]);
            assert_eq!(
                got,
                dotmac_unrolled(u as i64, n),
                "unroll×{u} dotMac({n}) != oracle"
            );
            assert_eq!(
                got.to_string(),
                vm_call_result(&src, "dotMacU", n),
                "unroll×{u} dotMac({n}) != VM"
            );
        }

        // The placed spatial datapath — its size (rows = combinational depth, cols = width).
        let n_in = lf.n_state + 1 + lf.invariants.len();
        let grid = route_loop_body(&lf.body, n_in, &lf.next);
        // Energy estimate: scaled from the measured signoff-clean decision_hft anchor (17 compute tiles → 1.19 mW).
        let compute_tiles = lf.body.len();
        let est_mw = (compute_tiles as f64 / 17.0) * 1.19;
        println!(
            "  unroll×{u}: {u} MACs/clock, {compute_tiles} compute-tiles, grid {}×{} ({} channels), est {est_mw:.2} mW",
            grid.rows, grid.cols, grid.used_channels()
        );
    }

    // The 8-wide datapath emits synthesizable RTL with hardware multiplies.
    let lf8 = loop_fabric_from_source(&dotmac_unrolled_fn(8), "dotMacU");
    let rtl = emit_loop_fabric_verilog("gemm_mac8", &lf8);
    assert!(
        rtl.contains("module cgra_gemm_mac8"),
        "the datapath emits a named CGRA module"
    );
    assert!(
        rtl.matches('*').count() >= 8,
        "the 8-wide MAC datapath must emit ≥8 hardware multiplies"
    );
    assert!(rtl.contains("endmodule"), "complete synthesizable module");
    println!("SPEED: spatial int8 MAC scales 1→8 MACs/clock (proven ≡ VM), emits synthesizable datapath RTL.");
}

/// SPEED (baseline) — the SEQUENTIAL cost the fabric beats: the same MAC on the scalar soft-core RTL,
/// measured by the on-chip cycle counter under iverilog. One multiply per several sequential instructions,
/// vs the fabric's many-multiplies-per-wave. iverilog-gated.
#[test]
fn int8_mac_sequential_soft_core_cycle_cost_is_measured() {
    const N: i64 = 16;
    let inline = dotmac_inline(N);

    if !common::require_tool("iverilog") {
        return;
    }
    let sil = root().join("benchmarks/silicon");
    let tmp = std::env::temp_dir().join("logos_gemm_fabric_speed");
    std::fs::create_dir_all(&tmp).expect("create temp dir");
    let sim = tmp.join("nexys_scalar_run.vvp");
    let build = Command::new("iverilog")
        .current_dir(&sil)
        .args(["-g2012", "-s", "tb", "-DSOC_MODULE=nexys_soc_scalar", "-o"])
        .arg(&sim)
        .args([
            "nexys_soc_scalar.v",
            "soft_core_scalar.v",
            "alu.v",
            "tb_nexys_run.v",
        ])
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let words = compile_to_scalar_words(&inline).expect("scalar encode");
    let hex = tmp.join("mac.hex");
    std::fs::write(&hex, to_hex(&words)).expect("write hex");
    let run = Command::new("vvp")
        .current_dir(&sil)
        .arg(&sim)
        .arg(format!("+HEX={}", hex.display()))
        .arg(format!("+N={}", words.len()))
        .output()
        .expect("run vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let (shows, cycles) =
        parse_result_and_cycles(&stdout).unwrap_or_else(|| panic!("no RESULT/CYCLES:\n{stdout}"));
    assert_eq!(
        shows,
        dotmac(N).to_string(),
        "soft-core MAC result != oracle"
    );
    assert!(
        cycles > 0,
        "the on-chip cycle counter must report a positive count"
    );
    println!("SPEED baseline: the sequential soft-core computes dotMac({N}) in {cycles} cycles (on-chip counter).");
}
