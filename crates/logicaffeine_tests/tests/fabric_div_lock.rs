//! fabric_div_lock — Phase 1 of the spatial-interpreter campaign: DIVISION as a fabric functional unit.
//!
//! The decision/loop fabric lowers a branchy or counted function to a select-DAG / CGRA of `Add`/`Sub`/
//! `Mul` tiles. Real kernels also DIVIDE (a microprice is `(bid·asksz + ask·bidsz) / (bidsz + asksz)`);
//! division is not a combinational ALU op — on silicon it is a sequential `shared_divider` functional
//! unit. This lock pins the lowering's SOUNDNESS: a function containing `/` and `%` lowers to a fabric
//! whose evaluation equals the running VM EXACTLY over a broad seeded fuzz, including the divide-by-zero
//! trap (which the soft-core's divider defines as 0, and the fabric must match).
//!
//! RED before Phase 1: `decision_fabric_from_source` panics in `exec_ops` on `Op::Div` ("unsupported").
//! GREEN: division becomes a `FuncUnit` fabric cell (distinct from the combinational `AluOp`), evaluated
//! as the VM's truncated signed division and lowered to the sequential divider in RTL.

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::fabric_rtl::{
    decision_fabric_from_source, emit_decision_fabric_verilog, eval_decision_fabric, FuncUnit,
    LoopCell,
};
use logicaffeine_compile::compile::vm_outcome;

/// A microprice-style kernel: multiply/add to form a weighted numerator, then DIVIDE by the total size.
/// `num / den` is `Op::Div` (truncating) because `den` is a computed value, not a power-of-two constant.
const MICROPRICE_FN: &str = "\
## To microprice (bid: Int, ask: Int, bidsz: Int, asksz: Int) -> Int:
    Let num be bid * asksz + ask * bidsz.
    Let den be bidsz + asksz.
    Return num / den.
";
const MICROPRICE: &str = "microprice";

/// A modulo kernel (the divider's remainder path): `x % m`.
const WRAP_FN: &str = "\
## To wrapmod (x: Int, m: Int) -> Int:
    Return x % m.
";
const WRAP: &str = "wrapmod";

/// The independent Rust reference for the microprice — truncated signed division, matching `Op::Div`.
fn microprice_ref(bid: i64, ask: i64, bidsz: i64, asksz: i64) -> i64 {
    let num = bid
        .wrapping_mul(asksz)
        .wrapping_add(ask.wrapping_mul(bidsz));
    let den = bidsz.wrapping_add(asksz);
    if den == 0 {
        0
    } else {
        num.wrapping_div(den)
    }
}

fn arg(n: i64) -> String {
    if n < 0 {
        format!("(0 - {})", -n)
    } else {
        n.to_string()
    }
}

fn vm_call(func_src: &str, name: &str, args: &[i64]) -> i64 {
    let call = args.iter().map(|a| arg(*a)).collect::<Vec<_>>().join(", ");
    let src = format!("{func_src}## Main\n    Show {name}({call}).\n");
    let out = vm_outcome(&src);
    assert!(
        out.error.is_none(),
        "VM error on {name}({args:?}): {:?}",
        out.error
    );
    out.output
        .split_whitespace()
        .next()
        .expect("a Show")
        .parse::<i64>()
        .expect("int")
}

/// splitmix64 — dependency-free deterministic fuzz.
fn mix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[test]
fn microprice_vm_matches_reference() {
    // The English microprice runs identically to the independent Rust reference — the spec is pinned.
    let mut st = 0xABCDEF0123456789u64;
    for _ in 0..200 {
        let bid = (mix(&mut st) % 20000) as i64;
        let ask = bid + (mix(&mut st) % 50) as i64;
        let bidsz = 1 + (mix(&mut st) % 500) as i64; // >= 1 so den >= 2
        let asksz = 1 + (mix(&mut st) % 500) as i64;
        assert_eq!(
            vm_call(MICROPRICE_FN, MICROPRICE, &[bid, ask, bidsz, asksz]),
            microprice_ref(bid, ask, bidsz, asksz),
            "VM microprice diverged from reference on ({bid},{ask},{bidsz},{asksz})"
        );
    }
}

#[test]
fn microprice_fabric_matches_vm_over_fuzz() {
    // RED until Phase 1: the fabric panics on Op::Div. GREEN: the FuncUnit divider tile evals ≡ VM.
    let df = decision_fabric_from_source(MICROPRICE_FN, MICROPRICE);
    assert_eq!(df.n_in, 4, "microprice has 4 inputs");
    let mut st = 0x0DDBA11_5EED_u64;
    for _ in 0..500 {
        let bid = (mix(&mut st) % 20000) as i64;
        let ask = bid + (mix(&mut st) % 50) as i64;
        let bidsz = 1 + (mix(&mut st) % 500) as i64;
        let asksz = 1 + (mix(&mut st) % 500) as i64;
        let inputs = [bid, ask, bidsz, asksz];
        let got = eval_decision_fabric(&df, &inputs);
        let want = vm_call(MICROPRICE_FN, MICROPRICE, &inputs);
        assert_eq!(
            got, want,
            "microprice fabric ≠ VM on {inputs:?} (got {got}, want {want})"
        );
    }
}

#[test]
fn division_traps_divide_by_zero_to_zero_like_the_chip() {
    // The soft-core's `shared_divider` defines `b == 0 → 0` (a hardware trap), and the fabric's `FuncUnit`
    // must match that SILICON behavior. (The VM instead ERRORS on `n / 0` — a higher-level guard the bare
    // chip lacks — so this trap parity is checked against the chip's divider, not the VM.)
    let df = decision_fabric_from_source(MICROPRICE_FN, MICROPRICE);
    // bidsz + asksz == 0 → den == 0 → the divider traps the quotient to 0.
    assert_eq!(
        eval_decision_fabric(&df, &[100, 100, 0, 0]),
        0,
        "microprice with den == 0 must trap to 0 like the shared_divider"
    );
    let dm = decision_fabric_from_source(WRAP_FN, WRAP);
    assert_eq!(
        eval_decision_fabric(&dm, &[42, 0]),
        0,
        "x % 0 must trap to 0 like the shared_divider"
    );
}

#[test]
fn modulo_fabric_matches_vm_over_fuzz() {
    // The remainder path of the same divider: `x % m` lowers and evals ≡ VM.
    let df = decision_fabric_from_source(WRAP_FN, WRAP);
    assert_eq!(df.n_in, 2, "wrapmod has 2 inputs");
    let mut st = 0xC0FFEE_1234_u64;
    for _ in 0..500 {
        let x = (mix(&mut st) % 200000) as i64 - 100000;
        let m = 1 + (mix(&mut st) % 999) as i64; // nonzero divisor
        let inputs = [x, m];
        let got = eval_decision_fabric(&df, &inputs);
        let want = vm_call(WRAP_FN, WRAP, &inputs);
        assert_eq!(
            got, want,
            "wrapmod fabric ≠ VM on {inputs:?} (got {got}, want {want})"
        );
    }
}

#[test]
fn microprice_fabric_emits_a_division_functional_unit() {
    // The division lowered to a sequential FUNCTIONAL-UNIT tile (`Func { DivS }`), NOT a combinational
    // `Arith` tile — so the emitter can wire the `shared_divider`, not synthesize a giant combinational
    // divider. The emitted Verilog is a deterministic, tapeable artifact with the `b == 0 → 0` trap guard.
    let df = decision_fabric_from_source(MICROPRICE_FN, MICROPRICE);
    let div_tiles = df
        .cells
        .iter()
        .filter(|c| {
            matches!(
                c,
                LoopCell::Func {
                    unit: FuncUnit::DivS,
                    ..
                }
            )
        })
        .count();
    assert_eq!(
        div_tiles, 1,
        "microprice has exactly one division functional-unit tile, got {div_tiles}"
    );
    let v1 = emit_decision_fabric_verilog("microprice", &df);
    let v2 = emit_decision_fabric_verilog("microprice", &df);
    assert_eq!(
        v1, v2,
        "emitted division Verilog must be deterministic (a reproducible tapeable artifact)"
    );
    assert!(
        v1.contains("module decision_microprice("),
        "emits a named module"
    );
    assert!(
        v1.contains("$signed") && v1.contains(" / "),
        "contains the signed-division divider tile"
    );
    assert!(
        v1.contains("== 0") && v1.contains("64'sd0"),
        "the divider tile carries the b == 0 → 0 trap guard"
    );
}

#[test]
fn microprice_rtl_matches_vm_under_iverilog() {
    // The strong RTL proof: the EMITTED Verilog (the divider tile included) computes ≡ the VM under a
    // real gate-level simulator. Skipped gracefully where iverilog is absent (minimal CI) — the eval and
    // structural guards above are simulator-free and cover CI; this adds the RTL≡VM layer where a
    // simulator exists (this box). Div-by-zero never reaches iverilog: the `b == 0 → 0` guard is combinational.
    if !common::require_tool("iverilog") {
        return;
    }
    let df = decision_fabric_from_source(MICROPRICE_FN, MICROPRICE);
    let module = emit_decision_fabric_verilog("microprice", &df);

    // Deterministic vectors with nonzero den — some large enough to exercise the divider's full range.
    let vectors: Vec<[i64; 4]> = vec![
        [10000, 10005, 3, 7],
        [500, 520, 100, 100],
        [9999, 10001, 1, 250],
        [0, 50, 4, 4],
        [20000, 20000, 250, 250],
        [123, 456, 7, 13],
        [10050, 10050, 2, 500],
        [1, 2, 1, 1],
    ];
    let mut tb = String::from(
        "module tb;\n  reg signed [63:0] in0,in1,in2,in3;\n  wire signed [63:0] result;\n  \
         decision_microprice dut(in0,in1,in2,in3,result);\n  initial begin\n",
    );
    for v in &vectors {
        tb.push_str(&format!(
            "    in0={}; in1={}; in2={}; in3={}; #1 $display(\"%0d\", $signed(result));\n",
            v[0], v[1], v[2], v[3]
        ));
    }
    tb.push_str("    $finish;\n  end\nendmodule\n");

    let dir = std::env::temp_dir().join("logos_fabric_div_iverilog");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let modv = dir.join("microprice.v");
    let tbv = dir.join("tb.v");
    let sim = dir.join("sim.vvp");
    std::fs::write(&modv, &module).expect("write module");
    std::fs::write(&tbv, &tb).expect("write tb");

    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&sim)
        .arg(&modv)
        .arg(&tbv)
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog failed to compile the microprice fabric: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .output()
        .expect("run vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    // vvp prints a `$finish called at ...` banner alongside the `$display` results; keep only the
    // integer result lines (one per vector).
    let got: Vec<i64> = stdout
        .lines()
        .map(str::trim)
        .filter_map(|l| l.parse::<i64>().ok())
        .collect();
    assert_eq!(
        got.len(),
        vectors.len(),
        "expected one integer sim line per vector, got: {stdout:?}"
    );
    for (v, g) in vectors.iter().zip(got.iter()) {
        let want = vm_call(MICROPRICE_FN, MICROPRICE, v);
        assert_eq!(
            *g, want,
            "RTL microprice ≠ VM on {v:?} (rtl {g}, vm {want})"
        );
    }
}
