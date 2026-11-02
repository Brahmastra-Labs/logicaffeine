//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ELEMENTWISE-CHAIN FUSION — the general, supercompiler-smart operator-fusion pass.
//!
//! The orchestrator ([`codegen_cuda::qwen_emit`]) used to emit ONE kernel per elementwise stage, chaining
//! every intermediate through GLOBAL memory (write-then-read) and paying a kernel launch each. That is one
//! hand-recognized fusion short of a real compiler: it only fused the single `add → rmsnorm` case.
//!
//! This lock pins a GENERAL fusion pass — not a bag of special cases. A maximal run of consecutive
//! elementwise ops (`+`, `hadamard`) collapses into ONE kernel whose intermediates live in REGISTERS and
//! never touch global memory; only the run's final result (and any value a later stage still needs) is
//! materialized. It is:
//!   • GENERAL — any length, any mix of `+`/`hadamard`, no shape the compiler was hand-told (versatility);
//!   • CORRECT-BY-DATAFLOW — a value read by a stage OUTSIDE the run STOPS the run there, so it stays
//!     materialized (the "won't corrupt your weights" guard); proven both structurally and numerically ≡ VM;
//!   • A WIN — N elementwise launches + (N−1) global round-trips collapse to 1 launch, 0 intermediate I/O.
//!
//! Structural assertions always run; the numeric ≡-VM assertions are gated on `nvcc` + a live GPU and SKIP
//! cleanly otherwise.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_elementwise_fusion_lock)'
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::process::Command;

use logicaffeine_compile::compile::{compile_to_cuda_forward, vm_outcome};

mod common;

fn have_nvcc() -> bool {
    Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
fn have_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
}

/// Compile the emitted `.cu` with nvcc and run it, returning stdout whitespace tokens.
fn nvcc_run(cu: &str, tag: &str) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("logos_fuse_{tag}_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (src, bin) = (dir.join("f.cu"), dir.join("f"));
    std::fs::write(&src, cu).unwrap();
    let c = Command::new("nvcc")
        .args(["-O2", "-arch=sm_86"])
        .arg(&src)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("nvcc");
    assert!(
        c.status.success(),
        "emitted fused forward must compile:\n{}\n--- cu ---\n{cu}",
        String::from_utf8_lossy(&c.stderr)
    );
    let r = Command::new(&bin).output().expect("run fused forward");
    assert!(
        r.status.success(),
        "emitted fused forward run failed:\n{}",
        String::from_utf8_lossy(&r.stderr)
    );
    let out = String::from_utf8_lossy(&r.stdout)
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    out
}

/// A 3-op linear elementwise chain `E=A+B; F=E∘C; G=F+D` over 1×4 Int tensors (∘ = hadamard).
const LINEAR_INT: &str = "## Main\n\
    Let A be a 1 by 4 tensor of Int.\n\
    Set item 1 of A to 1.\nSet item 2 of A to 2.\nSet item 3 of A to 3.\nSet item 4 of A to 4.\n\
    Let B be a 1 by 4 tensor of Int.\n\
    Set item 1 of B to 5.\nSet item 2 of B to 6.\nSet item 3 of B to 7.\nSet item 4 of B to 8.\n\
    Let C be a 1 by 4 tensor of Int.\n\
    Set item 1 of C to 2.\nSet item 2 of C to 2.\nSet item 3 of C to 3.\nSet item 4 of C to 3.\n\
    Let D be a 1 by 4 tensor of Int.\n\
    Set item 1 of D to 10.\nSet item 2 of D to 20.\nSet item 3 of D to 30.\nSet item 4 of D to 40.\n\
    Let E be A plus B.\n\
    Let F be E hadamard C.\n\
    Let G be F plus D.\n\
    Show item 1 of G.\nShow item 2 of G.\nShow item 3 of G.\nShow item 4 of G.\n";

#[test]
fn a_linear_elementwise_chain_fuses_into_one_kernel() {
    let cu = compile_to_cuda_forward(LINEAR_INT)
        .expect("linear elementwise chain must lower to composed CUDA");
    // ONE fused kernel for the whole chain — and NO standalone per-op elementwise kernels remain.
    assert!(
        cu.contains("k0_fused"),
        "the 3-op chain must fuse into a single kernel:\n{cu}"
    );
    assert!(
        !cu.contains("_elem"),
        "no standalone per-op elementwise kernels should survive the fusion:\n{cu}"
    );
}

#[test]
fn the_fused_kernel_keeps_intermediates_in_registers() {
    let cu = compile_to_cuda_forward(LINEAR_INT).expect("must lower");
    // The chain runs as register temps: t = A+B; t = t*C; t = t+D; o = t — a SINGLE global write for 3 ops.
    assert!(
        cu.contains("long long t = i0[idx] + i1[idx]"),
        "first op reads two inputs into a register:\n{cu}"
    );
    assert!(
        cu.contains("t = t * i2[idx]"),
        "second op consumes the register, not a global buffer:\n{cu}"
    );
    assert!(
        cu.contains("t = t + i3[idx]"),
        "third op consumes the register:\n{cu}"
    );
    assert_eq!(
        cu.matches("o[idx] = t").count(),
        1,
        "exactly ONE global store for the whole chain:\n{cu}"
    );
    // The intermediates E and F never round-trip: the fused launch passes only the externals + final output.
    assert!(
        cu.contains("(buf_A,buf_B,buf_C,buf_D,buf_G,4)"),
        "fused launch wires externals + G only (no buf_E/buf_F):\n{cu}"
    );
}

/// A 4-op chain mixing `+` and `hadamard`, none of which the compiler was hand-told about — generality.
const MIXED4: &str = "## Main\n\
    Let A be a 1 by 4 tensor of Int.\n\
    Set item 1 of A to 1.\nSet item 2 of A to 2.\nSet item 3 of A to 3.\nSet item 4 of A to 4.\n\
    Let B be a 1 by 4 tensor of Int.\n\
    Set item 1 of B to 5.\nSet item 2 of B to 6.\nSet item 3 of B to 7.\nSet item 4 of B to 8.\n\
    Let C be a 1 by 4 tensor of Int.\n\
    Set item 1 of C to 1.\nSet item 2 of C to 1.\nSet item 3 of C to 2.\nSet item 4 of C to 2.\n\
    Let D be a 1 by 4 tensor of Int.\n\
    Set item 1 of D to 2.\nSet item 2 of D to 3.\nSet item 3 of D to 4.\nSet item 4 of D to 5.\n\
    Let E be A plus B.\n\
    Let F be E plus C.\n\
    Let G be F hadamard D.\n\
    Let H be G plus A.\n\
    Show item 1 of H.\nShow item 2 of H.\nShow item 3 of H.\nShow item 4 of H.\n";

#[test]
fn fusion_is_general_over_op_mix_and_length() {
    let cu = compile_to_cuda_forward(MIXED4).expect("4-op mixed chain must lower");
    assert!(
        cu.contains("k0_fused"),
        "a 4-op +/hadamard chain must fuse into one kernel:\n{cu}"
    );
    assert!(!cu.contains("_elem"), "no per-op kernels remain:\n{cu}");
    // Four elementwise ops = one seed op reading two inputs, then THREE register-carried ops.
    assert_eq!(
        cu.matches("t = t ").count(),
        3,
        "three register-carried tail ops for a length-4 chain:\n{cu}"
    );
}

/// SAFETY: an intermediate needed AFTER a non-elementwise stage must be materialized, not fused away.
/// `E=A+B; F=E∘C; G=F+D; P=rmsnorm(G); Q=P+E` — E (stage 0) is revived by Q AFTER the rmsnorm. The run
/// [F,G] still fuses (a real win), but E stays a materialized buffer so Q reads the correct value.
const SAFETY: &str = "## Main\n\
    Let A be a 1 by 4 tensor of Real.\n\
    Set item 1 of A to 1.0.\nSet item 2 of A to 2.0.\nSet item 3 of A to 3.0.\nSet item 4 of A to 4.0.\n\
    Let B be a 1 by 4 tensor of Real.\n\
    Set item 1 of B to 0.5.\nSet item 2 of B to 0.5.\nSet item 3 of B to 1.0.\nSet item 4 of B to 1.0.\n\
    Let C be a 1 by 4 tensor of Real.\n\
    Set item 1 of C to 2.0.\nSet item 2 of C to 2.0.\nSet item 3 of C to 2.0.\nSet item 4 of C to 2.0.\n\
    Let D be a 1 by 4 tensor of Real.\n\
    Set item 1 of D to 0.1.\nSet item 2 of D to 0.2.\nSet item 3 of D to 0.3.\nSet item 4 of D to 0.4.\n\
    Let E be A plus B.\n\
    Let F be E hadamard C.\n\
    Let G be F plus D.\n\
    Let P be rmsnorm of G.\n\
    Let Q be P plus E.\n\
    Show item 1 of Q.\nShow item 2 of Q.\nShow item 3 of Q.\nShow item 4 of Q.\n";

#[test]
fn an_intermediate_needed_later_is_materialized_not_fused_away() {
    let cu = compile_to_cuda_forward(SAFETY).expect("safety program must lower");
    // E is standalone-materialized (it has a reader after the rmsnorm), F∘G still fuse (a real win),
    // rmsnorm is its own row kernel, and Q reads the MATERIALIZED E across the rmsnorm.
    assert!(
        cu.contains("k0_elem"),
        "E must be materialized as its own kernel (a later stage revives it):\n{cu}"
    );
    assert!(
        cu.contains("k1_fused"),
        "the F+G run must still fuse — the guard is surgical, not global:\n{cu}"
    );
    assert!(
        cu.contains("k2_row"),
        "rmsnorm(G) is its own fused-row kernel:\n{cu}"
    );
    assert!(
        cu.contains("k3_elem"),
        "Q is its own kernel after the rmsnorm break:\n{cu}"
    );
    assert!(
        cu.contains("buf_P,buf_E,buf_Q,4"),
        "Q reads the materialized E across the rmsnorm (correctness guard):\n{cu}"
    );
}

/// SOUNDNESS (direct unit test of the chain rule): a value read by a later norm's RESIDUAL input must NOT be fused
/// away. `E=A+B; F=E+C; Y=rmsnorm(X+E)·W` — E is the residual the norm adds, so the shared table under the forward's
/// shim must refuse to run [0,1] as one chain (which would elide E) and leave E materialized. Built directly as the
/// lifted ops because no sugar spells a norm over a residual; this pins the rule's liveness guard in isolation.
#[test]
fn the_fusion_guard_respects_a_fused_row_residual_input() {
    use logicaffeine_compile::codegen_cuda::forward_shim::ForwardShim;
    use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
    use logicaffeine_compile::intern::Interner;
    use logicaffeine_compile::spatial::passes::recognize_stages::{DeclaredTensor, RecognizedProgram, RecognizedStage, TensorOpSpec};
    use logicaffeine_compile::tensor_graph::fuse::optimize;
    use logicaffeine_compile::tensor_graph::graph::{Storage, Sym, TensorGraph};
    use logicaffeine_compile::tensor_recognize::{ElementwiseOp, ElementwiseSpec, Epsilon, Fill, FusedRowSpec, RowReduceKind};

    let mut it = Interner::new();
    let names = ["A", "B", "C", "E", "F", "X", "Y", "W"];
    let [a, b, c, e, f, x, y, w] = names.map(|n| it.intern(n));
    let add = |a, b, c| ElementwiseSpec { a, b, c, op: ElementwiseOp::Add, n: 4, row: None };
    let stage = |op: TensorOpSpec, ins: &[&str], out: &str| RecognizedStage { op, stmt_index: 0, out: out.into(), ins: ins.iter().map(|s| s.to_string()).collect() };
    let rp = RecognizedProgram {
        stages: vec![
            stage(TensorOpSpec::Elementwise(add(a, b, e)), &["A", "B"], "E"),
            stage(TensorOpSpec::Elementwise(add(e, c, f)), &["E", "C"], "F"),
            stage(
                TensorOpSpec::FusedRow(FusedRowSpec {
                    rows: 1,
                    cols: 4,
                    reduce: RowReduceKind::RmsNorm,
                    x,
                    y,
                    weight: Some(w),
                    weight_rows: 1,
                    residual: Some(e),
                    bias: None,
                    eps: Epsilon::DEFAULT,
                }),
                &["X", "W", "E"],
                "Y",
            ),
        ],
        declared: ["A", "B", "C", "X", "W"].iter().map(|t| DeclaredTensor { name: t.to_string(), len: 4, fill: Fill::ConstF(0.5f64.to_bits()) }).collect(),
        symbols: names.iter().map(|n| (it.intern(n), n.to_string())).collect(),
        ..RecognizedProgram::default()
    };
    let g = TensorGraph::lift(&rp, None, None, &|_| Storage::Values).expect("the three ops lift");
    let fused = optimize(&g, &ForwardShim { kind: Q8MatKind::Dequant }, &[]).graph;
    assert!(
        matches!(fused.ops[0].sym, Sym::Prim(TensorOpSpec::Elementwise(_))),
        "E must stay materialized — a later residual reads it: {:?}",
        fused.ops.iter().map(|o| o.kind().to_string()).collect::<Vec<_>>()
    );
    assert!(
        fused.ops.iter().all(|o| o.kind() != "elementwise_chains"),
        "the residual reader must block the chain: {:?}",
        fused.ops.iter().map(|o| o.kind().to_string()).collect::<Vec<_>>()
    );
    assert_eq!(fused.ops.len(), 3, "no run collapses when the liveness guard fires");
}

/// LIFT-AND-SHIFT-LEFT: absorb a unary `silu` into the elementwise chain. `X=A+B; Y=silu(X); Z=Y∘C` is the
/// SwiGLU gate shape — it must fuse into ONE kernel `t=a+b; t=silu(t); t=t*c; o=t` (the silu + both elementwise
/// ops carried in registers), not three separate kernels.
const SWIGLU_CHAIN: &str = "## Main\n\
    Let A be a 1 by 4 tensor of Real.\n\
    Set item 1 of A to -1.0.\nSet item 2 of A to 0.5.\nSet item 3 of A to 2.0.\nSet item 4 of A to -0.5.\n\
    Let B be a 1 by 4 tensor of Real.\n\
    Set item 1 of B to 0.5.\nSet item 2 of B to 0.5.\nSet item 3 of B to 0.5.\nSet item 4 of B to 0.5.\n\
    Let C be a 1 by 4 tensor of Real.\n\
    Set item 1 of C to 1.0.\nSet item 2 of C to 2.0.\nSet item 3 of C to 3.0.\nSet item 4 of C to 4.0.\n\
    Let X be A plus B.\n\
    Let Y be silu of X.\n\
    Let Z be Y hadamard C.\n\
    Show item 1 of Z.\nShow item 2 of Z.\nShow item 3 of Z.\nShow item 4 of Z.\n";

#[test]
fn silu_absorbs_into_the_elementwise_chain() {
    let cu = compile_to_cuda_forward(SWIGLU_CHAIN).expect("swiglu-shaped chain must lower");
    assert!(
        cu.contains("k0_fused"),
        "the +/silu/* chain must fuse into one kernel:\n{cu}"
    );
    assert!(
        !cu.contains("_silu"),
        "the silu must be absorbed into the chain, not a separate kernel:\n{cu}"
    );
    assert!(
        !cu.contains("_elem"),
        "no standalone elementwise kernels remain:\n{cu}"
    );
    assert!(
        cu.contains("float t = i0[idx] + i1[idx]"),
        "seed add in a register:\n{cu}"
    );
    assert!(
        cu.contains("t = t/(1.0f+expf(-t))"),
        "silu applied in-register on the carry:\n{cu}"
    );
    assert!(
        cu.contains("t = t * i2[idx]"),
        "then the hadamard multiply in-register:\n{cu}"
    );
    assert_eq!(
        cu.matches("o[idx] = t").count(),
        1,
        "single global store for the whole SwiGLU chain:\n{cu}"
    );
}

#[test]
fn silu_chain_equals_the_vm_on_the_gpu() {
    let cu = compile_to_cuda_forward(SWIGLU_CHAIN).expect("must lower");
    if !(have_nvcc() && have_gpu()) {
        eprintln!(
            "SKIP silu-chain GPU: needs nvcc + a live GPU. (Emission asserted structurally.)"
        );
        return;
    }
    let vm = vm_outcome(SWIGLU_CHAIN);
    assert!(vm.error.is_none(), "VM error: {:?}", vm.error);
    let expect: Vec<f64> = vm
        .output
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect();
    let got: Vec<f32> = nvcc_run(&cu, "silu")
        .iter()
        .map(|s| s.parse().unwrap())
        .collect();
    assert_eq!(got.len(), 4, "expected 4 outputs");
    for (g, r) in got.iter().zip(&expect) {
        assert!(
            (*g as f64 - r).abs() < 1e-3,
            "silu-absorbed chain on GPU {g} vs VM {r}"
        );
    }
    println!("Fusion: silu absorbed into a +/silu/* chain (SwiGLU gate) ran as ONE kernel on the GPU ≡ VM.");
}

#[test]
fn fused_chain_equals_the_vm_on_the_gpu() {
    let cu = compile_to_cuda_forward(LINEAR_INT).expect("must lower");
    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP fusion GPU run: needs nvcc + a live GPU. (Emission asserted by the structural tests.)");
        return;
    }
    let vm = vm_outcome(LINEAR_INT);
    assert!(vm.error.is_none(), "VM error: {:?}", vm.error);
    let expect: Vec<i64> = vm
        .output
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect();
    let got: Vec<i64> = nvcc_run(&cu, "linint")
        .iter()
        .map(|s| s.parse().unwrap())
        .collect();
    assert_eq!(
        got, expect,
        "fused elementwise chain on the GPU must equal the VM exactly (int)"
    );
    println!(
        "Fusion: a 3-op elementwise chain ran as ONE fused kernel on the GPU ≡ VM {expect:?}."
    );
}

#[test]
fn the_safety_program_equals_the_vm_on_the_gpu() {
    let cu = compile_to_cuda_forward(SAFETY).expect("must lower");
    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP safety GPU run: needs nvcc + a live GPU. (Materialization asserted structurally.)");
        return;
    }
    let vm = vm_outcome(SAFETY);
    assert!(vm.error.is_none(), "VM error: {:?}", vm.error);
    let expect: Vec<f64> = vm
        .output
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect();
    assert_eq!(expect.len(), 4, "VM must produce 4 outputs, got {expect:?}");
    let got: Vec<f32> = nvcc_run(&cu, "safety")
        .iter()
        .map(|s| s.parse().unwrap())
        .collect();
    assert_eq!(got.len(), 4, "expected 4 outputs");
    for (g, r) in got.iter().zip(&expect) {
        assert!(
            (*g as f64 - r).abs() < 1e-3,
            "fused+materialized program on GPU {g} vs VM {r} — the guard must preserve E"
        );
    }
    println!("Fusion safety: E survived a non-elementwise stage (materialized) while F+G fused — GPU ≡ VM.");
}

/// A diamond of integer elementwise ops `S=A+B; P=A∘B; Y=S+P` over 1×4 Int tensors: two arms off the same operands,
/// joined.
const DIAMOND_INT: &str = "## Main\n\
    Let A be a 1 by 4 tensor of Int.\n\
    Set item 1 of A to 1.\nSet item 2 of A to 2.\nSet item 3 of A to 3.\nSet item 4 of A to 4.\n\
    Let B be a 1 by 4 tensor of Int.\n\
    Set item 1 of B to 5.\nSet item 2 of B to 6.\nSet item 3 of B to 7.\nSet item 4 of B to 8.\n\
    Let S be A plus B.\n\
    Let P be A hadamard B.\n\
    Let Y be S plus P.\n\
    Show item 1 of Y.\nShow item 2 of Y.\nShow item 3 of Y.\nShow item 4 of Y.\n";

/// A real diamond with a SiLU on one arm whose other arm a later op reads: `S=A+B; T=silu(S); P=A∘B; Y=T∘P; Z=P+A`.
/// The region writes `P` out beside `Y` for `Z`.
const DIAMOND_KEPT: &str = "## Main\n\
    Let A be a 1 by 4 tensor of Real.\n\
    Set item 1 of A to 0.5.\nSet item 2 of A to -1.25.\nSet item 3 of A to 2.0.\nSet item 4 of A to 0.75.\n\
    Let B be a 1 by 4 tensor of Real.\n\
    Set item 1 of B to 1.5.\nSet item 2 of B to 0.25.\nSet item 3 of B to -0.5.\nSet item 4 of B to 3.0.\n\
    Let S be A plus B.\n\
    Let T be silu of S.\n\
    Let P be A hadamard B.\n\
    Let Y be T hadamard P.\n\
    Let Z be P plus A.\n\
    Show item 1 of Y.\nShow item 2 of Y.\nShow item 3 of Y.\nShow item 4 of Y.\n\
    Show item 1 of Z.\nShow item 2 of Z.\nShow item 3 of Z.\nShow item 4 of Z.\n";

/// A connected region of elementwise ops — two arms off one pair of operands, joined — is ONE kernel
/// (PARITY-ELEMENTWISE-REGIONS): no per-op kernel and no chain survives, and every arm lives in registers.
#[test]
fn a_diamond_shaped_pointwise_region_fuses_into_one_kernel() {
    let cu = compile_to_cuda_forward(DIAMOND_INT).expect("the diamond must lower");
    assert!(cu.contains("k0_region"), "the diamond is one region kernel:\n{cu}");
    assert!(!cu.contains("_elem") && !cu.contains("_fused"), "no per-op kernel and no chain beside the region:\n{cu}");
    assert!(!cu.contains("buf_S") && !cu.contains("buf_P"), "the arms live in registers, never in buffers:\n{cu}");
}

/// A region whose arm a later op reads writes that arm out beside its result, and the later op reads it there: one
/// region kernel with two outputs, then `Z`'s kernel reading the written `P`.
#[test]
fn a_region_writes_out_an_arm_a_later_op_reads() {
    let cu = compile_to_cuda_forward(DIAMOND_KEPT).expect("the diamond must lower");
    assert!(cu.contains("k0_region"), "the diamond is one region kernel:\n{cu}");
    assert!(cu.contains("buf_P") && cu.contains("buf_Y"), "the region writes `P` out beside `Y`:\n{cu}");
    assert!(!cu.contains("buf_S") && !cu.contains("buf_T"), "the arms nobody else reads live in registers:\n{cu}");
}

#[test]
fn the_diamond_regions_equal_the_vm_on_the_gpu() {
    let int = compile_to_cuda_forward(DIAMOND_INT).expect("must lower");
    let kept = compile_to_cuda_forward(DIAMOND_KEPT).expect("must lower");
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let vm = vm_outcome(DIAMOND_INT);
    assert!(vm.error.is_none(), "VM error: {:?}", vm.error);
    let expect: Vec<i64> = vm.output.split_whitespace().map(|s| s.parse().unwrap()).collect();
    let got: Vec<i64> = nvcc_run(&int, "diamond").iter().map(|s| s.parse().unwrap()).collect();
    assert_eq!(got, expect, "the integer region on the GPU equals the VM exactly");
    let vm = vm_outcome(DIAMOND_KEPT);
    assert!(vm.error.is_none(), "VM error: {:?}", vm.error);
    let shown: Vec<f64> = vm.output.split_whitespace().map(|s| s.parse().unwrap()).collect();
    let z = &shown[4..];
    let got: Vec<f32> = nvcc_run(&kept, "diamond-kept").iter().map(|s| s.parse().unwrap()).collect();
    assert_eq!(got.len(), 4, "the forward prints its final tensor, Z: {got:?}");
    for (g, r) in got.iter().zip(z) {
        assert!((*g as f64 - r).abs() < 1e-3, "Z, read from the arm the region wrote out, on the GPU {g} vs the VM {r}");
    }
}
