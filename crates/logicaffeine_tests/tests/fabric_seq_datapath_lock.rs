//! fabric_seq_datapath_lock — the LATENCY-INSENSITIVE fabric: multi-cycle (variable-latency) TILES, then a
//! whole program (loop + division + float) unified on ONE spatial fabric.
//!
//! Phase A: `emit_seq_datapath_verilog` sequences a straight-line decision fabric — a `Func` DIVISION cell
//! issues the multi-cycle `shared_divider` and stalls until `done` (a variable-latency tile), Arith/Select
//! settle in one step.
//!
//! Phase B+C: `emit_seq_program_verilog` is the whole-program fabric — per-block sequenced datapaths (with the
//! fpu.v FPU tiles: fp_addsub/mul/div/sqrt + the i64↔f64 converters, AND the integer divider) driven by a
//! block-PC FSM that starts each block, waits its `done`, latches the register writes, and takes the terminator
//! (Jump/Branch/Return). So a loop that DIVIDES and does IEEE-754 FLOAT runs on ONE fabric ≡ the VM.
//!
//! Proof: the emitted Verilog (with the real `shared_divider.v` + `fpu.v`) computes ≡ the VM under iverilog.
//! Run: `cargo test -p logicaffeine-tests --test fabric_seq_datapath_lock`

#[path = "common/mod.rs"]
mod common;
#[path = "common/golden.rs"]
mod golden;

use logicaffeine_compile::codegen_sva::fabric_grid::run_program_cfg_on_grid;
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    decision_fabric_from_source, emit_seq_datapath_testbench, emit_seq_datapath_verilog,
    emit_seq_program_testbench, emit_seq_program_verilog, eval_decision_fabric, eval_seq_program,
    program_cfg_fabric, try_emit_seq_datapath_verilog, try_emit_seq_program_verilog, CfgBlock,
    DecisionFabric, FuncUnit, LoopCell, LoweringError, ProgramCfgFabric, Src, Term,
};
use logicaffeine_compile::compile::vm_outcome;

const DIV_FN: &str = "\
## To score (bid: Int, ask: Int, sz: Int) -> Int:
    Let num be bid * sz + ask.
    Let den be sz + 1.
    Return num / den + num % den.
";

/// pi_leibniz — a LOOP that does IEEE-754 float (add/sub/mul/div) + int→float promotion + floor, returning an Int.
const PI_FN: &str = "\
## To pislice (n: Int) -> Int:
    Let mutable sum be 0.0.
    Let mutable sign be 1.0.
    Let mutable k be 0.
    While k is less than n:
        Set sum to sum + sign / (2.0 * k + 1.0).
        Set sign to 0.0 - sign.
        Set k to k + 1.
    Return floor(sum * 4.0 * 1000000.0).
";

/// THE UNIFICATION — ONE program, ONE fabric: a LOOP doing IEEE-754 FLOAT (add/sub/mul/div + int→float) each
/// iteration, then FLOORING to an Int AND adding an INTEGER DIVISION + REMAINDER. So the fpu.v FPU tiles AND the
/// shared_divider tile both run as variable-latency tiles in one whole program (fixed by clearing the stale
/// float tag on the `floor` result — an integer-producing op must retract a reused reg's prior float mark).
const UNIFIED_FN: &str = "\
## To blend (n: Int) -> Int:
    Let mutable sum be 0.0.
    Let mutable sign be 1.0.
    Let mutable k be 0.
    While k is less than n:
        Set sum to sum + sign / (2.0 * k + 1.0).
        Set sign to 0.0 - sign.
        Set k to k + 1.
    Return floor(sum * 4000000.0) + n / 3 + n % 7.
";

/// A mandelbrot escape kernel with the genuine IEEE-754 float COMPARE `zr² + zi² > 4.0` (FGtF → the shared
/// combinational `fp_cmp` tile) driving a conditional Return inside a loop — proves the float-compare tile.
const MANDEL_FN: &str = "\
## To mandel (px: Int) -> Int:
    Let cr be 0.01 * px - 2.0.
    Let ci be 0.5.
    Let mutable zr be 0.0.
    Let mutable zi be 0.0.
    Let mutable i be 0.
    While i is less than 50:
        Let zr2 be zr * zr - zi * zi + cr.
        Let zi2 be 2.0 * zr * zi + ci.
        Set zr to zr2.
        Set zi to zi2.
        If zr * zr + zi * zi is greater than 4.0:
            Return i.
        Set i to i + 1.
    Return i.
";

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

fn sil_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon")
}

/// iverilog a module + testbench (linking shared_divider.v + fpu.v) and assert its stdout contains "PASS:".
fn run_iverilog(dir: &std::path::Path, module: &str, tb: &str, tag: &str) {
    let modv = dir.join(format!("{tag}.v"));
    let tbv = dir.join(format!("tb_{tag}.v"));
    let sim = dir.join(format!("sim_{tag}.vvp"));
    std::fs::write(&modv, module).expect("write module");
    std::fs::write(&tbv, tb).expect("write tb");
    let sil = sil_dir();
    let divider = sil.join("shared_divider.v");
    let fpu = sil.join("fpu.v");
    assert!(
        divider.exists() && fpu.exists(),
        "missing shared_divider.v / fpu.v under {sil:?}"
    );
    let build = std::process::Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&sim)
        .arg(&modv)
        .arg(&tbv)
        .arg(&divider)
        .arg(&fpu)
        .current_dir(dir)
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog failed to compile {tag}:\n{}\n--- RTL ---\n{module}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(dir)
        .output()
        .expect("run vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("PASS:"),
        "{tag} diverged from the VM under iverilog:\n{stdout}"
    );
}

// ── Phase A: a straight-line decision fabric with the multi-cycle DIVIDER tile ──────────────────────────────

#[test]
fn the_division_kernel_lowers_to_multicycle_divider_tiles() {
    let df = decision_fabric_from_source(DIV_FN, "score");
    assert!(
        df.cells.iter().any(|c| matches!(
            c,
            LoopCell::Func {
                unit: FuncUnit::DivS,
                ..
            }
        )),
        "no DivS tile"
    );
    assert!(
        df.cells.iter().any(|c| matches!(
            c,
            LoopCell::Func {
                unit: FuncUnit::ModS,
                ..
            }
        )),
        "no ModS tile"
    );
    for v in [[10i64, 3, 7], [100, 50, 4], [0, 5, 0], [-40, 7, 3]] {
        assert_eq!(
            eval_decision_fabric(&df, &v),
            vm_call(DIV_FN, "score", &v),
            "eval != VM on {v:?}"
        );
    }
}

#[test]
fn the_sequenced_divider_fabric_matches_vm_under_iverilog() {
    let df = decision_fabric_from_source(DIV_FN, "score");
    let vectors: Vec<[i64; 3]> = vec![
        [10, 3, 7],
        [100, 50, 4],
        [0, 5, 0],
        [9, 9, 1],
        [-40, 7, 3],
        [20000, 20000, 250],
    ];
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = std::env::temp_dir().join("logos_fabric_seq_iverilog");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let mut in_hex = String::new();
    let mut out_hex = String::new();
    for v in &vectors {
        for &x in v.iter() {
            in_hex.push_str(&format!("{:016x}\n", x as u64));
        }
        out_hex.push_str(&format!("{:016x}\n", vm_call(DIV_FN, "score", v) as u64));
    }
    std::fs::write(dir.join("seq_score_in.hex"), in_hex).expect("in");
    std::fs::write(dir.join("seq_score_out.hex"), out_hex).expect("out");
    run_iverilog(
        &dir,
        &emit_seq_datapath_verilog("score", &df),
        &emit_seq_datapath_testbench("score", 3, vectors.len()),
        "seq_score",
    );
    println!(
        "SEQUENCED FABRIC — DIVISION: multi-cycle shared_divider tile ran ≡ VM under iverilog"
    );
}

// ── Phase B+C: the WHOLE-PROGRAM sequenced fabric — loop + division + IEEE-754 float on one fabric ───────────

/// Lower a whole function via `program_cfg_fabric`, prove the model (`run_program_cfg_on_grid`) matches the VM,
/// then prove the emitted whole-program sequenced fabric (with real fpu.v + shared_divider.v) matches the VM
/// under iverilog.
fn prove_seq_program_eq_vm(tag: &str, fn_src: &str, fn_name: &str, vectors: &[i64]) {
    let pf = program_cfg_fabric(fn_src, fn_name);
    // The sequenced fabric's OWN model (block-walk + shared heap) is the oracle — faithful to emit_seq_program
    // and general enough for looping heap bodies (unlike the grid model, which cannot place them).
    for &v in vectors {
        assert_eq!(
            eval_seq_program(&pf, &[v]),
            vm_call(fn_src, fn_name, &[v]),
            "seq model != VM on {v}"
        );
    }
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = std::env::temp_dir().join("logos_fabric_seq_iverilog");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let mut in_hex = String::new();
    let mut out_hex = String::new();
    for &v in vectors {
        in_hex.push_str(&format!("{:016x}\n", v as u64));
        out_hex.push_str(&format!("{:016x}\n", vm_call(fn_src, fn_name, &[v]) as u64));
    }
    std::fs::write(dir.join(format!("seqprog_{tag}_in.hex")), in_hex).expect("in");
    std::fs::write(dir.join(format!("seqprog_{tag}_out.hex")), out_hex).expect("out");
    let module = emit_seq_program_verilog(tag, &pf);
    let tb = emit_seq_program_testbench(tag, pf.n_params, vectors.len());
    run_iverilog(&dir, &module, &tb, &format!("seqprog_{tag}"));
}

#[test]
fn the_pi_leibniz_loop_lowers_with_float_tiles() {
    // Non-vacuity + model: the loop lowers to blocks whose bodies carry the FPU tiles (FAddF/FSubF/FMulF/FDivF)
    // and int→float, and the model already matches the VM.
    let pf = program_cfg_fabric(PI_FN, "pislice");
    let has = |u: FuncUnit| {
        pf.blocks.iter().any(|b| {
            b.body
                .iter()
                .any(|c| matches!(c, LoopCell::Func { unit, .. } if *unit == u))
        })
    };
    assert!(
        has(FuncUnit::FDivF) && has(FuncUnit::FAddF) && has(FuncUnit::FMulF),
        "missing FPU tiles: {pf:?}"
    );
    for n in [0i64, 1, 2, 5, 10] {
        assert_eq!(
            run_program_cfg_on_grid(&pf, &[n]),
            vm_call(PI_FN, "pislice", &[n]),
            "model != VM at n={n}"
        );
    }
}

#[test]
fn the_whole_program_float_loop_matches_vm_under_iverilog() {
    // A LOOP doing IEEE-754 float on the whole-program sequenced fabric (fp_addsub/mul/div tiles + i64→f64 +
    // floor), ≡ the VM under iverilog. (pi_leibniz to a floored Int.)
    prove_seq_program_eq_vm("pi", PI_FN, "pislice", &[0, 1, 2, 5, 10]);
    println!("WHOLE-PROGRAM FABRIC — FLOAT LOOP: pi_leibniz ran on multi-cycle fpu.v tiles ≡ VM under iverilog");
}

#[test]
fn the_unified_loop_division_and_float_matches_vm_under_iverilog() {
    // THE UNIFICATION: one fabric, one loop, that DIVIDES an integer AND does IEEE-754 FLOAT (sqrt/div/add) each
    // iteration, then floors and adds an integer division — division + float + control flow ≡ the VM.
    let pf = program_cfg_fabric(UNIFIED_FN, "blend");
    let has = |u: FuncUnit| {
        pf.blocks.iter().any(|b| {
            b.body
                .iter()
                .any(|c| matches!(c, LoopCell::Func { unit, .. } if *unit == u))
        })
    };
    assert!(has(FuncUnit::FDivF), "no float FDivF tile");
    assert!(has(FuncUnit::FAddF), "no float FAddF tile");
    assert!(has(FuncUnit::FFloorF), "no FFloorF tile");
    assert!(has(FuncUnit::DivS), "no integer DivS tile");
    assert!(has(FuncUnit::ModS), "no integer ModS tile");
    prove_seq_program_eq_vm("blend", UNIFIED_FN, "blend", &[0, 1, 2, 3, 5, 8, 10]);
    println!("WHOLE-PROGRAM FABRIC — LOOP+FLOAT+DIVISION (unified): one program with fpu.v tiles AND the shared_divider tile ran ≡ VM under iverilog");
}

#[test]
fn the_float_compare_loop_matches_vm_under_iverilog() {
    // A loop whose escape test is a genuine IEEE-754 float COMPARE (`zr²+zi² > 4.0` → FGtF → the shared fp_cmp
    // tile), driving a conditional Return — proves float compares run as variable-latency tiles ≡VM.
    let pf = program_cfg_fabric(MANDEL_FN, "mandel");
    let has = |u: FuncUnit| {
        pf.blocks.iter().any(|b| {
            b.body
                .iter()
                .any(|c| matches!(c, LoopCell::Func { unit, .. } if *unit == u))
        })
    };
    assert!(has(FuncUnit::FGtF), "no float FGtF compare tile in {pf:?}");
    prove_seq_program_eq_vm("mandel", MANDEL_FN, "mandel", &[0, 50, 100, 250]);
    println!("WHOLE-PROGRAM FABRIC — FLOAT COMPARE (mandelbrot): the escape test zr²+zi²>4.0 ran on the fp_cmp tile ≡ VM under iverilog");
}

#[test]
fn a_division_only_fabric_instantiates_no_fpu() {
    // Pit of success: the fabric emits ONLY the units it uses. A division-only program must synthesize with NO
    // fpu.v units (fp_addsub/mul/div/sqrt/cmp) — else the "emit only used tiles" invariant has silently broken.
    let pf = program_cfg_fabric(DIV_FN, "score");
    let rtl =
        logicaffeine_compile::codegen_sva::fabric_rtl::emit_seq_program_verilog("scoredbg", &pf);
    assert!(
        rtl.contains("shared_divider"),
        "a division fabric must instantiate the shared_divider"
    );
    for fpu in ["fp_addsub", "fp_mul", "fp_div ", "fp_sqrt", "fp_cmp"] {
        assert!(
            !rtl.contains(fpu),
            "a division-only fabric must NOT instantiate {fpu} (emit-only-used-units broke)"
        );
    }
    // And a float fabric DOES instantiate the FPU (non-vacuity).
    let pff = program_cfg_fabric(PI_FN, "pislice");
    let rtlf =
        logicaffeine_compile::codegen_sva::fabric_rtl::emit_seq_program_verilog("pidbg", &pff);
    assert!(
        rtlf.contains("fp_addsub") && rtlf.contains("fp_div"),
        "a float fabric must instantiate the fpu.v units"
    );
}

// ── Robustness: the whole-program lowering is VM-exact over a broad program fuzz (rule 10) ───────────────────
// A diverse battery of float / division / mixed / compare programs, each over many inputs, all ≡ the VM at the
// model layer (`run_program_cfg_on_grid`, which the sequenced-fabric RTL is proven to match). This is the wide
// net that would have caught the stale-float-tag boundary bug across every mixing shape, not just one program.

#[test]
fn the_whole_program_lowering_is_vm_exact_over_a_program_fuzz() {
    let programs: &[(&str, &str)] = &[
        // integer division + remainder loop
        ("f", "## To f (n: Int) -> Int:\n    Let mutable a be 0.\n    Let mutable i be 1.\n    While i is less than or equal to n:\n        Set a to a + n / i + n % i.\n        Set i to i + 1.\n    Return a.\n"),
        // float loop → floored Int
        ("f", "## To f (n: Int) -> Int:\n    Let mutable s be 0.0.\n    Let mutable k be 0.\n    While k is less than n:\n        Set s to s + 1.0 / (k * 1.0 + 1.0).\n        Set k to k + 1.\n    Return floor(s * 1000.0).\n"),
        // MIXED: float-floored Int + integer division + remainder (the fixed boundary)
        ("f", "## To f (n: Int) -> Int:\n    Let mutable s be 0.0.\n    Let mutable sign be 1.0.\n    Let mutable k be 0.\n    While k is less than n:\n        Set s to s + sign / (2.0 * k + 1.0).\n        Set sign to 0.0 - sign.\n        Set k to k + 1.\n    Return floor(s * 4000000.0) + n / 3 + n % 7.\n"),
        // float COMPARE driving a conditional return
        ("f", "## To f (n: Int) -> Int:\n    Let mutable x be 0.5.\n    Let mutable c be 0.\n    Let mutable i be 0.\n    While i is less than n:\n        If x is greater than 3.0:\n            Set c to c + 1.\n        Set x to x + 0.4.\n        Set i to i + 1.\n    Return c.\n"),
        // sqrt + division + floor, no loop
        ("f", "## To f (n: Int) -> Int:\n    Let x be n * 1.0.\n    Return floor(sqrt(x) * 100.0) + n / 4.\n"),
        // negative floor + division
        ("f", "## To f (n: Int) -> Int:\n    Let x be n * 1.0.\n    Return floor(0.0 - x / 3.0) + n % 5.\n"),
        // reuse a reg across float then int (the register-reuse hazard)
        ("f", "## To f (n: Int) -> Int:\n    Let a be n * 2.5.\n    Let b be floor(a).\n    Return b + n / 2 - n % 3.\n"),
    ];
    let inputs: [i64; 12] = [0, 1, 2, 3, 4, 5, 7, 8, 10, 13, 20, 40];
    let mut checked = 0;
    for (name, src) in programs {
        let pf = program_cfg_fabric(src, name);
        for &v in &inputs {
            assert_eq!(
                run_program_cfg_on_grid(&pf, &[v]),
                vm_call(src, name, &[v]),
                "fabric lowering != VM on {name}({v}) for program:\n{src}"
            );
            checked += 1;
        }
    }
    println!("PROGRAM FUZZ: {checked} whole-program fabric evaluations (float/division/mixed/compare) all ≡ VM");
}

// ── Completeness: EVERY FuncUnit is a first-class sequenced tile (no reachable panic) ─────────────────────────
// `exec_ops` can emit Pow (`to the power of`), PopCount, MulHiU (magic-divide), MulOvfS (checked ×), and
// ApproxEqF (`is approximately`) into a decision/program fabric. Each must be a real sequenced-datapath tile —
// Pow via a variable-latency repeat-multiply FSM (usel 4, like the divider), the other four reusing the grid's
// already-iverilog-proven combinational `FuncUnit::verilog()` as `is_comb=1`. This lock builds a single-cell
// fabric per unit and proves the emitted sequenced RTL == `eval_decision_fabric` (== the VM) under iverilog, so
// `seq_cell_arm` can never again panic on a unit `exec_ops` produces.

/// Build a one-cell decision fabric `Func(unit, in0, in1)`, emit its sequenced datapath, and prove it computes
/// `eval_decision_fabric` (the proven oracle) for every vector under iverilog (linking shared_divider.v + fpu.v).
fn prove_seq_unit_eq_eval(name: &str, unit: FuncUnit, vectors: &[[i64; 2]]) {
    let df = DecisionFabric {
        n_in: 2,
        cells: vec![LoopCell::Func {
            unit,
            a: Src::In(0),
            b: Src::In(1),
        }],
        result: Src::Val(0),
        escaped_first_return: false,
    };
    let dir = std::env::temp_dir().join("logos_fabric_seq_iverilog");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let mut in_hex = String::new();
    let mut out_hex = String::new();
    for v in vectors {
        for &x in v.iter() {
            in_hex.push_str(&format!("{:016x}\n", x as u64));
        }
        out_hex.push_str(&format!("{:016x}\n", eval_decision_fabric(&df, v) as u64));
    }
    std::fs::write(dir.join(format!("seq_{name}_in.hex")), in_hex).expect("in");
    std::fs::write(dir.join(format!("seq_{name}_out.hex")), out_hex).expect("out");
    run_iverilog(
        &dir,
        &emit_seq_datapath_verilog(name, &df),
        &emit_seq_datapath_testbench(name, 2, vectors.len()),
        &format!("seq_{name}"),
    );
}

#[test]
fn the_sequenced_datapath_completes_the_multicycle_tile_set() {
    if !common::require_tool("iverilog") {
        return;
    }
    // The SEQUENTIAL power tile — a b-step repeat-multiply FSM (usel 4), variable-latency like the divider. The
    // battery spans a==base, b==0 (→1), b==1, wrapping negative base, and a clamped negative exponent (→1).
    prove_seq_unit_eq_eval(
        "upow",
        FuncUnit::Pow,
        &[
            [2, 10],
            [3, 4],
            [5, 0],
            [7, 1],
            [2, 0],
            [10, 3],
            [-2, 3],
            [2, -5],
            [6, 2],
            [1, 63],
        ],
    );
    // The COMBINATIONAL tiles (reusing the grid's proven `FuncUnit::verilog()` as `is_comb=1; comb_y=…`):
    prove_seq_unit_eq_eval(
        "upopc",
        FuncUnit::PopCount,
        &[
            [0, 0],
            [255, 0],
            [7, 0],
            [-1, 0],
            [1023, 0],
            [1, 0],
            [i64::MAX, 0],
        ],
    );
    prove_seq_unit_eq_eval(
        "umulhi",
        FuncUnit::MulHiU,
        &[
            [0, 5],
            [1 << 40, 1 << 40],
            [-1, -1],
            [1_000_000, 1_000_000],
            [1 << 32, 1 << 32],
        ],
    );
    prove_seq_unit_eq_eval(
        "umulovf",
        FuncUnit::MulOvfS,
        &[
            [2, 3],
            [1 << 40, 1 << 40],
            [i64::MAX, 2],
            [-1, -1],
            [1000, 1000],
            [i64::MIN, 2],
        ],
    );
    prove_seq_unit_eq_eval(
        "uapprox",
        FuncUnit::ApproxEqF,
        &[
            [5, 5],
            [5, 6],
            [1_000_000_000, 1_000_000_000],
            [2_000_000_000, 2_000_000_001],
            [0, 0],
            [7, 9],
        ],
    );
    println!("SEQUENCED TILE SET COMPLETE: Pow (sequential FSM) + MulHiU/MulOvfS/ApproxEqF/PopCount (combinational) all ≡ eval under iverilog");
}

#[test]
fn a_program_with_integer_pow_runs_on_the_sequenced_fabric() {
    // Pow must also work in the whole-program BLOCK-WALK context: alone, MIXED with integer division (both the
    // pow FSM and the divider run as tiles in one fabric), and inside a LOOP body (the block-PC FSM stalls on the
    // pow tile's `done` every iteration). The model layer (`run_program_cfg_on_grid`) is checked first, then the
    // sequenced RTL under iverilog.
    let progs: &[(&str, &str, &[i64])] = &[
        ("cube", "## To cube (n: Int) -> Int:\n    Return n to the power of 3.\n", &[0, 1, 2, 3, 5, 7, 10]),
        ("mix", "## To mix (n: Int) -> Int:\n    Let p be n to the power of 3.\n    Return p + n / 2.\n", &[1, 2, 4, 5, 9, 12]),
        (
            "sumsq",
            "## To sumsq (n: Int) -> Int:\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is less than or equal to n:\n        Set acc to acc + i to the power of 2.\n        Set i to i + 1.\n    Return acc.\n",
            &[0, 1, 2, 3, 4, 6, 10],
        ),
    ];
    for (name, src, inputs) in progs {
        let pf = program_cfg_fabric(src, name);
        for &v in *inputs {
            assert_eq!(
                run_program_cfg_on_grid(&pf, &[v]),
                vm_call(src, name, &[v]),
                "pow model != VM on {name}({v})"
            );
        }
    }
    if !common::require_tool("iverilog") {
        return;
    }
    for (name, src, inputs) in progs {
        prove_seq_program_eq_vm(name, src, name, &inputs.to_vec());
    }
    println!(
        "POW ON THE SEQUENCED FABRIC: cube / mix(pow+div) / sumsq(loop) all ≡ VM under iverilog"
    );
}

// ── Pit of success: the STATE-PLANE boundary is an explicit, tested typed refusal (not a raw panic) ───────────
// The sequenced datapath is a register-only arithmetic fabric (divider + FPU + power + converter tiles). A
// `Mem`/`Heap` cell (any Seq/string program) is the state-plane follow-on — `try_emit_seq_program_verilog`
// returns a typed `LoweringError::StatePlaneUnlowered` rather than panicking, so the frontier is EXPLICIT and
// shrinkable (this test flips the day the sequenced state plane lands). Arithmetic programs must NOT over-refuse.

fn has_heap_cell(pf: &logicaffeine_compile::codegen_sva::fabric_rtl::ProgramCfgFabric) -> bool {
    pf.blocks.iter().any(|b| {
        b.body.iter().any(|c| {
            matches!(
                c,
                LoopCell::HeapRead { .. }
                    | LoopCell::HeapWrite { .. }
                    | LoopCell::HeapAlloc { .. }
                    | LoopCell::MemRead { .. }
            )
        })
    })
}

#[test]
fn the_sequenced_fabric_state_plane_boundary_is_an_explicit_typed_refusal() {
    // Register-only programs (power / float / mixed division) lower to RTL — the boundary never over-refuses.
    for (name, src) in [
        (
            "cube",
            "## To cube (n: Int) -> Int:\n    Return n to the power of 3.\n",
        ),
        ("blend", UNIFIED_FN),
        ("pislice", PI_FN),
    ] {
        let pf = program_cfg_fabric(src, name);
        assert!(
            !has_heap_cell(&pf),
            "{name} must be register-only (no state-plane cells)"
        );
        assert!(
            try_emit_seq_program_verilog(name, &pf).is_ok(),
            "{name} (register-only) must lower, not refuse"
        );
    }
    // GRADUATED (RATCHET-2): a Seq/heap whole-program now LOWERS via the shared state plane — no longer refused.
    let seq_src = "## To firstplus (n: Int) -> Int:\n    Let xs be [n, n + 1, n + 2].\n    Return item 1 of xs.\n";
    let pf = program_cfg_fabric(seq_src, "firstplus");
    assert!(
        has_heap_cell(&pf),
        "non-vacuity: the Seq program must lower to state-plane cells"
    );
    assert!(
        try_emit_seq_program_verilog("firstplus", &pf).is_ok(),
        "a Seq/heap whole-program must now LOWER on the state plane (graduated from the refusal boundary)"
    );
    // The typed-refusal MECHANISM stays live on the two residual frontiers (never a raw panic):
    // (1) the standalone decision datapath is register-only — a heap DecisionFabric belongs on the state-plane
    //     emitter, so `try_emit_seq_datapath_verilog` refuses it.
    let heap_df = DecisionFabric {
        n_in: 1,
        cells: vec![LoopCell::HeapRead { addr: Src::In(0) }],
        result: Src::Val(0),
        escaped_first_return: false,
    };
    match try_emit_seq_datapath_verilog("hd", &heap_df) {
        Err(LoweringError::StatePlaneUnlowered(_)) => {}
        other => panic!("the bare datapath must refuse a heap cell, got {other:?}"),
    }
    // (2) a `MemRead` loop-sequence cell (a distinct scratchpad mechanism) is the residual whole-program frontier.
    let memread_pf = ProgramCfgFabric {
        n_params: 1,
        blocks: vec![CfgBlock {
            inputs: vec![0],
            body: vec![LoopCell::MemRead { addr: Src::In(0) }],
            writes: vec![],
            shows: vec![],
            term: Term::Return(Src::Val(0)),
        }],
    };
    match try_emit_seq_program_verilog("mr", &memread_pf) {
        Err(LoweringError::StatePlaneUnlowered(_)) => {}
        other => {
            panic!("a MemRead program must be a typed StatePlaneUnlowered refusal, got {other:?}")
        }
    }
    println!("STATE-PLANE BOUNDARY: Seq/heap whole-programs GRADUATED to lowering; datapath-heap + MemRead stay typed refusals (no panic)");
}

// ── THE STATE PLANE: a Seq/string whole-program runs on ONE sequenced fabric (shared BRAM heap) ───────────────
// A Seq/string program lowers to a ProgramCfgFabric with HeapAlloc (bump the shared heap pointer) + HeapWrite
// (fill the collection: heap[p]=len, heap[p+1..=p+len]=elements) + HeapRead (`length of xs`=heap[p], `item i`=
// heap[p+i]). The whole-program sequenced fabric now carries a shared async-read register-file HEAP threaded
// across the block-walk (same layout the VM uses), so a program that BUILDS a Seq and READS it runs ≡VM on ONE
// fabric — proven under iverilog with the real state plane. This graduates the state-plane boundary (RATCHET-2).

#[test]
fn a_seq_building_program_runs_on_the_sequenced_fabric_state_plane() {
    // Straight-line Seq programs: build a heap-allocated collection then read length / items back. Each exercises
    // HeapAlloc + HeapWrite (build) + HeapRead (read) on the shared state plane, ≡VM under iverilog.
    let progs: &[(&str, &str, &[i64])] = &[
        ("slen", "## To slen (n: Int) -> Int:\n    Let xs be [n, n + 1, n + 2].\n    Return length of xs.\n", &[1, 5, 9, 100]),
        ("sfirst", "## To sfirst (n: Int) -> Int:\n    Let xs be [n, n + 1, n + 2].\n    Return item 1 of xs.\n", &[0, 1, 5, 42, -7]),
        (
            "ssum",
            "## To ssum (n: Int) -> Int:\n    Let xs be [n, n * 2, n * 3].\n    Return item 1 of xs + item 2 of xs + item 3 of xs.\n",
            &[0, 1, 2, 5, 10, 100],
        ),
        // A STRING is a char-list on the same heap: building "hello" allocs + writes its bytes, `length of` reads
        // heap[ptr]. (Element access on a string yields a CHAR, not an Int — a LOGOS value semantic, not a
        // state-plane limit — so `length of` is the Int-valued string witness.)
        ("strlen", "## To strlen (n: Int) -> Int:\n    Let s be \"hello\".\n    Return length of s + n.\n", &[0, 1, 5, 40]),
        // MUTATION after build: `Set item i of xs to v` is a HeapWrite into a live collection — the state plane
        // must see the update on a later read (sync write, async read). Proves the heap is genuinely mutable.
        (
            "setget",
            "## To setget (n: Int) -> Int:\n    Let mutable xs be [n, n + 1, n + 2].\n    Set item 2 of xs to n * 10.\n    Return item 2 of xs.\n",
            &[0, 3, 5, 9, 40],
        ),
        (
            "setsum",
            "## To setsum (n: Int) -> Int:\n    Let mutable xs be [1, 2, 3, 4].\n    Set item 1 of xs to n.\n    Set item 3 of xs to n + n.\n    Return item 1 of xs + item 2 of xs + item 3 of xs + item 4 of xs.\n",
            &[0, 5, 10, 100],
        ),
    ];
    for (name, src, inputs) in progs {
        prove_seq_program_eq_vm(name, src, name, &inputs.to_vec());
    }
    println!("STATE PLANE: Seq/string-building programs (alloc + fill + read) run ≡VM on ONE sequenced fabric under iverilog");
}

#[test]
fn a_seq_looping_program_runs_on_the_sequenced_fabric_state_plane() {
    // The GENERAL case: a whole program that BUILDS a Seq and LOOPS over it (dynamic-index HeapReads inside a
    // back-edge loop, the heap persisting across every iteration on the shared state plane). This is a Seq/string
    // whole-program on ONE sequenced fabric — control flow + collections + arithmetic unified, ≡VM under iverilog.
    let progs: &[(&str, &str, &[i64])] = &[
        // sum every element of a built Seq via a While + `item i of xs`
        (
            "lsum",
            "## To lsum (n: Int) -> Int:\n    Let xs be [n, n * 2, n * 3, n + 7].\n    Let mutable s be 0.\n    Let mutable i be 1.\n    While i is less than or equal to length of xs:\n        Set s to s + item i of xs.\n        Set i to i + 1.\n    Return s.\n",
            &[0, 1, 3, 5, 10, 25],
        ),
        // max of a built Seq (a conditional inside the loop over heap reads)
        (
            "lmax",
            "## To lmax (n: Int) -> Int:\n    Let xs be [n, n * 3, n - 1, n + 5].\n    Let mutable m be item 1 of xs.\n    Let mutable i be 2.\n    While i is less than or equal to length of xs:\n        If item i of xs is greater than m:\n            Set m to item i of xs.\n        Set i to i + 1.\n    Return m.\n",
            &[0, 2, 5, 10, 100],
        ),
    ];
    for (name, src, inputs) in progs {
        prove_seq_program_eq_vm(name, src, name, &inputs.to_vec());
    }
    println!("STATE PLANE (LOOPING): build-a-Seq-then-loop programs (dynamic-index heap reads across a back-edge) run ≡VM on ONE sequenced fabric under iverilog");
}

/// Generator: the Nexys A7 SoC for the LOOPING Seq program `lsum` (build [n,2n,3n,n+7] on the shared state plane,
/// then loop-sum it), with the VM oracle + the build/flash recipe. This is the whole-program STATE-PLANE fabric
/// bound for real silicon: stream an input `n` in, the fabric builds the Seq in its BRAM heap, loops, and streams
/// the sum back over the UART. A run writes the SoC into a scratch directory and asserts the sequenced fabric's
/// model answers each baked input as the VM does, and that those answers are the goldens
/// `scripts/validate-on-nexys-seqstate.sh` checks the board against. The committed
/// `benchmarks/silicon/nexys_seqprog_lsum.v` feeds `nexys_seqprog_lsum.bit`; `scripts/test/gen-silicon-goldens.sh
/// lsum-soc` writes a new one, which is proven only by rebuilding the bitstream and running that script.
#[test]
#[ignore]
fn generate_lsum_state_plane_soc() {
    let src = "## To lsum (n: Int) -> Int:\n    Let xs be [n, n * 2, n * 3, n + 7].\n    Let mutable s be 0.\n    Let mutable i be 1.\n    While i is less than or equal to length of xs:\n        Set s to s + item i of xs.\n        Set i to i + 1.\n    Return s.\n";
    let ns: [i64; 4] = [1, 3, 5, 8];
    let pf = program_cfg_fabric(src, "lsum");
    let vectors: Vec<Vec<i64>> = ns.iter().map(|&n| vec![n]).collect();
    let soc = logicaffeine_compile::codegen_sva::fabric_rtl::emit_seqprog_soc_verilog(
        "lsum", &pf, &vectors,
    );
    let path = golden::output_dir("benchmarks/silicon", "logos_seqprog_lsum_soc")
        .join("nexys_seqprog_lsum.v");
    std::fs::write(&path, &soc).expect("write SoC");
    let expect: Vec<i64> = ns.iter().map(|&n| vm_call(src, "lsum", &[n])).collect();
    for (&n, &want) in ns.iter().zip(&expect) {
        assert_eq!(
            eval_seq_program(&pf, &[n]),
            want,
            "the sequenced fabric's model != VM on lsum({n})"
        );
    }
    let board =
        std::fs::read_to_string(golden::repo_root().join("scripts/validate-on-nexys-seqstate.sh"))
            .expect("read the board script");
    let goldens = board
        .lines()
        .find_map(|l| l.strip_prefix("EXPECT=\"")?.strip_suffix('"'))
        .expect("the board script's EXPECT line");
    assert_eq!(
        goldens,
        expect.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","),
        "scripts/validate-on-nexys-seqstate.sh checks the board against goldens the VM does not produce"
    );
    // The core RTL must reference the shared heap (state plane) and NOT drag in the divider/FPU (lsum is heap +
    // integer add only) — the SRC list for the open flow is then just the one file.
    assert!(
        soc.contains("reg [63:0] heap [0:63]"),
        "the SoC must instantiate the shared state-plane heap"
    );
    assert!(
        !soc.contains("shared_divider") && !soc.contains("fp_addsub"),
        "lsum needs no divider/FPU modules"
    );
    eprintln!("wrote {path:?}");
    eprintln!("VM oracle (n={ns:?}): {expect:?}");
    eprintln!(
        "build:  TOP=nexys_seqprog_lsum SRC=\"nexys_seqprog_lsum.v\" BIT=nexys_seqprog_lsum.bit ABC9=1 ROUTER=router1 PLACERELAX=1 SEED=3 bash scripts/openxc7-build.sh"
    );
    eprintln!(
        "run:    sudo -n openFPGALoader -c digilent --ftdi-serial 210292BFA310 benchmarks/silicon/nexys_seqprog_lsum.bit && sudo -n python3 scripts/seqprog-host.py --port /dev/ttyUSB3 --baud 9600 --n 8 --expect {}",
        expect.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")
    );
}
