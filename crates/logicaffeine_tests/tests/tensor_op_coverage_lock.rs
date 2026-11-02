//! tensor_op_coverage_lock — the PIT-OF-SUCCESS ratchet for LOGOS tensor ops across every backend.
//!
//! Rule 14, made mechanical. Every tensor-sugar op — `matmul`, elementwise `plus`/`hadamard`, and the float
//! reductions `rmsnorm`/`softmax` — must run on EVERY backend we ship, or be listed as an explicit, reasoned
//! gap. The gap list is **shrink-only**, enforced two ways so the matrix can only march toward "every op on
//! every backend":
//!
//!   • a cell that is NOT a known gap MUST work (≡ the VM for the executing tiers; lowers/recognizes for the
//!     codegen tiers) — a regression that drops a backend fails RED;
//!   • a cell that IS a known gap MUST actually fail — the moment an op starts working on a gapped backend
//!     (someone wrote the recognizer), the lock DEMANDS the gap be removed. You cannot leave a stale gap
//!     masking real coverage, and you cannot silently regress covered coverage.
//!
//! The VM is the cross-tier ORACLE: each executing backend must match the VM's stdout on the very same
//! program. That lets the float reductions (whose output is `round(y·1e6)`, cross-tier-deterministic) sit in
//! the same matrix as the integer ops. `KNOWN_GAPS` is EMPTY and `the_matrix_has_no_missing_cells` keeps it
//! so. (AOT-Rust parity — a real cargo build per op — is locked separately in `tensor_sugar_all_backends_lock`
//! and `tensor_reduction_ops_lock`; the seven backends here are pure-Rust or light native toolchains.)
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(tensor_op_coverage_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_riscv::RISCV_RUNTIME_C;
use logicaffeine_compile::compile::{
    compile_to_c, compile_to_cuda, compile_to_cuda_tensor_op, compile_to_riscv,
    compile_to_tenstorrent, compile_to_tenstorrent_tensor_op, compile_to_wasm, tw_outcome,
    vm_outcome,
};

/// The probe programs live in `common/tensor_probes.rs` (D4: one table, included per lock via `#[path]`),
/// so this matrix and the spatial stage ledger (`spatial_op_gap_ledger_lock`) can never disagree on which
/// programs "the ops" are. The matrix runs `BACKEND_MATRIX_PROBES`; the pending sugars (`row_of`/`argmax`/
/// `set_row`, B-P5/B-P6) and the fragment-miscompiled `silu` are the ledger's business until they parse /
/// are recognized by name.
#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;
use tensor_probes::OpProbe as TensorOpProbe;

const TENSOR_OPS: &[TensorOpProbe] = tensor_probes::BACKEND_MATRIX_PROBES;

/// The seven backends this ratchet holds. Executing tiers compare stdout ≡ the VM; codegen tiers assert the
/// backend lowers/recognizes the program.
const BACKENDS: &[&str] = &[
    "tree-walker",
    "vm",
    "aot-c",
    "riscv",
    "wasm",
    "cuda",
    "tenstorrent",
];

/// SHRINK-ONLY gaps — a `(op, backend)` that does not yet work, each with an honest reason. **Empty is the
/// goal, and the goal is met:** every tensor op (matmul, elementwise, and the float reductions) runs on or is
/// recognized by every backend. A new op that cannot yet reach a backend goes here with a reason; the
/// `the_matrix_has_no_missing_cells` lock keeps this list empty until then.
const KNOWN_GAPS: &[(&str, &str)] = &[];

#[derive(Debug, PartialEq)]
enum Verdict {
    Works,
    Fails,
    Skipped,
}

fn have(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
fn norm(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

fn run_c(program: &str) -> Option<Vec<String>> {
    let c = compile_to_c(program).ok()?;
    let dir = std::env::temp_dir().join(format!(
        "logos_topcov_c_{}_{:x}",
        std::process::id(),
        c.len()
    ));
    std::fs::create_dir_all(&dir).ok()?;
    let (src, bin) = (dir.join("o.c"), dir.join("o"));
    std::fs::write(&src, &c).ok()?;
    let compiled = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&src)
        .arg("-lm")
        .output()
        .ok()?;
    let out = compiled
        .status
        .success()
        .then(|| {
            std::process::Command::new(&bin)
                .output()
                .ok()
                .filter(|r| r.status.success())
                .map(|r| norm(&String::from_utf8_lossy(&r.stdout)))
        })
        .flatten();
    let _ = std::fs::remove_dir_all(&dir);
    out
}

fn run_riscv(program: &str) -> Option<Vec<String>> {
    let asm = compile_to_riscv(program).ok()?;
    let dir = std::env::temp_dir().join(format!(
        "logos_topcov_rv_{}_{:x}",
        std::process::id(),
        asm.len()
    ));
    std::fs::create_dir_all(&dir).ok()?;
    let (a, rt, bin) = (dir.join("m.s"), dir.join("rt.c"), dir.join("m"));
    std::fs::write(&a, &asm).ok()?;
    std::fs::write(&rt, RISCV_RUNTIME_C).ok()?;
    let c = std::process::Command::new("riscv64-linux-gnu-gcc")
        .args(["-O2", "-static", "-o"])
        .arg(&bin)
        .arg(&a)
        .arg(&rt)
        .arg("-lm")
        .output()
        .ok()?;
    let out = c
        .status
        .success()
        .then(|| {
            std::process::Command::new("qemu-riscv64")
                .arg(&bin)
                .output()
                .ok()
                .filter(|r| r.status.success())
                .map(|r| norm(&String::from_utf8_lossy(&r.stdout)))
        })
        .flatten();
    let _ = std::fs::remove_dir_all(&dir);
    out
}

/// The verdict for one (op, backend) cell. The VM is the cross-tier ORACLE: each executing tier must match its
/// stdout; codegen tiers assert the program lowers/recognizes. Toolchain-gated tiers `Skip` when tools absent.
fn verdict(p: &TensorOpProbe, backend: &str) -> Verdict {
    let oracle = norm(&vm_outcome(p.program).output);
    let ok = |got: Vec<String>| {
        if got == oracle {
            Verdict::Works
        } else {
            Verdict::Fails
        }
    };
    match backend {
        "tree-walker" => ok(norm(&tw_outcome(p.program).output)),
        "vm" => Verdict::Works, // the oracle itself
        "aot-c" => {
            if !have("gcc") {
                Verdict::Skipped
            } else {
                run_c(p.program).map(ok).unwrap_or(Verdict::Fails)
            }
        }
        "riscv" => {
            if !(have("riscv64-linux-gnu-gcc") && have("qemu-riscv64")) {
                Verdict::Skipped
            } else {
                run_riscv(p.program).map(ok).unwrap_or(Verdict::Fails)
            }
        }
        // Codegen tiers: pure-Rust predicates, no external toolchain — always evaluated.
        //
        // The cell asks the same question as every other column — CAN THIS BACKEND RUN THE PROGRAM — and an
        // accelerator has two ways to say yes: a SPECIALIZED single-kernel lowering (`*_tensor_op`, which
        // recognizes the op and emits a hand-tuned kernel) or the GENERAL full-language lowering, which
        // compiles the desugared loops like any other LOGOS program. Both genuinely execute on the device, so
        // both count here. Which ops additionally have a *fused* kernel is a finer question, and conflating it
        // with "runs at all" would have made this matrix unanswerable for `attention` and `moe` — ops that have
        // shipped on GPUs for two campaigns but lower through the forward path, not `*_tensor_op`. That second
        // question gets its own monotone ledger below (`FUSED_KERNEL_OPS`).
        "wasm" => {
            if compile_to_wasm(p.program).is_ok() {
                Verdict::Works
            } else {
                Verdict::Fails
            }
        }
        "cuda" => {
            if compile_to_cuda_tensor_op(p.program).is_ok() || compile_to_cuda(p.program).is_ok() {
                Verdict::Works
            } else {
                Verdict::Fails
            }
        }
        "tenstorrent" => {
            if compile_to_tenstorrent_tensor_op(p.program).is_ok()
                || compile_to_tenstorrent(p.program).is_ok()
            {
                Verdict::Works
            } else {
                Verdict::Fails
            }
        }
        other => panic!("unknown backend `{other}`"),
    }
}

fn is_gap(op: &str, backend: &str) -> bool {
    KNOWN_GAPS.contains(&(op, backend))
}

/// Ops that have a SPECIALIZED single-kernel CUDA lowering today, i.e. `compile_to_cuda_tensor_op` recognizes
/// them and emits a hand-tuned kernel rather than the general loop lowering. This list only ever GROWS: the
/// test below fails if a listed op stops having one (a fusion regression) AND names any op that has quietly
/// gained one, so the ledger cannot drift out of date in either direction. An entry stands for a kernel that
/// computes the op it names: a fragment of the op's desugar lowered as another op's kernel is a miscompile
/// (the spatial ledger's `MISRECOGNIZED` trap), never an entry.
///
/// The Qwen3.8 ops (`gated_delta`, `causal_conv`, `full_attention`, …) are deliberately absent: they run on
/// the GPU today through the general lowering, and their fused kernels are the throughput work. When that
/// lands, this list grows and the assertion below is what forces the update.
const FUSED_KERNEL_OPS: &[&str] = &[
    "matmul",
    "plus",
    "hadamard",
    "rmsnorm",
    "softmax",
    "rmsnorm_weighted",
    "fused_add_rmsnorm",
    "layernorm",
    "swiglu",
    "softmax_rows",
];

#[test]
fn the_fused_kernel_ledger_matches_reality() {
    let mut lost = Vec::new();
    let mut gained = Vec::new();
    for p in TENSOR_OPS {
        let fused = compile_to_cuda_tensor_op(p.program).is_ok();
        match (FUSED_KERNEL_OPS.contains(&p.op), fused) {
            (true, false) => lost.push(format!(
                "{} lost its specialized CUDA kernel (fusion regression)",
                p.op
            )),
            (false, true) => gained.push(format!(
                "{} gained a specialized CUDA kernel — add it to FUSED_KERNEL_OPS",
                p.op
            )),
            _ => {}
        }
    }
    assert!(
        lost.is_empty() && gained.is_empty(),
        "fused-kernel ledger is stale:\n{}",
        [lost, gained].concat().join("\n")
    );
}

/// REGRESSION LOCK for a silent miscompile the fused-kernel ledger uncovered: `compile_to_cuda_tensor_op`
/// used to ACCEPT a Mixture-of-Experts program and return a `fused_row` **softmax** kernel — it had matched the
/// softmax over the router's E logits, a fragment nested inside the block, and reported success while
/// computing essentially none of the MoE. Composite ops must be refused here and lowered through the forward
/// path instead, so a caller gets an error rather than a plausible, wrong kernel.
#[test]
fn the_single_op_cuda_path_refuses_composite_blocks_instead_of_lowering_a_fragment() {
    for op in ["moe", "attention"] {
        let p = TENSOR_OPS
            .iter()
            .find(|p| p.op == op)
            .expect("probe exists");
        let got = compile_to_cuda_tensor_op(p.program);
        assert!(
            got.is_err(),
            "compile_to_cuda_tensor_op ACCEPTED the composite `{op}` program and emitted:\n{}",
            got.unwrap_or_default()
                .lines()
                .take(4)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

#[test]
fn the_vm_computes_the_known_answers() {
    // Independent correctness anchor for the integer ops: the oracle itself is right, not just self-consistent.
    for p in TENSOR_OPS {
        if let Some(expect) = p.expect {
            assert_eq!(
                norm(&vm_outcome(p.program).output),
                expect,
                "{} on the VM oracle",
                p.op
            );
        }
    }
}

#[test]
fn every_probe_actually_computes_something_on_the_oracle() {
    // TEETH FOR THE ORACLE ITSELF. `verdict` scores a backend by comparing its stdout to the VM's, so a probe
    // that fails to PARSE is silently unanimous: every executing tier prints nothing, the comparison passes,
    // and a completely broken op reads as covered on 4 of 7 backends. A probe must really run — no VM error,
    // and exactly one value per `Show`. Without this the matrix can only catch DISAGREEMENT, not absence.
    let mut bad = Vec::new();
    for p in TENSOR_OPS {
        let out = vm_outcome(p.program);
        if let Some(err) = out.error.as_deref() {
            bad.push(format!(
                "{}: VM errored: {}",
                p.op,
                err.lines().next().unwrap_or(err)
            ));
            continue;
        }
        let (got, want) = (norm(&out.output).len(), p.program.matches("Show ").count());
        if got != want {
            bad.push(format!(
                "{}: VM printed {got} values but the program has {want} `Show`s",
                p.op
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "probes that do not actually execute on the oracle:\n{}",
        bad.join("\n")
    );
}

#[test]
fn every_tensor_op_runs_on_every_backend_or_is_a_known_gap() {
    let mut violations = Vec::new();
    for p in TENSOR_OPS {
        for &backend in BACKENDS {
            if is_gap(p.op, backend) {
                continue;
            }
            match verdict(p, backend) {
                Verdict::Works | Verdict::Skipped => {}
                Verdict::Fails => violations.push(format!(
                    "{} on {backend}: FAILS but is not a known gap",
                    p.op
                )),
            }
        }
    }
    assert!(
        violations.is_empty(),
        "tensor-op coverage regressed:\n{}",
        violations.join("\n")
    );
}

#[test]
fn no_known_gap_masks_a_backend_that_actually_works() {
    // Shrink-only teeth: a gap that has started working must be REMOVED.
    let mut fixed = Vec::new();
    for &(op, backend) in KNOWN_GAPS {
        let p = TENSOR_OPS
            .iter()
            .find(|p| p.op == op)
            .unwrap_or_else(|| panic!("gap names unknown op `{op}`"));
        if verdict(p, backend) == Verdict::Works {
            fixed.push(format!(
                "{op} on {backend} now WORKS — delete its KNOWN_GAPS row (gaps only shrink)"
            ));
        }
    }
    assert!(
        fixed.is_empty(),
        "stale gaps mask real coverage:\n{}",
        fixed.join("\n")
    );
}

#[test]
fn the_matrix_has_no_missing_cells() {
    // "We don't want missing things." Every tensor op reaches every backend — the gap list is EMPTY, and this
    // lock keeps it that way: adding an op that can't reach a backend (a new gap) fails here until it's closed.
    assert!(
        KNOWN_GAPS.is_empty(),
        "the tensor-op × backend matrix has gaps {KNOWN_GAPS:?} — close them (add the recognizer/emitter), don't ship holes"
    );
}

#[test]
fn known_gaps_reference_real_ops_and_backends() {
    for &(op, backend) in KNOWN_GAPS {
        assert!(
            TENSOR_OPS.iter().any(|p| p.op == op),
            "KNOWN_GAPS names unknown op `{op}`"
        );
        assert!(
            BACKENDS.contains(&backend),
            "KNOWN_GAPS names unknown backend `{backend}`"
        );
    }
}
