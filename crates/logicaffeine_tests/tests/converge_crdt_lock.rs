//! ★ THE `Converge` (CrdtJoin) IDEMPOTENT-LATTICE-JOIN LOCK ★ (RED-first, TDD).
//!
//! `Merge <src> into <tgt>` combines struct fields with ADD for bare-`Int` fields — NOT idempotent
//! (applying it twice doubles the delta). `Converge <src> into <tgt>` is the state-based CRDT LATTICE
//! JOIN — the IDEMPOTENT sibling: a bare-`Int` field takes per-field MAX, a `ConvergentCount` (G-Counter)
//! field takes per-replica MAX (the SAME as `Merge`, already idempotent), and any other field is
//! last-writer-wins (also the same as `Merge`). So `Converge == Merge` EXCEPT bare-`Int` fields take MAX
//! instead of ADD. The single source of truth is `crate::semantics::arith::crdt_join_field`.
//!
//! This file pins FOUR properties and proves them equal across every execution tier:
//!   1. DISTINCTION      — `Converge` (max) diverges from `Merge` (add) on bare-`Int` fields.
//!   2. IDEMPOTENCY      — a repeated `Converge` is a no-op; a repeated `Merge` keeps adding.
//!   3. CROSS-TIER ≡VM   — the tree-walker, the VM, the AST-C backend, the bytecode→C Op-walker, and
//!                          the RISC-V AOT tier (when its toolchain is present) all agree with the VM.
//!   4. COUNTER PARITY   — on a `ConvergentCount` field, `Converge` and `Merge` give the same value
//!                          (both per-replica MAX / idempotent), and both stay idempotent under repeat.
//!
//! ⚠️ RED tests are the spec. If a tier disagrees, fix that tier's `Converge`/`CrdtJoin` lowering —
//! NEVER weaken this file. The gcc/riscv execution halves SKIP cleanly when their toolchain is absent;
//! the oracle-agreement + distinction + idempotency halves always run (pure VM/tree-walker).
#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::codegen_riscv::RISCV_RUNTIME_C;
use logicaffeine_compile::compile::{
    compile_to_c, compile_to_c_via_bytecode, compile_to_riscv, tw_outcome, vm_outcome,
};

// ============================================================
//  The corpus programs (Converge vs its Merge twin)
// ============================================================

/// A plain 2-field `Int` struct; `Converge b into a` takes per-field MAX. `a=(3,10)`, `b=(5,2)` → `(5,10)`.
const CONVERGE_ONCE: &str = "## Definition\nA Vec has:\n    x: Int.\n    y: Int.\n## Main\n\
    Let mutable a be a new Vec with x 3 and y 10.\n\
    Let mutable b be a new Vec with x 5 and y 2.\n\
    Converge b into a.\n\
    Show a's x.\n\
    Show a's y.\n";

/// The `Merge` twin of `CONVERGE_ONCE`: ADD → `a=(3+5, 10+2)=(8,12)`.
const MERGE_ONCE: &str = "## Definition\nA Vec has:\n    x: Int.\n    y: Int.\n## Main\n\
    Let mutable a be a new Vec with x 3 and y 10.\n\
    Let mutable b be a new Vec with x 5 and y 2.\n\
    Merge b into a.\n\
    Show a's x.\n\
    Show a's y.\n";

/// `Converge` applied TWICE — the whole point: MAX is idempotent, so still `(5,10)`.
const CONVERGE_TWICE: &str = "## Definition\nA Vec has:\n    x: Int.\n    y: Int.\n## Main\n\
    Let mutable a be a new Vec with x 3 and y 10.\n\
    Let mutable b be a new Vec with x 5 and y 2.\n\
    Converge b into a.\n\
    Converge b into a.\n\
    Show a's x.\n\
    Show a's y.\n";

/// `Merge` applied TWICE — ADD is NOT idempotent: `a=(3+5+5, 10+2+2)=(13,14)`.
const MERGE_TWICE: &str = "## Definition\nA Vec has:\n    x: Int.\n    y: Int.\n## Main\n\
    Let mutable a be a new Vec with x 3 and y 10.\n\
    Let mutable b be a new Vec with x 5 and y 2.\n\
    Merge b into a.\n\
    Merge b into a.\n\
    Show a's x.\n\
    Show a's y.\n";

/// A `ConvergentCount` (G-Counter) field bumped on two DISTINCT replicas, then joined — per-replica MAX.
/// `Converge` and `Merge` MUST give the same total (5 + 3 = 8), and both must be idempotent (join twice
/// is still 8). The two `Increase`s happen on separate replica structs, so a real per-replica map forms.
const CONVERGE_COUNTER: &str =
    "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n\
    Let mutable a be a new Counter.\n    Increase a's points by 5.\n\
    Let mutable b be a new Counter.\n    Increase b's points by 3.\n\
    Converge b into a.\n    Converge b into a.\n    Show a's points.\n";

/// The `Merge` twin of `CONVERGE_COUNTER`: the counter field ALSO takes per-replica MAX under `Merge`
/// (a G-Counter is already idempotent), so `Merge b into a` twice is ALSO 8 — proving parity on counters.
const MERGE_COUNTER: &str =
    "## Definition\nA Counter is Shared and has:\n    points: ConvergentCount.\n## Main\n\
    Let mutable a be a new Counter.\n    Increase a's points by 5.\n\
    Let mutable b be a new Counter.\n    Increase b's points by 3.\n\
    Merge b into a.\n    Merge b into a.\n    Show a's points.\n";

// ============================================================
//  Tier runners (gated helpers)
// ============================================================

fn have_gcc() -> bool {
    std::process::Command::new("gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn have_riscv() -> bool {
    let gcc = std::process::Command::new("riscv64-linux-gnu-gcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let qemu = std::process::Command::new("qemu-riscv64")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    gcc && qemu
}

/// Build `code` with `cc` (+`extra` flags) as `prog.<ext>`, run it, return trimmed stdout. `None` on any
/// compile/build/run failure. Mirrors `c_bytecode_concurrency_lock`'s `build_run_best_of` (`-O2 -lm`).
fn build_run(code: &str, cc: &str, extra: &[&str], ext: &str, label: &str) -> Option<String> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "logos_converge_{}_{}_{}",
        std::process::id(),
        label,
        n
    ));
    let _ = std::fs::create_dir_all(&dir);
    let cfile = dir.join(format!("prog.{ext}"));
    let bin = dir.join("prog");
    std::fs::write(&cfile, code).ok()?;
    let mut cmd = std::process::Command::new(cc);
    cmd.args(["-O2"])
        .args(extra)
        .arg("-o")
        .arg(&bin)
        .arg(&cfile)
        .arg("-lm");
    let build = cmd.output().ok()?;
    if !build.status.success() {
        eprintln!(
            "[converge] {label} build failed with {cc}:\n{}",
            String::from_utf8_lossy(&build.stderr)
        );
        return None;
    }
    let run = std::process::Command::new(&bin).output().ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    if !run.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&run.stdout).trim().to_string())
}

/// AST-C backend (`compile_to_c`) → gcc → run.
fn ast_c_output(src: &str, label: &str) -> Option<String> {
    let c = compile_to_c(src).ok()?;
    build_run(&c, "gcc", &[], "c", label)
}

/// Bytecode→C Op-walker (`compile_to_c_via_bytecode`) → g++ (the walker body is C++) → run.
fn bytecode_c_output(src: &str, label: &str) -> Option<String> {
    let c = compile_to_c_via_bytecode(src).ok()?;
    build_run(&c, "g++", &["-pthread"], "cpp", label)
}

/// RISC-V AOT (`compile_to_riscv`) → riscv64-linux-gnu-gcc (static) → qemu-riscv64 → run.
fn riscv_output(src: &str, label: &str) -> Option<String> {
    let asm = compile_to_riscv(src).ok()?;
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "logos_converge_rv_{}_{}_{}",
        std::process::id(),
        label,
        n
    ));
    let _ = std::fs::create_dir_all(&dir);
    let asm_path = dir.join("main.s");
    let rt_path = dir.join("logos_rt.c");
    let bin_path = dir.join("main");
    std::fs::write(&asm_path, &asm).ok()?;
    std::fs::write(&rt_path, RISCV_RUNTIME_C).ok()?;
    let c = std::process::Command::new("riscv64-linux-gnu-gcc")
        .args(["-O2", "-static", "-o"])
        .arg(&bin_path)
        .arg(&asm_path)
        .arg(&rt_path)
        .arg("-lm")
        .output()
        .ok()?;
    if !c.status.success() {
        eprintln!(
            "[converge] {label} riscv assemble failed:\n{}",
            String::from_utf8_lossy(&c.stderr)
        );
        return None;
    }
    let r = std::process::Command::new("qemu-riscv64")
        .arg(&bin_path)
        .output()
        .ok()?;
    let _ = std::fs::remove_dir_all(&dir);
    if !r.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&r.stdout).trim().to_string())
}

// ============================================================
//  1. DISTINCTION — Converge (max) ≠ Merge (add) on bare-Int fields
// ============================================================

#[test]
fn converge_takes_max_where_merge_adds() {
    // Converge: per-field MAX → x=max(3,5)=5, y=max(10,2)=10.
    let c = vm_outcome(CONVERGE_ONCE);
    assert!(c.error.is_none(), "Converge program errored: {:?}", c.error);
    assert_eq!(
        c.output.lines().collect::<Vec<_>>(),
        vec!["5", "10"],
        "Converge must take per-field MAX"
    );

    // Merge: per-field ADD → x=3+5=8, y=10+2=12.
    let m = vm_outcome(MERGE_ONCE);
    assert!(m.error.is_none(), "Merge program errored: {:?}", m.error);
    assert_eq!(
        m.output.lines().collect::<Vec<_>>(),
        vec!["8", "12"],
        "Merge must take per-field ADD"
    );

    // They MUST differ — that is the whole distinction.
    assert_ne!(
        c.output.trim(),
        m.output.trim(),
        "Converge (max) must diverge from Merge (add) on bare-Int fields"
    );
}

// ============================================================
//  2. IDEMPOTENCY — the whole point
// ============================================================

#[test]
fn converge_is_idempotent_but_merge_is_not() {
    // Converge twice == Converge once (5, 10) — MAX is idempotent.
    let cc = vm_outcome(CONVERGE_TWICE);
    assert!(cc.error.is_none(), "Converge-twice errored: {:?}", cc.error);
    assert_eq!(
        cc.output.lines().collect::<Vec<_>>(),
        vec!["5", "10"],
        "Converge applied twice must equal applying it once (idempotent)"
    );

    // Merge twice keeps ADDING: (3+5+5, 10+2+2) = (13, 14).
    let mm = vm_outcome(MERGE_TWICE);
    assert!(mm.error.is_none(), "Merge-twice errored: {:?}", mm.error);
    assert_eq!(
        mm.output.lines().collect::<Vec<_>>(),
        vec!["13", "14"],
        "Merge applied twice must double-count (NOT idempotent)"
    );
}

// ============================================================
//  3. CROSS-TIER ≡VM (tree-walker, AST-C, bytecode→C, RISC-V)
// ============================================================

/// Every corpus program, with its expected VM output — the unambiguous target for every tier.
fn corpus() -> Vec<(&'static str, &'static str, Vec<&'static str>)> {
    vec![
        ("converge_once", CONVERGE_ONCE, vec!["5", "10"]),
        ("merge_once", MERGE_ONCE, vec!["8", "12"]),
        ("converge_twice", CONVERGE_TWICE, vec!["5", "10"]),
        ("merge_twice", MERGE_TWICE, vec!["13", "14"]),
        ("converge_counter", CONVERGE_COUNTER, vec!["8"]),
        ("merge_counter", MERGE_COUNTER, vec!["8"]),
    ]
}

#[test]
fn the_oracles_agree_with_the_pinned_expectation() {
    // The tree-walker and the VM must agree with each other AND with the hand-pinned expectation, so
    // "≡ VM" names an unambiguous target every AOT tier is then held to.
    for (label, src, want) in corpus() {
        let tw = tw_outcome(src);
        let vm = vm_outcome(src);
        assert!(
            tw.error.is_none(),
            "`{label}` tree-walker errored: {:?}",
            tw.error
        );
        assert!(vm.error.is_none(), "`{label}` VM errored: {:?}", vm.error);
        assert_eq!(
            tw.output.lines().collect::<Vec<_>>(),
            want,
            "`{label}`: tree-walker output must equal the pinned expectation"
        );
        assert_eq!(
            vm.output.lines().collect::<Vec<_>>(),
            want,
            "`{label}`: VM output must equal the pinned expectation"
        );
    }
}

#[test]
fn ast_c_backend_agrees_with_vm() {
    if !have_gcc() {
        eprintln!("SKIP ast-c execution: gcc not installed");
        return;
    }
    for (label, src, want) in corpus() {
        let got = ast_c_output(src, label).unwrap_or_else(|| {
            panic!("`{label}` failed to compile/run on the AST-C backend — fix codegen_c's Converge lowering")
        });
        assert_eq!(
            got.lines().collect::<Vec<_>>(),
            want,
            "`{label}`: AST-C output must equal the VM oracle"
        );
    }
}

#[test]
fn bytecode_c_walker_agrees_with_vm() {
    let g_plus = std::process::Command::new("g++")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !g_plus {
        eprintln!("SKIP bytecode-c execution: g++ not installed");
        return;
    }
    for (label, src, want) in corpus() {
        let got = bytecode_c_output(src, label).unwrap_or_else(|| {
            panic!("`{label}` failed to compile/run on the bytecode→C walker — fix from_program.rs's CrdtJoin arm")
        });
        assert_eq!(
            got.lines().collect::<Vec<_>>(),
            want,
            "`{label}`: bytecode→C walker output must equal the VM oracle"
        );
    }
}

#[test]
fn riscv_aot_tier_agrees_with_vm() {
    if !have_riscv() {
        eprintln!("SKIP riscv execution: riscv64-linux-gnu-gcc / qemu-riscv64 not installed");
        return;
    }
    for (label, src, want) in corpus() {
        let got = riscv_output(src, label).unwrap_or_else(|| {
            panic!("`{label}` failed to compile/run on the RISC-V AOT tier — fix codegen_riscv's CrdtJoin arm")
        });
        assert_eq!(
            got.lines().collect::<Vec<_>>(),
            want,
            "`{label}`: RISC-V AOT output must equal the VM oracle"
        );
    }
}

// ============================================================
//  4. COUNTER-FIELD PARITY — Converge == Merge on a ConvergentCount
// ============================================================

#[test]
fn converge_equals_merge_on_a_counter_field() {
    // A G-Counter is already idempotent under BOTH ops (per-replica MAX), so a two-replica join of the
    // SAME state gives the SAME total whether you Converge or Merge, and repeating it never double-counts.
    let c = vm_outcome(CONVERGE_COUNTER);
    let m = vm_outcome(MERGE_COUNTER);
    assert!(c.error.is_none(), "converge_counter errored: {:?}", c.error);
    assert!(m.error.is_none(), "merge_counter errored: {:?}", m.error);
    assert_eq!(
        c.output.trim(),
        "8",
        "Converge of two distinct-replica counters must total per-replica MAX = 8"
    );
    assert_eq!(
        m.output.trim(),
        "8",
        "Merge of two distinct-replica counters must total per-replica MAX = 8"
    );
    assert_eq!(
        c.output.trim(),
        m.output.trim(),
        "Converge and Merge must AGREE on a ConvergentCount field (both per-replica MAX / idempotent)"
    );
}

// ============================================================
//  5. FUZZ DIFFERENTIAL — every tier agrees over the INPUT SPACE (not just the fixed corpus)
// ============================================================

/// SplitMix64 — a deterministic, seeded generator (same constants as the runtime's `SeededRng`), so the
/// fuzz corpus is reproducible run-to-run.
struct FRng(u64);
impl FRng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A random `n`-field `Int` struct joined/merged from `b` into `a`. Returns the program source and the
/// EXACTLY-computed expected output lines — `max(a_i,b_i)` per field for a join, `a_i.wrapping_add(b_i)` for
/// a merge (the VM's `crdt_join_field`/`crdt_merge_field` per-field semantics), applied `reps` times.
fn gen_program(seed: u64, is_join: bool, reps: usize) -> (String, Vec<String>) {
    // Distinct field names (namespaced under the struct, so no clash with the `a`/`b` instance vars).
    const NAMES: &[&str] = &["p", "q", "r", "s", "t", "u"];
    // Same seed → same shape/values for join & merge, so they differ ONLY in the operator.
    let mut rng = FRng(seed.wrapping_add(1));
    let n = 1 + rng.below(NAMES.len() as u64) as usize; // 1..=6 fields
                                                        // Non-negative so `with p <v>` never lexes `-v` as a subtraction; range is wide enough that MAX≠ADD.
    let av: Vec<i64> = (0..n).map(|_| rng.below(500) as i64).collect(); // [0, 499]
    let bv: Vec<i64> = (0..n).map(|_| rng.below(500) as i64).collect();

    let mut def = String::from("## Definition\nA Rec has:\n");
    for name in NAMES.iter().take(n) {
        def.push_str(&format!("    {name}: Int.\n"));
    }
    let fields_a: Vec<String> = NAMES
        .iter()
        .take(n)
        .zip(&av)
        .map(|(nm, v)| format!("{nm} {v}"))
        .collect();
    let fields_b: Vec<String> = NAMES
        .iter()
        .take(n)
        .zip(&bv)
        .map(|(nm, v)| format!("{nm} {v}"))
        .collect();
    let op = if is_join { "Converge" } else { "Merge" };
    let mut main = String::from("## Main\n");
    main.push_str(&format!(
        "Let mutable a be a new Rec with {}.\n",
        fields_a.join(" and ")
    ));
    main.push_str(&format!(
        "Let mutable b be a new Rec with {}.\n",
        fields_b.join(" and ")
    ));
    for _ in 0..reps {
        main.push_str(&format!("{op} b into a.\n"));
    }
    for name in NAMES.iter().take(n) {
        main.push_str(&format!("Show a's {name}.\n"));
    }

    // Expected: fold each field `reps` times. Join is idempotent (MAX) so reps>1 == reps==1; Merge ADDs each rep.
    let expected: Vec<String> = av
        .iter()
        .zip(&bv)
        .map(|(a, b)| {
            let mut acc = *a;
            for _ in 0..reps {
                acc = if is_join {
                    acc.max(*b)
                } else {
                    acc.wrapping_add(*b)
                };
            }
            acc.to_string()
        })
        .collect();
    (format!("{def}{main}"), expected)
}

/// EVERY tier agrees with the exactly-computed per-field expectation over a fuzzed corpus of random struct
/// shapes + values, for BOTH `Converge` (idempotent MAX) and `Merge` (ADD), applied once AND twice. The
/// tree-walker + VM always run (pure); the AST-C and bytecode→C tiers run when their compiler is present.
/// This is the "make SURE all things agree" robustness lock — the fixed corpus proves the shapes, the fuzzer
/// proves the SEMANTICS hold across the input space and across tiers.
#[test]
fn every_tier_agrees_on_fuzzed_converge_and_merge() {
    let seeds: u64 = std::env::var("LOGOS_CONVERGE_FUZZ_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let gcc = have_gcc();
    let g_plus = std::process::Command::new("g++")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let mut ran_c = 0usize;
    for seed in 0..seeds {
        for is_join in [true, false] {
            for reps in [1usize, 2] {
                let (src, want) = gen_program(seed, is_join, reps);
                let label = format!(
                    "fuzz_s{seed}_{}_{reps}",
                    if is_join { "join" } else { "merge" }
                );

                // Tree-walker and VM: the pure oracles — ALWAYS agree with the computed expectation.
                let tw = tw_outcome(&src);
                let vm = vm_outcome(&src);
                assert!(
                    tw.error.is_none(),
                    "`{label}` tree-walker errored: {:?}\n{src}",
                    tw.error
                );
                assert!(
                    vm.error.is_none(),
                    "`{label}` VM errored: {:?}\n{src}",
                    vm.error
                );
                let tw_lines: Vec<String> = tw.output.lines().map(|s| s.to_string()).collect();
                let vm_lines: Vec<String> = vm.output.lines().map(|s| s.to_string()).collect();
                assert_eq!(
                    tw_lines, want,
                    "`{label}`: tree-walker must equal the per-field expectation\n{src}"
                );
                assert_eq!(
                    vm_lines, want,
                    "`{label}`: VM must equal the per-field expectation\n{src}"
                );

                // AST-C backend (gcc) — must match the VM byte-for-byte.
                if gcc {
                    if let Some(out) = ast_c_output(&src, &label) {
                        assert_eq!(
                            out.lines().map(|s| s.to_string()).collect::<Vec<_>>(),
                            want,
                            "`{label}`: AST-C must equal the VM\n{src}"
                        );
                        ran_c += 1;
                    } else {
                        panic!("`{label}`: AST-C failed to compile/run a fuzzed Converge/Merge program\n{src}");
                    }
                }
                // Bytecode→C Op-walker (g++) — must match the VM byte-for-byte.
                if g_plus {
                    if let Some(out) = bytecode_c_output(&src, &label) {
                        assert_eq!(
                            out.lines().map(|s| s.to_string()).collect::<Vec<_>>(),
                            want,
                            "`{label}`: bytecode→C walker must equal the VM\n{src}"
                        );
                    } else {
                        panic!("`{label}`: bytecode→C walker failed to compile/run a fuzzed program\n{src}");
                    }
                }
            }
        }
    }
    eprintln!("[converge-fuzz] {seeds} seeds × (join|merge) × (1|2 reps) — tree-walker ≡ VM on all; {ran_c} C tier runs agreed");
}
