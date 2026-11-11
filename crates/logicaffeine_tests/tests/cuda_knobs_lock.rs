//! cuda_knobs_lock — the CUDA backend's optimization **knob lattice**, held to its design principle:
//!
//!   > Every optimization is a trade-off → it gets a KNOB. If a transform is *strictly better*
//!   > (Pareto-dominant, no downside for any valid program) it is baked in with NO knob. Every knob
//!   > defaults to the best GENERAL case, and records the regime where it wins and where it loses.
//!
//! This lock makes that principle an *invariant*, not a comment:
//!   • the defaults ARE the best-general point (int4 loads, always-correct precision, gen-time wins on, two-pass
//!     int8 activations);
//!   • every knob has a documented, non-empty trade-off (a real win AND a real loss);
//!   • the strictly-better transforms are a non-empty list DISJOINT from the knobs (never knobbed);
//!   • `Auto` precision resolves to the always-correct path (opt into the fast path, never default into it);
//!   • flipping the `precision` knob genuinely changes the emitted kernel;
//!   • (nvcc + GPU) BOTH precision paths run on the RTX 3060 and equal the VM on an int8-safe program.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(cuda_knobs_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::knobs::ResolvedPrecision;
use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
use logicaffeine_compile::codegen_cuda::{
    lower_matmul_to_cuda_with, CudaKnobs, LoadWidth, MatmulPrecision, STRICTLY_BETTER,
};
use logicaffeine_compile::compile::{
    compile_to_cuda_matmul, compile_to_cuda_matmul_with, vm_outcome,
};

/// The knob fields, by name — the single source of truth the trade-off table and the strictly-better list
/// are both checked against. If a `CudaKnobs` field is added, add it here (and the compiler forces the
/// tradeoffs table to match, below).
const KNOB_FIELDS: &[&str] = &[
    "gemv_load",
    "precision",
    "kv_cache",
    "cuda_graphs",
    "fuse_quant",
    "chip",
    "activations",
    "recurrent_state",
    "cache_precision",
    "logits",
];

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
    Push (p % 5) - 2 to b.
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

#[test]
fn defaults_are_the_best_general_case() {
    let d = CudaKnobs::default();
    // Wide loads (measured best-general), always-correct precision, and the generation-time wins enabled.
    assert_eq!(
        d.gemv_load,
        LoadWidth::Int4,
        "default load width must be the measured-best Int4"
    );
    assert_eq!(
        d.precision,
        MatmulPrecision::Auto,
        "default precision must be Auto (⇒ always-correct)"
    );
    assert!(d.kv_cache, "KV-cache defaults on");
    assert!(d.cuda_graphs, "CUDA-graph replay defaults on");
    assert!(d.fuse_quant, "quant fusion defaults on");
    assert_eq!(
        d.activations,
        Q8MatKind::TwoPassInt8,
        "a projection's activations default to the two-word int8 lane: the weight-byte speed, the f32 run's walk"
    );
}

#[test]
fn every_knob_documents_a_real_tradeoff() {
    let table = CudaKnobs::tradeoffs();
    // One row per knob field, in the same set (a knob with no trade-off row, or a phantom row, fails here).
    let mut named: Vec<&str> = table.iter().map(|t| t.knob).collect();
    named.sort_unstable();
    let mut expect: Vec<&str> = KNOB_FIELDS.to_vec();
    expect.sort_unstable();
    assert_eq!(
        named, expect,
        "the trade-off table must have exactly one row per CudaKnobs field"
    );

    // Each trade-off must be REAL: a non-empty win AND a non-empty loss (that's *why* it's a knob, not baked in).
    for t in table {
        assert!(
            !t.default.trim().is_empty(),
            "knob `{}` must state its default",
            t.knob
        );
        assert!(
            !t.wins.trim().is_empty(),
            "knob `{}` must state where it WINS",
            t.knob
        );
        assert!(
            !t.loses.trim().is_empty(),
            "knob `{}` must state where it LOSES (else it'd be baked in)",
            t.knob
        );
    }
}

#[test]
fn strictly_better_transforms_are_baked_in_not_knobs() {
    assert!(
        !STRICTLY_BETTER.is_empty(),
        "there must be strictly-better transforms we always apply"
    );
    for s in STRICTLY_BETTER {
        assert!(
            !s.trim().is_empty(),
            "each strictly-better entry must be described"
        );
        // A strictly-better transform must NOT masquerade as a knob — it has no downside, so no toggle.
        for knob in KNOB_FIELDS {
            assert!(
                !s.contains(knob),
                "strictly-better `{s}` names knob `{knob}` — a Pareto-dominant transform must not be a knob"
            );
        }
    }
}

#[test]
fn auto_precision_resolves_to_the_correct_general_path() {
    // The trade-off (dp4a) is opt-IN; you never DEFAULT into a restricted-domain fast path.
    assert_eq!(
        MatmulPrecision::Auto.resolved(),
        ResolvedPrecision::Int64,
        "Auto ⇒ always-correct int64"
    );
    assert_eq!(MatmulPrecision::Int64.resolved(), ResolvedPrecision::Int64);
    assert_eq!(
        MatmulPrecision::Int8Dp4a.resolved(),
        ResolvedPrecision::Int8Dp4a,
        "opt-in fast path is honored"
    );
}

#[test]
fn toggling_the_precision_knob_changes_the_kernel() {
    let (spec, _cu) = compile_to_cuda_matmul(&gemm_program(16, 16, 16)).unwrap();
    let int64 = lower_matmul_to_cuda_with(
        &spec,
        &CudaKnobs {
            precision: MatmulPrecision::Int64,
            ..Default::default()
        },
    );
    let dp4a = lower_matmul_to_cuda_with(
        &spec,
        &CudaKnobs {
            precision: MatmulPrecision::Int8Dp4a,
            ..Default::default()
        },
    );
    assert_ne!(
        int64, dp4a,
        "flipping precision must produce a different kernel"
    );
    assert!(
        dp4a.contains("__dp4a"),
        "the Int8Dp4a path must actually emit __dp4a"
    );
    assert!(!int64.contains("__dp4a"), "the general path must not");
    // Default routes to the always-correct general kernel (Auto ⇒ int64).
    let def = lower_matmul_to_cuda_with(&spec, &CudaKnobs::default());
    assert_eq!(
        def, int64,
        "the default (Auto) kernel is the always-correct general one"
    );
}

#[test]
fn the_public_compile_entry_is_driven_by_the_knobs() {
    // "With knobs" end-to-end: the SAME source compiled under different knob points yields different kernels,
    // straight from the public entrypoint the CLI/API call — the lattice is user-drivable, not just internal.
    let src = gemm_program(16, 16, 16);
    let (_s_def, cu_def) = compile_to_cuda_matmul(&src).unwrap();
    let (_s_gen, cu_gen) = compile_to_cuda_matmul_with(
        &src,
        &CudaKnobs {
            precision: MatmulPrecision::Int64,
            ..Default::default()
        },
    )
    .unwrap();
    let (_s_fast, cu_fast) = compile_to_cuda_matmul_with(
        &src,
        &CudaKnobs {
            precision: MatmulPrecision::Int8Dp4a,
            ..Default::default()
        },
    )
    .unwrap();

    assert_eq!(
        cu_def, cu_gen,
        "the no-knob entry must equal the explicit best-general (Auto ⇒ Int64) point"
    );
    assert!(
        cu_fast.contains("__dp4a"),
        "the Int8Dp4a knob must drive the dp4a kernel through the public entry"
    );
    assert_ne!(
        cu_gen, cu_fast,
        "different knob points must yield different kernels from the public entry"
    );
    // A non-matmul is still a typed refusal regardless of knobs.
    assert!(
        compile_to_cuda_matmul_with("## Main\n    Show 6 * 7.\n", &CudaKnobs::default()).is_err()
    );
}

fn have_nvcc() -> bool {
    std::process::Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
fn have_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
}

fn run_cu(cu: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("logos_cuda_knob_{}_{tag}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let (s, b) = (dir.join("m.cu"), dir.join("m"));
    std::fs::write(&s, cu).unwrap();
    let c = std::process::Command::new("nvcc")
        .args(["-O2", "-arch=sm_86", "-o"])
        .arg(&b)
        .arg(&s)
        .output()
        .unwrap();
    assert!(
        c.status.success(),
        "[{tag}] emitted CUDA must compile:\n{}\n\n{cu}",
        String::from_utf8_lossy(&c.stderr)
    );
    let r = std::process::Command::new(&b).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&r.stdout).trim().to_string()
}

#[test]
fn both_precision_paths_equal_the_vm_on_the_gpu() {
    if !(have_nvcc() && have_gpu()) {
        eprintln!("SKIP knob GPU exec: needs nvcc + a GPU driver (/dev/nvidia0).");
        return;
    }
    // These dims + fill are int8-safe (operands in [-3,3]/[-2,2]; contraction fits int32), so BOTH the
    // always-correct int64 path and the dp4a fast path must equal the VM. That's the invariant that lets the
    // fast path be a *sound* knob: on its valid domain it changes speed, never the answer.
    for (m, n, k) in [(16usize, 16usize, 16usize), (24, 32, 40), (8, 64, 48)] {
        let src = gemm_program(m, n, k);
        let want = vm_outcome(&src).output.trim().to_string();
        let (spec, _cu) = compile_to_cuda_matmul(&src).unwrap();
        for prec in [MatmulPrecision::Int64, MatmulPrecision::Int8Dp4a] {
            let cu = lower_matmul_to_cuda_with(
                &spec,
                &CudaKnobs {
                    precision: prec,
                    ..Default::default()
                },
            );
            let got = run_cu(&cu, &format!("{m}x{n}x{k}_{prec:?}"));
            assert_eq!(got, want, "({m}×{n}×{k}) {prec:?} must equal the VM");
        }
    }
}
