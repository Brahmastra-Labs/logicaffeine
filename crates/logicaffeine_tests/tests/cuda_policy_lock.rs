//! ★ THE C / CUDA SECURITY-POLICY CHECK — PIT OF SUCCESS ★ (RED-first, TDD).
//!
//! A `Check that <subject> is <predicate>` guards execution: if the subject fails the policy predicate
//! the program ERRORS (`Security Check Failed: …`), exactly like the tree-walker/VM oracle
//! (`semantics::policy::check_policy`). This lock proves the AOT-C backend lowers that guard FAITHFULLY —
//! an allowed check runs on to print, a denied one traps before the guarded output — and that the same
//! guard survives the CUDA device-lift onto the GPU (`fprintf+exit` → PTX trap), byte-for-byte ≡VM.
//!
//! Scope: PREDICATE checks (`is <predicate>`), the ≡VM-provable form — the condition is `FieldEquals`
//! (strcmp/`==` over a struct field, `is_string_literal`-driven), `FieldBool`, a named `Predicate`, and
//! `And`/`Or` of those. Capability checks (`can <action> the <object>`) resolve their object by TYPE and
//! error in BOTH oracles through this compile path, so they are not ≡VM-exercisable here; codegen refuses
//! any not-yet-lowerable condition form LOUDLY (a C `#error`), never a silent no-op. This mirrors
//! `riscv_aot_lock`'s `CheckPolicy => S` (also a predicate-only corpus).
//!
//! Discipline (mirrors `c_temporal_lock`):
//! - The oracle pair (`tw_outcome` + `vm_outcome`) AGREES on every corpus program (allow → output, deny →
//!   error) so the target is unambiguous.
//! - An `allow` program compiles to C and prints ≡ the oracle; a `deny` program compiles, then EXITS
//!   NONZERO without emitting the guarded output — proving the check actually gates (not a silent no-op).
//! - The emitted C literally contains the guard (`Security Check Failed`) — anti-floor-gaming.
//! - The same holds on the GPU: allow prints ≡VM; deny traps (no guarded output).
//!
//! ⚠️ RED tests are the spec: if a program fails ≡VM, fix `codegen_c`, NEVER weaken this file. The C
//! execution half is gated on `gcc`; the GPU half on `nvcc`+`/dev/nvidia0`. Both SKIP when absent.
#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::compile::{compile_to_c, compile_to_cuda, tw_outcome, vm_outcome};

/// An ALLOWED predicate check: the subject satisfies two predicates — a plain `FieldEquals` (`admin`) and
/// an `Or` of two `FieldEquals` (`staff`) — so both guards pass and the guarded `Show` runs. Mirrors
/// `riscv_aot_lock`'s `policy_allowed`, so the two AOT tiers exercise the identical predicate program.
const ALLOW_SRC: &str = "## Definition\nA User has:\n    a role, which is Text.\n\n\
## Policy\nA User is admin if the user's role equals \"admin\".\n\
A User is staff if the user's role equals \"admin\" or the user's role equals \"mod\".\n\n\
## To process (u: User):\n    Check that the u is admin.\n    Check that the u is staff.\n    Show \"allowed\".\n\n\
## Main\nLet u be a new User with role \"admin\".\nprocess(u).\n";

/// A DENIED predicate check: the subject fails the policy, so the program errors BEFORE the guarded
/// `Show`. The sentinel `reached` must never be printed.
const DENY_SRC: &str = "## Definition\nA User has:\n    a role, which is Text.\n\n\
## Policy\nA User is admin if the user's role equals \"admin\".\n\n\
## Main\nLet u be a new User with role \"guest\".\nCheck that the u is admin.\nShow \"reached\".\n";

fn have_gcc() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn have_nvcc_gpu() -> bool {
    std::path::Path::new("/dev/nvidia0").exists()
        && std::process::Command::new("nvcc")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

/// Compile to C, build with gcc, run. Returns `(compiled, exit_ok, trimmed_stdout)`:
/// `compiled=false` means the C failed to compile (a hard failure for any corpus program); otherwise
/// `exit_ok` is the process success and the stdout is captured (empty allowed).
fn c_run(src: &str, label: &str) -> (bool, bool, String) {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let c = compile_to_c(src).expect("compile_to_c");
    let dir = std::env::temp_dir().join(format!(
        "logos_cpolicy_{}_{}_{}",
        std::process::id(),
        label,
        n
    ));
    let _ = std::fs::create_dir_all(&dir);
    let cfile = dir.join("prog.c");
    let bin = dir.join("prog");
    std::fs::write(&cfile, &c).expect("write");
    let build = std::process::Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&cfile)
        .arg("-lm")
        .output()
        .expect("gcc");
    if !build.status.success() {
        eprintln!(
            "gcc failed for {label}:\n{}",
            String::from_utf8_lossy(&build.stderr)
        );
        return (false, false, String::new());
    }
    let run = std::process::Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_dir_all(&dir);
    (
        true,
        run.status.success(),
        String::from_utf8_lossy(&run.stdout).trim().to_string(),
    )
}

/// nvcc-compile a `.cu` and run it on the GPU. Returns `(compiled, exit_ok, trimmed_stdout)` — same shape
/// as `c_run`. `-fmad=false` matches the VM's non-fused IEEE semantics (irrelevant here, kept for parity).
fn cu_run(cu: &str, label: &str) -> (bool, bool, String) {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "logos_cudapolicy_{}_{}_{}",
        std::process::id(),
        label,
        n
    ));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("k.cu");
    let bin = dir.join("k");
    std::fs::write(&src, cu).expect("write cu");
    let c = std::process::Command::new("nvcc")
        .args(["-O2", "-arch=sm_86", "-fmad=false", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect("nvcc");
    if !c.status.success() {
        eprintln!(
            "nvcc failed for {label}:\n{}",
            String::from_utf8_lossy(&c.stderr)
                .lines()
                .take(4)
                .collect::<Vec<_>>()
                .join("\n")
        );
        return (false, false, String::new());
    }
    let r = std::process::Command::new(&bin).output().expect("run cu");
    let _ = std::fs::remove_dir_all(&dir);
    (
        true,
        r.status.success(),
        String::from_utf8_lossy(&r.stdout).trim().to_string(),
    )
}

// ============================================================
//  Oracle agreement — the target is unambiguous
// ============================================================

#[test]
fn the_policy_oracles_agree() {
    // allow → output "allowed" no error; deny → error, no "reached".
    let ta = tw_outcome(ALLOW_SRC);
    let va = vm_outcome(ALLOW_SRC);
    assert_eq!(
        (ta.output.trim(), ta.error.is_some()),
        (va.output.trim(), va.error.is_some())
    );
    assert_eq!(va.output.trim(), "allowed");
    assert!(va.error.is_none());

    let td = tw_outcome(DENY_SRC);
    let vd = vm_outcome(DENY_SRC);
    assert_eq!(
        (td.output.trim(), td.error.is_some()),
        (vd.output.trim(), vd.error.is_some())
    );
    assert!(
        vd.error.is_some(),
        "a denied check must error in the VM oracle"
    );
    assert!(
        !vd.output.contains("reached"),
        "guarded output must not print on a denied check"
    );
}

// ============================================================
//  Structural — the guard is really lowered (anti-floor-gaming)
// ============================================================

#[test]
fn emitted_c_contains_the_guard() {
    let c = compile_to_c(ALLOW_SRC).expect("compile");
    // Two predicate guards (admin + staff), both real — never a silent no-op.
    assert_eq!(
        c.matches("Security Check Failed").count(),
        2,
        "both checks must lower to inline guards"
    );
    assert!(
        c.contains("strcmp((u).role, \"admin\") == 0"),
        "admin FieldEquals must lower to strcmp"
    );
    assert!(
        c.contains("(strcmp((u).role, \"admin\") == 0) || (strcmp((u).role, \"mod\") == 0)"),
        "staff Or must lower"
    );
    assert!(
        !c.contains("/* unsupported stmt */"),
        "no statement in the corpus should be unsupported"
    );
    assert!(
        !c.contains("#error"),
        "the predicate corpus must lower without a loud refusal"
    );
}

// ============================================================
//  C execution — allow prints ≡VM, deny traps
// ============================================================

#[test]
fn c_allow_prints_and_c_deny_traps() {
    if !have_gcc() {
        eprintln!("skip: gcc absent");
        return;
    }
    let (compiled_a, exit_a, out_a) = c_run(ALLOW_SRC, "allow");
    assert!(compiled_a, "allow program must compile to C");
    assert!(exit_a, "allow program must exit 0");
    assert_eq!(out_a, vm_outcome(ALLOW_SRC).output.trim());

    let (compiled_d, exit_d, out_d) = c_run(DENY_SRC, "deny");
    assert!(compiled_d, "deny program must compile to C");
    assert!(
        !exit_d,
        "the deny program must exit nonzero (the check traps)"
    );
    assert!(
        !out_d.contains("reached"),
        "the guarded Show must not run after a denied check; got {out_d:?}"
    );
}

// ============================================================
//  GPU execution — the guard survives the device-lift (gated)
// ============================================================

#[test]
fn cuda_allow_prints_and_cuda_deny_traps_on_the_gpu() {
    // The CUDA codegen must SUCCEED for both programs regardless of a GPU being present.
    let allow_art = compile_to_cuda(ALLOW_SRC).expect("allow lowers to CUDA");
    let deny_art = compile_to_cuda(DENY_SRC).expect("deny lowers to CUDA");
    assert!(
        allow_art.kernel_cu.contains("Security Check Failed"),
        "the guard must survive into the .cu"
    );

    if !have_nvcc_gpu() {
        eprintln!("SKIP: needs nvcc + /dev/nvidia0 — cannot run the policy kernel on the GPU");
        return;
    }
    let (ca, ea, oa) = cu_run(&allow_art.kernel_cu, "allow");
    assert!(ca, "allow .cu must nvcc-compile");
    assert!(ea, "allow kernel must exit 0");
    assert_eq!(oa, vm_outcome(ALLOW_SRC).output.trim(), "allow kernel ≡VM");

    let (cd, ed, od) = cu_run(&deny_art.kernel_cu, "deny");
    assert!(cd, "deny .cu must nvcc-compile");
    assert!(!ed, "the deny kernel must trap (nonzero) on a denied check");
    assert!(
        !od.contains("reached"),
        "the guarded Show must not run on the GPU after a denied check; got {od:?}"
    );
}
