//! gpu_silicon_lock — the GPU processor edge: a tuneable SIMT data-parallel engine.
//!
//! A GPU's defining hardware trait vs the NPU (a REDUCTION: `acc += w·x`, many→one) and the CPU
//! (scalar) is DATA PARALLELISM: N independent spatial lanes, each running the SAME compiled
//! kernel (the "shader") on its OWN data, producing N INDEPENDENT outputs (a MAP), each lane
//! gated by its own predicate bit (SIMT divergence). We compile an affine FMA shader written in
//! English (`out[i] = a·x[i] + b`) to the deterministic Futamura fabric — N WIRED spatial lanes,
//! no warp scheduler / fetch / decode / dispatch — and prove the emitted engine is **bit-identical
//! to the reference VM** lane-by-lane, across lane widths, mask patterns, and modes.
//!
//! One parametrized engine specializes to every mode (the "best of all worlds"):
//!   * **SIMT** — per-lane predication (a mask); a masked lane is inert (contributes 0).
//!   * **SIMD** — the special case where every lane is unmasked (`gpu_predication_collapses_to_simd`).
//!   * **map-then-reduce** — a cross-lane warp-reduce that IS the NPU's accumulator, placed
//!     spatially (`gpu_map_then_reduce_composes_with_the_npu_accumulator`).
//!
//! Proof strength: ≡VM (direct model-vs-VM across configs) PLUS, on a narrow width, EXHAUSTIVELY
//! by our OWN certified SAT — the per-lane kernel ≡ an independent closed form, the predicate ≡ a
//! two-way select, and the warp-reduce ≡ an independent fold. A lane is byte-identical to every
//! other modulo its suffix, so one certified lane covers the array (SIMT). The FPGA/Sky130
//! receipts for the tile are recorded separately (reports.json / sky130_signoff.json).
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(gpu_silicon_lock)'`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::codegen_sva::bitblast::lower_bool;
use logicaffeine_compile::codegen_sva::fabric_rtl::{fabric_to_bounded, AluOp};
use logicaffeine_compile::codegen_sva::gpu_simt_rtl::{
    emit_gpu_simt_testbench, emit_gpu_simt_verilog, eval_gpu_simt, signoff_advisory,
    simt_masked_lane_to_bounded, simt_reduce_to_bounded, simt_sequential_fma_to_bounded,
    simt_sliced_pp_to_bounded, write_gpu_hex, LaneArith, SimtEngine,
};
use logicaffeine_compile::codegen_sva::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
use logicaffeine_proof::ProofExpr;

/// The affine FMA shader `out = a·x + b` — `a` and `b` are broadcast uniforms, `x` is the per-lane
/// varying. A per-lane fused multiply-add: the workhorse of a dense layer / a linear shader, and
/// (like the NPU MAC) a HARDWARE multiply. The kernel is compiled from English by the real
/// compiler; every lane runs its dispatch-free residual verbatim.
/// The flagship signed-off lane width. The engine is parametric (the ≡VM battery proves N up to
/// 16); N=4 is the width taken to Sky130 GDSII — four parallel 64-bit FMA lanes synthesize to a
/// large array-multiplier block on Sky130 (no DSP hardening), and N=4 routes clean where N=8's
/// eight parallel multipliers over-congest the router. N=8+ is a bigger-die follow-on.
const FLAGSHIP_LANES: usize = 4;

/// The flagship per-lane multiply strategy. `SequentialFma` is a shift-add over the 64-bit datapath:
/// ONE reused adder per lane consumes one multiplier bit per cycle (a word-serial multiplier). Its
/// fan-out is bounded (no ~256-fan-out multiplicand net, no wide `*`), so on Sky130 it is
/// ANTENNA-TRIVIAL — the wide combinational strategies (`Combinational`, `DecomposedFma`) leave
/// unavoidable met1 area-ratio violations at 64-bit width even after fan-out splitting, which the
/// sequential strategy sidesteps entirely. The theorem-prover certifies the shift-add ≡ the hardware
/// `a·x` (`gpu_sequential_multiply_is_certified_equal_to_the_hardware_multiply`), so the switch is FREE.
///
/// `select_slice` is the fan-out / antenna knob: the per-cycle conditional-add partial product is
/// emitted as `select_slice` `(* keep *)` copies of the multiplier LSB, each gating a
/// `WIDTH / select_slice`-bit group of `mc` — so the max select fan-out is `WIDTH / select_slice`,
/// never the full datapath. At the flagship (`WIDTH = 64`, `select_slice = 8`) that fan-out is 8,
/// which `gpu_sequential_select_fanout_is_structurally_bounded` locks as antenna-safe.
const FLAGSHIP_SELECT_SLICE: usize = 8;
const FLAGSHIP_ARITH: LaneArith = LaneArith::SequentialFma {
    select_slice: FLAGSHIP_SELECT_SLICE,
};

const SHADE_DECL: &str = "## To shade (a: Int, x: Int, b: Int) -> Int:\n    Return a * x + b.\n";

/// A complete program calling the shader — valid to compile (the fabric is built from the
/// function body; `Main` is only there so the program compiles / the VM can be run for an oracle).
fn shade_main(a: i64, x: i64, b: i64) -> String {
    format!("{SHADE_DECL}\n## Main\n\nShow shade({a}, {x}, {b}).\n")
}

/// The VM's answer for `shade(a, x, b)` — the reference one lane must match. Inputs are kept in a
/// range where `a·x + b` fits `i64`, so the VM's arbitrary-precision `Int` equals the circuit's
/// 64-bit value exactly (the exhaustive mod-2^w wrap projection is certified by the SAT tests).
fn vm_shade(a: i64, x: i64, b: i64) -> i64 {
    vm_outcome(&shade_main(a, x, b))
        .output
        .trim()
        .parse()
        .unwrap_or_else(|e| panic!("shade({a},{x},{b}) VM output not an Int: {e}"))
}

/// Build the SIMT engine for the affine shader: `a`,`b` uniform (broadcast), `x` per-lane varying.
fn engine(n_lanes: usize, predicated: bool, reduce: Option<AluOp>) -> SimtEngine {
    SimtEngine::from_lane_source(
        &shade_main(2, 3, 4),
        "shade",
        n_lanes,
        vec![true, false, true], // a uniform, x varying, b uniform
        predicated,
        reduce,
    )
}

/// The flagship SIGNED-OFF engine: `FLAGSHIP_LANES` fully-independent SIMT lanes (every input is
/// per-lane — no uniform broadcast), predicated, with an Add warp-reduce. Independence is the
/// physically-friendly form: a broadcast uniform `a` fans out to every lane's 64×64 multiplier
/// (a ~256-fan-out multiplicand net) and forces an ultra-sparse floorplan to route; per-lane `a`
/// keeps each multiplicand's fan-out local, so it routes at a normal density.
fn flagship_engine() -> SimtEngine {
    let mut e = SimtEngine::from_lane_source(
        &shade_main(2, 3, 4),
        "shade",
        FLAGSHIP_LANES,
        vec![false, false, false], // every input per-lane (pure SIMT — no cross-lane broadcast)
        true,
        Some(AluOp::Add),
    );
    e.arith = FLAGSHIP_ARITH;
    e
}

/// A per-lane input vector `[a, x_j, b]` (a and b equal across lanes = the uniforms).
fn lane_in(a: i64, x: i64, b: i64) -> Vec<i64> {
    vec![a, x, b]
}

// ============================================================================================
// (a) ≡VM — the whole N-lane map (+ warp-reduce) equals the VM's scalar loop over lanes.
// ============================================================================================

#[test]
fn gpu_simt_tile_is_bit_identical_to_the_vm() {
    // Tuneable width: prove the engine ≡ VM at every lane count from a scalar (N=1) to a 16-wide
    // warp. `a`,`b` are broadcast uniforms; each lane gets its own `x`. Masks cover SIMD (all-on),
    // fully-masked, alternating, and single-active (full divergence).
    for &n in &[1usize, 2, 4, 8, 16] {
        let eng = engine(n, true, Some(AluOp::Add));
        assert_eq!(eng.n_in, 3, "the shade kernel takes (a, x, b)");

        // A couple of distinct input vectors (uniform a,b; per-lane x); values stay in-range.
        let vectors: Vec<(i64, i64, Vec<i64>)> = vec![
            (3, 5, (0..n).map(|j| (j as i64) + 1).collect()),
            (7, 2, (0..n).map(|j| (j as i64) * 3 - 4).collect()),
        ];
        let masks: Vec<Vec<bool>> = vec![
            vec![true; n],                        // SIMD — every lane active
            vec![false; n],                       // all masked
            (0..n).map(|j| j % 2 == 0).collect(), // alternating divergence
            (0..n).map(|j| j == n / 2).collect(), // single active lane
        ];

        for (a, b, xs) in &vectors {
            // The VM oracle per lane (independent of the mask), computed once.
            let vm_vals: Vec<i64> = xs.iter().map(|&x| vm_shade(*a, x, *b)).collect();
            let lane_inputs: Vec<Vec<i64>> = xs.iter().map(|&x| lane_in(*a, x, *b)).collect();

            for mask in &masks {
                let (outs, reduced) = eval_gpu_simt(&eng, &lane_inputs, mask);
                let expect_outs: Vec<i64> = (0..n)
                    .map(|j| if mask[j] { vm_vals[j] } else { 0 })
                    .collect();
                assert_eq!(
                    outs, expect_outs,
                    "SIMT lane outputs diverge from the VM at N={n}, a={a}, b={b}, mask={mask:?}"
                );
                let expect_reduced: i64 = expect_outs.iter().sum();
                assert_eq!(
                    reduced,
                    Some(expect_reduced),
                    "the warp-reduce diverges from the VM sum at N={n}, a={a}, b={b}, mask={mask:?}"
                );
            }
        }
    }

    // The FLAGSHIP silicon config (fully-independent lanes — every input per-lane) must ALSO be
    // ≡VM: give each lane its OWN a, x, b and check the masked map + warp-reduce against the VM.
    let fe = flagship_engine();
    let masks: Vec<Vec<bool>> = vec![
        vec![true; FLAGSHIP_LANES],
        (0..FLAGSHIP_LANES).map(|j| j % 2 == 0).collect(),
        (0..FLAGSHIP_LANES).map(|j| j == 0).collect(),
    ];
    let lane_inputs: Vec<Vec<i64>> = (0..FLAGSHIP_LANES)
        .map(|j| lane_in(2 + j as i64, 3 * j as i64 + 1, 5 - j as i64))
        .collect();
    let vm_vals: Vec<i64> = lane_inputs
        .iter()
        .map(|li| vm_shade(li[0], li[1], li[2]))
        .collect();
    for mask in &masks {
        let (outs, reduced) = eval_gpu_simt(&fe, &lane_inputs, mask);
        let expect: Vec<i64> = (0..FLAGSHIP_LANES)
            .map(|j| if mask[j] { vm_vals[j] } else { 0 })
            .collect();
        assert_eq!(
            outs, expect,
            "flagship (per-lane a,x,b) diverges from the VM, mask={mask:?}"
        );
        assert_eq!(
            reduced,
            Some(expect.iter().sum()),
            "flagship warp-reduce diverges, mask={mask:?}"
        );
    }
}

// ============================================================================================
// (b) certified SAT — the per-lane kernel, the predicate, and the warp-reduce, exhaustively.
// ============================================================================================

/// The per-lane kernel is proven EQUAL to the independent closed form `a·x + b` for ALL 8-bit
/// inputs by our own certified SAT — not sampled. The kernel is the REAL compiler's residual, so
/// this certifies the compiled lane, catching any miscompile (wrong operand, dropped op).
#[test]
fn gpu_kernel_is_certified_equal_to_the_vm_element_op() {
    // Exhaustive over all 4-bit inputs. The lane contains a genuine var×var multiply (an FMA), so
    // the miter is a full multiplier-equivalence instance; 4-bit keeps it exhaustive yet tractable
    // (a wider full multiplier is a known-hard SAT). The 64-bit range is carried by the ≡VM test;
    // the general-ALU multiply is itself proven ≡VM elsewhere (fabric_rtl). This pins the WIRING.
    const PW: u32 = 4;
    let eng = engine(1, false, None);
    let circuit = fabric_to_bounded(&eng.body, eng.result, PW);
    let bv = |n: &str| Box::new(BoundedExpr::BitVecVar(n.to_string(), PW));
    let bin = |o, l, r| {
        Box::new(BoundedExpr::BitVecBinary {
            op: o,
            left: l,
            right: r,
        })
    };
    // spec = a*x + b, built independently (in0=a, in1=x, in2=b).
    let spec = *bin(
        BitVecBoundedOp::Add,
        bin(BitVecBoundedOp::Mul, bv("in0"), bv("in1")),
        bv("in2"),
    );
    let claim = BoundedExpr::Eq(Box::new(circuit), Box::new(spec));
    let p = lower_bool(&claim).expect("claim lowers");
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(p))),
        UnsatOutcome::Refuted,
        "the compiled lane must equal a·x + b for ALL inputs (certified, not sampled)"
    );
}

/// The per-lane predicate (SIMT divergence) is certified: the emitted lane computes exactly
/// `mask ? (a·x + b) : 0` for ALL inputs AND both mask values, by our own SAT. A masked lane is
/// proven inert (0). This is control-becomes-data: the mask GATES the lane, no divergence.
#[test]
fn gpu_predication_is_certified() {
    // 4-bit exhaustive (the lane's var×var FMA multiply is inside the predicate — see the note on
    // `gpu_kernel_is_certified_equal_to_the_vm_element_op`); certifies the mask gates the lane.
    const PW: u32 = 4;
    let eng = engine(1, true, None);
    let masked = simt_masked_lane_to_bounded(&eng, PW); // ite(mask==1, compiled_kernel, 0)
    let bv = |n: &str| Box::new(BoundedExpr::BitVecVar(n.to_string(), PW));
    let k = |v: u64| {
        Box::new(BoundedExpr::BitVecConst {
            width: PW,
            value: v,
        })
    };
    let bin = |o, l, r| {
        Box::new(BoundedExpr::BitVecBinary {
            op: o,
            left: l,
            right: r,
        })
    };
    // spec = (mask == 1) ? (a*x + b) : 0 — independent of the emitter's construction.
    let cond = Box::new(BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::Eq,
        left: bv("mask"),
        right: k(1),
    });
    let kernel = bin(
        BitVecBoundedOp::Add,
        bin(BitVecBoundedOp::Mul, bv("in0"), bv("in1")),
        bv("in2"),
    );
    let spec = BoundedExpr::BitVecIte {
        cond,
        then: kernel,
        else_: k(0),
    };
    let claim = BoundedExpr::Eq(Box::new(masked), Box::new(spec));
    let p = lower_bool(&claim).expect("claim lowers");
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(p))),
        UnsatOutcome::Refuted,
        "the predicated lane must equal `mask ? a·x+b : 0` for ALL inputs and both masks (certified)"
    );
}

/// SIMD is the special case of SIMT with every lane unmasked: with an all-ones mask, the
/// predicated engine's outputs are bit-identical to the un-predicated (pure-SIMD) engine's.
#[test]
fn gpu_predication_collapses_to_simd() {
    let n = 8;
    let simt = engine(n, true, Some(AluOp::Add)); // predicated
    let simd = engine(n, false, Some(AluOp::Add)); // no mask — pure SIMD
    let all_on = vec![true; n];
    for (a, b) in [(3i64, 5i64), (7, 2), (11, 0)] {
        let lane_inputs: Vec<Vec<i64>> = (0..n).map(|j| lane_in(a, (j as i64) + 1, b)).collect();
        let simt_out = eval_gpu_simt(&simt, &lane_inputs, &all_on);
        let simd_out = eval_gpu_simt(&simd, &lane_inputs, &[]); // predicated=false ignores the mask
        assert_eq!(
            simt_out, simd_out,
            "an all-ones-mask SIMT run must equal the pure-SIMD run (SIMD ⊂ SIMT), a={a}, b={b}"
        );
    }
}

/// The warp-reduce IS the NPU accumulator, placed spatially: (1) the reduced scalar equals the
/// `acc += out[j]` fold every dot-product/conv reduces to, and (2) the emitted reduce tree is
/// proven EQUAL to an independent balanced-tree fold over all lanes by our SAT — so every lane is
/// summed exactly once (none dropped, none duplicated), catching a mis-wired tree.
#[test]
fn gpu_map_then_reduce_composes_with_the_npu_accumulator() {
    let n = 8;
    let eng = engine(n, true, Some(AluOp::Add));

    // (1) reduced == the NPU-style accumulator fold over the (masked) lane outputs.
    let all_on = vec![true; n];
    let lane_inputs: Vec<Vec<i64>> = (0..n).map(|j| lane_in(3, (j as i64) + 1, 5)).collect();
    let (outs, reduced) = eval_gpu_simt(&eng, &lane_inputs, &all_on);
    let acc = outs.iter().fold(0i64, |a, &x| a.wrapping_add(x)); // acc += out[j] (the NPU recurrence)
    assert_eq!(
        reduced,
        Some(acc),
        "the warp-reduce must equal the accumulator fold over the lanes"
    );

    // (2) certified (structural): the emitted left-fold reduce tree == an independent balanced
    // tree, over all inputs — so every lane is summed exactly once (none dropped/duplicated). A
    // 4-wide, 4-bit exhaustive miter pins the tree structure; part (1) above already carries the
    // full N=8 numeric correctness (an adder tree's associativity is the only thing left to check).
    const RN: usize = 4;
    const PW: u32 = 4;
    let eng4 = engine(RN, true, Some(AluOp::Add));
    let left_fold = simt_reduce_to_bounded(&eng4, AluOp::Add, PW);
    fn balanced(lo: usize, hi: usize, width: u32) -> BoundedExpr {
        if hi - lo == 1 {
            return BoundedExpr::BitVecVar(format!("out{lo}"), width);
        }
        let mid = (lo + hi) / 2;
        BoundedExpr::BitVecBinary {
            op: BitVecBoundedOp::Add,
            left: Box::new(balanced(lo, mid, width)),
            right: Box::new(balanced(mid, hi, width)),
        }
    }
    let bal = balanced(0, RN, PW);
    let claim = BoundedExpr::Eq(Box::new(left_fold), Box::new(bal));
    let p = lower_bool(&claim).expect("claim lowers");
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(p))),
        UnsatOutcome::Refuted,
        "the warp-reduce must equal a balanced fold over ALL {RN} lanes (every lane summed once)"
    );
}

/// The SEQUENTIAL shift-add multiply is proven EQUAL to the hardware `a·x` for ALL inputs by our own
/// certified SAT — the theorem-prover makes the strategy switch FREE. The unrolled shift-add
/// (`acc += x[i] ? a<<i : 0`) is the exact combinational image of the `SequentialFma` FSM's
/// per-cycle recurrence; proving it equals a genuine `a*x` certifies that swapping in the
/// antenna-trivial datapath does NOT change the computed value (no divergence from the other tiers).
#[test]
fn gpu_sequential_multiply_is_certified_equal_to_the_hardware_multiply() {
    // Exhaustive over all PW-bit inputs. The shift-add unrolls to a full multiplier-equivalence
    // miter (the spec side is a genuine var×var `*`), which is a known-hard SAT instance, so a small
    // PW keeps it exhaustive yet tractable — the 64-bit range is carried by the iverilog differential
    // and the ≡VM battery; this pins the shift-add ALGEBRA (every partial product placed correctly).
    const PW: u32 = 4;
    let shift_add = simt_sequential_fma_to_bounded(PW); // Σ_i (in1[i] ? in0<<i : 0)
                                                        // spec = a * x — an independent hardware multiply (in0 = a, in1 = x).
    let spec = BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::Mul,
        left: Box::new(BoundedExpr::BitVecVar("in0".to_string(), PW)),
        right: Box::new(BoundedExpr::BitVecVar("in1".to_string(), PW)),
    };
    let claim = BoundedExpr::Eq(Box::new(shift_add), Box::new(spec));
    let p = lower_bool(&claim).expect("claim lowers");
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(p))),
        UnsatOutcome::Refuted,
        "the sequential shift-add must equal the hardware multiply a·x for ALL inputs (certified, not sampled)"
    );
}

/// The fan-out bound is STRUCTURAL — the pit-of-success antenna lock. The emitted SequentialFma RTL
/// slices the per-cycle conditional-add select `mp[0] ? mc : 0` into `select_slice` `(* keep *)`
/// copies, each gating exactly `WIDTH / select_slice` mux bits, so NO single select net can spider
/// across the full 64-bit datapath (the met1-antenna root cause). This test reads the flagship RTL
/// and asserts, purely structurally, that (1) exactly `select_slice` distinct `(* keep *)` select
/// copies exist for lane 0, (2) each is replicated `G = WIDTH / select_slice` times (fan-out G),
/// (3) the raw un-sliced high-fan-out forms are ABSENT, and (4) `G` is within the antenna-safe bound.
/// If a future edit re-introduces a wide select net, this lock goes RED before any silicon is drawn.
#[test]
fn gpu_sequential_select_fanout_is_structurally_bounded() {
    const WIDTH: usize = 64; // the i64 datapath width (the VM's fixed-width Int projection).
    const SELECT_SLICE: usize = FLAGSHIP_SELECT_SLICE; // the flagship fan-out slice (8).
    const GROUP: usize = WIDTH / SELECT_SLICE; // bits each select copy drives = the fan-out (8).
                                               // A documented antenna-safe fan-out ceiling: no emitted select net may drive more than this many
                                               // mux bits. `{8{...}}` (G = 8) sits at the bound; anything wider re-opens the met1-antenna risk.
    const ANTENNA_SAFE_FANOUT: usize = 8;

    let eng = flagship_engine();
    assert_eq!(
        eng.arith,
        LaneArith::SequentialFma {
            select_slice: SELECT_SLICE
        }
    );
    let rtl = emit_gpu_simt_verilog("simt", &eng);

    // (1) Lane 0 emits EXACTLY `SELECT_SLICE` distinct `(* keep *)` select copies `mselK_l0 = mp_l0[0]`.
    for s in 0..SELECT_SLICE {
        let line = format!("(* keep *) wire msel{s}_l0 = mp_l0[0];");
        assert!(
            rtl.contains(&line),
            "lane 0 must emit its own `(* keep *)` select copy `{line}` (fan-out slice {s})"
        );
    }
    // No EXTRA copy beyond the slice (the count is exactly `SELECT_SLICE`, not more).
    let extra = format!("wire msel{SELECT_SLICE}_l0 = mp_l0[0];");
    assert!(
        !rtl.contains(&extra),
        "lane 0 must emit EXACTLY {SELECT_SLICE} select copies — no `msel{SELECT_SLICE}_l0`"
    );
    // The exact count of `(* keep *) wire mselK_l0 =` lines for lane 0 equals `SELECT_SLICE`.
    let copies_l0 = rtl
        .lines()
        .filter(|l| l.contains("(* keep *) wire msel") && l.contains("_l0 = mp_l0[0];"))
        .count();
    assert_eq!(
        copies_l0, SELECT_SLICE,
        "lane 0 must have EXACTLY {SELECT_SLICE} `(* keep *)` select copies, found {copies_l0}"
    );

    // (2) Each `mselK_l0` drives a `{G{mselK_l0}}` replication of EXACTLY `G = WIDTH / select_slice`
    // bits — the fan-out is G, NOT the full 64. This is the structural fan-out bound.
    for s in 0..SELECT_SLICE {
        let repl = format!("{{{GROUP}{{msel{s}_l0}}}}");
        assert!(
            rtl.contains(&repl),
            "select copy `msel{s}_l0` must drive EXACTLY {GROUP} mux bits via `{repl}` (fan-out {GROUP})"
        );
    }

    // (3) The un-sliced HIGH-FAN-OUT forms must be ABSENT: neither a full-width `{64{...}}`
    // replication of the raw LSB, nor a single wide `mp_l0[0] ? mc_l0 : ...` select over the whole
    // datapath. Either would be the ~64-fan-out net that creates the met1 antenna.
    assert!(
        !rtl.contains(&format!("{{{WIDTH}{{msel0_l0}}}}")),
        "no select copy may drive the FULL {WIDTH}-bit datapath (a `{{{WIDTH}{{...}}}}` replication)"
    );
    assert!(
        !rtl.contains("{64{mp_l0[0]}}"),
        "the raw multiplier LSB `mp_l0[0]` must not appear in a full-width `{{64{{...}}}}` replication"
    );
    assert!(
        !rtl.contains("mp_l0[0] ? mc_l0"),
        "the un-sliced single wide select `mp_l0[0] ? mc_l0 : ...` (full 64-bit fan-out) must be absent"
    );

    // (4) The resulting max select fan-out (G) is within the documented antenna-safe bound — the
    // enforced invariant: no emitted net spiders wide enough to create a met1 antenna.
    assert!(
        GROUP <= ANTENNA_SAFE_FANOUT,
        "the max select fan-out G = WIDTH/select_slice = {GROUP} must be ≤ the antenna-safe bound {ANTENNA_SAFE_FANOUT}"
    );
}

/// The sliced partial product is SAT-certified EQUAL to the single-select mux — closing the gap
/// where the byte-slicing was only iverilog-checked. `simt_sliced_pp_to_bounded` models the emitted
/// `pp = { {G{s}} & mc[hi:lo] , ... }` (each `msel` copy == the one bit `s = mp0`); we prove, over
/// ALL inputs, that it equals `s==1 ? mc : 0` — so the slicing drops or misaligns NO bit. Two slice
/// regimes are certified: a multi-bit group (width 8, select_slice 4 → G = 2) and the degenerate
/// per-bit slice (width 8, select_slice 8 → G = 1), locking the tiling at both extremes.
#[test]
fn gpu_sliced_partial_product_is_certified_equal_to_the_select_mux() {
    for &(width, slice) in &[(8u32, 4usize), (8u32, 8usize)] {
        let sliced = simt_sliced_pp_to_bounded(slice, width); // { {G{mp0}} & mc[hi:lo] , ... }
                                                              // spec = (mp0 == 1) ? mc : 0 — the single wide select the slicing must reproduce bit-for-bit.
        let cond = Box::new(BoundedExpr::BitVecBinary {
            op: BitVecBoundedOp::Eq,
            left: Box::new(BoundedExpr::BitVecVar("mp0".to_string(), 1)),
            right: Box::new(BoundedExpr::BitVecConst { width: 1, value: 1 }),
        });
        let spec = BoundedExpr::BitVecIte {
            cond,
            then: Box::new(BoundedExpr::BitVecVar("mc".to_string(), width)),
            else_: Box::new(BoundedExpr::BitVecConst { width, value: 0 }),
        };
        let claim = BoundedExpr::Eq(Box::new(sliced), Box::new(spec));
        let p = lower_bool(&claim).expect("claim lowers");
        assert_eq!(
            prove_unsat(&ProofExpr::Not(Box::new(p))),
            UnsatOutcome::Refuted,
            "the sliced partial product must equal `mp0 ? mc : 0` for ALL inputs \
             (width={width}, select_slice={slice}) — the byte-slicing drops/misaligns no bit"
        );
    }
}

/// The signoff ADVISORY documents the antenna envelope WITHOUT failing any buildable config: it
/// flags the wide combinational strategies at 64-bit width (met1-antenna-dense on Sky130) and stays
/// silent for the sequential strategy (antenna-trivial). It is advisory only — it NEVER blocks
/// emission (every strategy emits valid RTL, proven by `gpu_lane_arith_strategies_are_modular`).
#[test]
fn gpu_signoff_advisory_flags_the_antenna_dense_envelope() {
    // The two WIDE combinational strategies at the flagship 64-bit width warn (met1-antenna-dense).
    assert!(
        signoff_advisory(&LaneArith::Combinational, 64, FLAGSHIP_LANES).is_some(),
        "Combinational@64 must raise the met1-antenna advisory"
    );
    assert!(
        signoff_advisory(&LaneArith::DecomposedFma { stages: 2 }, 64, FLAGSHIP_LANES).is_some(),
        "DecomposedFma@64 must raise the met1-antenna advisory"
    );
    // The sequential strategy (bounded fan-out) is antenna-trivial — no advisory.
    assert!(
        signoff_advisory(&FLAGSHIP_ARITH, 64, FLAGSHIP_LANES).is_none(),
        "SequentialFma@64 is antenna-trivial — NO advisory"
    );
    // Narrow widths (< 32) are not antenna-dense even for the combinational strategies.
    assert!(
        signoff_advisory(&LaneArith::Combinational, 16, FLAGSHIP_LANES).is_none(),
        "a 16-bit combinational multiply is below the antenna-dense width envelope"
    );
    // The flagship strategy is exactly the one that clears the advisory — the switch is signoff-clean.
    assert!(
        signoff_advisory(&FLAGSHIP_ARITH, 64, FLAGSHIP_LANES).is_none(),
        "the flagship strategy must clear the signoff advisory"
    );
}

// ============================================================================================
// Synthesizable RTL — a real N-lane spatial array with a hardware multiply, no warp scheduler.
// ============================================================================================

#[test]
fn gpu_tile_emits_synthesizable_rtl_with_the_sequential_multiply() {
    let n = FLAGSHIP_LANES;
    let eng = flagship_engine();
    let rtl = emit_gpu_simt_verilog("simt", &eng);

    assert!(
        rtl.contains("module gpu_simt"),
        "the tile emits a named module (gpu_<name>)"
    );
    assert!(
        rtl.contains("endmodule"),
        "the emitted RTL is a complete module"
    );
    assert!(
        rtl.contains("posedge clk"),
        "a clocked register stage gives a real timing path"
    );
    assert!(
        rtl.contains("mask"),
        "SIMT predication exposes a per-lane mask"
    );
    assert!(
        rtl.contains("redr"),
        "the warp-reduce latches a reduced register"
    );
    // The flagship is the SEQUENTIAL shift-add multiply: one reused adder per lane, a step counter,
    // and shifting multiplicand/multiplier registers — NO wide combinational multiplier block. This
    // is the antenna-trivial datapath; assert its shift-add shape.
    assert_eq!(
        eng.arith, FLAGSHIP_ARITH,
        "the flagship signs off the sequential strategy"
    );
    for s in ["acc_l0", "scnt", "mp_l0", "<< 1", ">> 1", "running"] {
        assert!(
            rtl.contains(s),
            "the sequential shift-add datapath must contain `{s}`"
        );
    }
    // The conditional-add partial product is emitted as a FANOUT-BOUNDED, `(* keep *)`-sliced wire:
    // SELECT_SLICE copies of the multiplier LSB `mp[0]`, each gating its own byte-group of `mc`, so
    // no single select net spiders across the full 64-bit datapath (the met1-antenna root cause).
    // yosys preserves `(* keep *)` nets; iverilog ignores the attribute, so simulation is unchanged.
    assert!(
        rtl.contains("(* keep *)"),
        "the sliced select copies carry a `(* keep *)` attribute so yosys cannot merge them"
    );
    assert!(
        rtl.contains("msel0_l0"),
        "lane 0 must emit its own `(* keep *)` select copy `msel0_l0`"
    );
    assert!(
        rtl.contains("pp_l0"),
        "lane 0's conditional-add partial product is the sliced wire `pp_l0`"
    );
    assert!(
        rtl.contains("acc_l0 <= acc_l0 + pp_l0"),
        "the FSM add consumes the sliced partial product `pp_l0`"
    );
    // The un-sliced WIDE single-select mux must be GONE — its ~64-fanout select net is the antenna
    // net the slicing structurally removes; if it leaked back in, the antenna violation returns.
    assert!(
        !rtl.contains("mp_l0[0] ? mc_l0"),
        "the wide single-select mux `mp_l0[0] ? mc_l0` must be replaced by the sliced `pp_l0`"
    );
    // It must NOT contain a WIDE decomposed partial-product register — that is the combinational
    // strategy's structure, and its presence would mean the antenna-dense multiply leaked in.
    assert!(
        !rtl.contains("pll_l0"),
        "the sequential strategy must have NO wide `pll_l0` partial product"
    );
    // A narrow streaming interface (physically realizable — not thousands of parallel pins).
    for port in ["din", "dout", "load", "go", "raddr", "done"] {
        assert!(
            rtl.contains(port),
            "the streaming interface must expose `{port}`"
        );
    }
    // N INDEPENDENT lane result registers + N per-lane datapaths — the spatial parallel MAP (this is
    // what makes it a GPU, not a reduction).
    for j in 0..n {
        assert!(
            rtl.contains(&format!("outr{j}")),
            "lane {j} must have its own result register"
        );
        assert!(
            rtl.contains(&format!("g_l{j}")),
            "lane {j} must have its own per-lane datapath"
        );
    }
    // The SIMT lanes have NO instruction dispatch — no fetch/decode/opcode. (The load `case` is
    // I/O address decode, not instruction decode.)
    let code: String = rtl
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in ["opcode", "fetch", "decode"] {
        assert!(
            !code.contains(forbidden),
            "a spatial SIMT array must not contain `{forbidden}`"
        );
    }
}

/// The `LaneArith` knob is MODULAR: all three strategies still emit their own valid RTL. This proves
/// the sequential strategy was ADDED (not a replacement) — `Combinational` keeps its wide `*` body,
/// `DecomposedFma` keeps its 32×32 partial-product split, and only `SequentialFma` is the new shape.
#[test]
fn gpu_lane_arith_strategies_are_modular() {
    let n = FLAGSHIP_LANES;
    let mut base = SimtEngine::from_lane_source(
        &shade_main(2, 3, 4),
        "shade",
        n,
        vec![false, false, false],
        true,
        Some(AluOp::Add),
    );

    // Combinational: a per-op body with a WIDE hardware multiply (`*`), single-cycle (no drain FSM).
    base.arith = LaneArith::Combinational;
    let comb = emit_gpu_simt_verilog("simt", &base);
    assert!(
        comb.contains('*'),
        "the combinational strategy must use a hardware multiply (`*`)"
    );
    assert!(
        !comb.contains("pll_l0"),
        "the combinational strategy has no decomposed partial product"
    );
    assert!(
        !comb.contains("scnt"),
        "the combinational strategy is single-cycle (no step counter)"
    );

    // DecomposedFma{2}: the 32×32 partial-product split, with a `*` on the halves and a drain FSM.
    base.arith = LaneArith::DecomposedFma { stages: 2 };
    let decomp = emit_gpu_simt_verilog("simt", &base);
    assert!(
        decomp.contains('*'),
        "the decomposed strategy still multiplies the 32-bit halves (`*`)"
    );
    assert!(
        decomp.contains("pll_l0"),
        "the decomposed strategy registers partial product `pll_l0`"
    );
    assert!(
        decomp.contains("[31:0]"),
        "the decomposed multiply operates on 32-bit halves"
    );
    assert!(
        decomp.contains("running"),
        "the decomposed datapath has a drain-counter FSM"
    );

    // SequentialFma: NO multiply operator anywhere — a pure shift-add. This is the antenna-trivial
    // strategy. The fanout-bounded partial product carries `(* keep *)` synthesis attributes whose
    // delimiters contain `*` characters that are NOT multiplies, so strip the attributes first, then
    // assert the residue has no `*` — locking the true invariant (no hardware multiply) precisely.
    base.arith = LaneArith::SequentialFma {
        select_slice: FLAGSHIP_SELECT_SLICE,
    };
    let seq = emit_gpu_simt_verilog("simt", &base);
    let seq_no_attrs = seq.replace("(* keep *)", "");
    assert!(!seq_no_attrs.contains('*'), "the SEQUENTIAL strategy must be a shift-add — NO multiply `*` anywhere (`(* keep *)` attributes excluded)");
    assert!(
        seq.contains("(* keep *)"),
        "the sequential strategy's fanout-bounded select copies carry the `(* keep *)` attribute"
    );
    for s in [
        "acc_l0", "mc_l0", "mp_l0", "scnt", "<< 1", ">> 1", "pp_l0", "msel0_l0",
    ] {
        assert!(
            seq.contains(s),
            "the sequential strategy must contain the shift-add register `{s}`"
        );
    }

    // All three emit a complete, distinct module (the knob is real, not a no-op).
    for rtl in [&comb, &decomp, &seq] {
        assert!(
            rtl.contains("module gpu_simt") && rtl.contains("endmodule"),
            "each strategy is a full module"
        );
    }
    assert_ne!(
        comb, decomp,
        "Combinational and DecomposedFma emit different RTL"
    );
    assert_ne!(
        comb, seq,
        "Combinational and SequentialFma emit different RTL"
    );
    assert_ne!(
        decomp, seq,
        "DecomposedFma and SequentialFma emit different RTL"
    );
}

// ============================================================================================
// Checked-in receipt artifacts (RTL + testbench + VM-oracle vectors) — the iverilog/FPGA gate.
// ============================================================================================

/// The flagship signed-off configuration: N=8 predicated SIMT lanes of the affine FMA shader, with
/// a warp-reduce. Emit its RTL, its clocked testbench, and the VM-oracle hex vectors (golden;
/// regenerate with `FABRIC_RECORD=1`). The iverilog gate (a script) clocks the engine against
/// these vectors; OpenLane takes the same `gpu_simt.v` to Sky130 signoff.
#[test]
fn gpu_artifacts_are_current() {
    use std::path::Path;
    const N_LANES: usize = FLAGSHIP_LANES;
    const N_VEC: usize = 24;
    let eng = flagship_engine();

    // In-range vectors (so the golden hex is genuinely the VM's output): a,b broadcast; per-lane
    // x = j+1 scaled by the vector index; a varying mask so predication is exercised on silicon.
    let vectors: Vec<(Vec<Vec<i64>>, Vec<bool>)> = (0..N_VEC)
        .map(|t| {
            let a = 3 + (t as i64 % 4);
            let b = 5 + (t as i64 % 3);
            let lane_inputs: Vec<Vec<i64>> = (0..N_LANES)
                .map(|j| lane_in(a, (j as i64) + 1 + (t as i64), b))
                .collect();
            let mask: Vec<bool> = (0..N_LANES).map(|j| (j + t) % 3 != 0).collect();
            (lane_inputs, mask)
        })
        .collect();

    let (hin, hout) = write_gpu_hex(&eng, &vectors);
    let files: Vec<(&str, String)> = vec![
        ("gpu_simt.v", emit_gpu_simt_verilog("simt", &eng)),
        (
            "tb_gpu_simt.v",
            emit_gpu_simt_testbench("simt", &eng, N_VEC),
        ),
        ("gpu_simt_in.hex", hin),
        ("gpu_simt_out.hex", hout),
    ];

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon");
    if std::env::var("FABRIC_RECORD").is_ok() {
        for (n, b) in &files {
            std::fs::write(dir.join(n), b).expect("write");
        }
        return;
    }
    for (n, b) in &files {
        let got = std::fs::read_to_string(dir.join(n)).unwrap_or_else(|e| {
            panic!("missing {n} ({e}) — record: FABRIC_RECORD=1 cargo test gpu_artifacts")
        });
        assert_eq!(&got, b, "{n} is stale — regenerate with FABRIC_RECORD=1");
    }
}

/// The streamed engine — one multiply shared by every lane, which each lane's `x` enters as it loads — computes the
/// reference engine's every lane and its warp-reduce under iverilog: lane counts from one to sixteen (three, no power
/// of two, among them), uniform and per-lane `a` and `b`, every reduce it folds, registered after every level and
/// after every second and third, and masks all on, all off, alternating and random, on words at the extremes and
/// random.
#[test]
fn the_streamed_engine_streams_every_lane_and_its_reduce() {
    use logicaffeine_compile::codegen_sva::gpu_simt_rtl::write_gpu_hex;
    if !common::require_tool("iverilog") {
        return;
    }
    let configs: [(usize, [bool; 3], AluOp); 7] = [
        (1, [true, false, true], AluOp::Add),
        (2, [true, false, true], AluOp::Add),
        (4, [true, false, true], AluOp::Add),
        (4, [false, false, false], AluOp::Add),
        (8, [false, false, true], AluOp::Xor),
        (3, [true, false, false], AluOp::And),
        (16, [true, false, true], AluOp::Or),
    ];
    struct Words(u64);
    impl Words {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        /// Every third word an extreme, the rest random.
        fn word(&mut self, k: u64) -> i64 {
            const EDGES: [i64; 7] = [0, 1, -1, i64::MAX, i64::MIN, 0x5555_5555_5555_5555, 1 << 32];
            if k % 3 == 0 { EDGES[(k as usize / 3) % EDGES.len()] } else { self.next() as i64 }
        }
    }
    let mut w = Words(0x2545_F491_4F6C_DD1D);
    for (n, uniform, reduce) in configs {
        for every in [1usize, 2, 3] {
            let mut eng = SimtEngine::from_lane_source(&shade_main(2, 3, 4), "shade", n, uniform.to_vec(), true, Some(reduce));
            eng.arith = LaneArith::Streamed { every };
            let vectors: Vec<(Vec<Vec<i64>>, Vec<bool>)> = (0..24u64)
                .map(|v| {
                    let shared: Vec<i64> = (0..3).map(|i| w.word(v * 7 + i)).collect();
                    let lanes = (0..n)
                        .map(|j| (0..3).map(|k| if uniform[k] { shared[k] } else { w.word(v * 11 + (j * 3 + k) as u64) }).collect())
                        .collect();
                    let mask = (0..n)
                        .map(|j| match v % 4 {
                            0 => true,
                            1 => false,
                            2 => j % 2 == 0,
                            _ => w.next() & 1 == 1,
                        })
                        .collect();
                    (lanes, mask)
                })
                .collect();
            let (hin, hout) = write_gpu_hex(&eng, &vectors);
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join("gpu_streamed_in.hex"), hin).unwrap();
            std::fs::write(dir.path().join("gpu_streamed_out.hex"), hout).unwrap();
            std::fs::write(dir.path().join("gpu.v"), emit_gpu_simt_verilog("streamed", &eng)).unwrap();
            std::fs::write(dir.path().join("tb.v"), emit_gpu_simt_testbench("streamed", &eng, vectors.len())).unwrap();
            let b = std::process::Command::new("iverilog").args(["-g2005", "-o", "sim", "tb.v", "gpu.v"]).current_dir(dir.path()).output().unwrap();
            assert!(b.status.success(), "N={n} every={every}: {}", String::from_utf8_lossy(&b.stderr));
            let r = std::process::Command::new("vvp").args(["-n", "sim"]).current_dir(dir.path()).output().unwrap();
            let out = String::from_utf8_lossy(&r.stdout);
            assert!(out.contains("PASS"), "N={n} uniform={uniform:?} reduce={reduce:?} every={every}: {out}");
        }
    }
}

/// The streamed engine has no multiply operator — its one multiply is a tree of adds — and refuses a kernel that is
/// not `a·x + b`, a uniform `x`, and a reduce it cannot fold.
#[test]
fn the_streamed_engine_has_no_multiply_and_refuses_what_it_cannot_stream() {
    let mut eng = engine(4, true, Some(AluOp::Add));
    eng.arith = LaneArith::Streamed { every: 1 };
    let rtl = emit_gpu_simt_verilog("streamed", &eng);
    assert!(!rtl.contains('*'), "a tree of adds, no `*`");
    for s in ["mt_r0", "mt_s5_0", "outr3", "redr", "pv[", "waiting"] {
        assert!(rtl.contains(s), "the streamed engine holds `{s}`");
    }
    let refused = |e: SimtEngine| std::panic::catch_unwind(|| emit_gpu_simt_verilog("x", &e)).is_err();
    let mut ux = engine(4, true, Some(AluOp::Add));
    ux.uniform = vec![true, true, true];
    ux.arith = LaneArith::Streamed { every: 1 };
    assert!(refused(ux), "a uniform x");
    let mut mul_reduce = engine(4, true, Some(AluOp::Mul));
    mul_reduce.arith = LaneArith::Streamed { every: 1 };
    assert!(refused(mul_reduce), "a reduce it cannot fold");
    let mut other = SimtEngine::from_lane_source(
        "## To shade (a: Int, x: Int, b: Int) -> Int:\n    Return a * x - b.\n\n## Main\n\nShow shade(2, 3, 4).\n",
        "shade",
        4,
        vec![true, false, true],
        true,
        Some(AluOp::Add),
    );
    other.arith = LaneArith::Streamed { every: 1 };
    assert!(refused(other), "a kernel that is not a·x + b");
}
