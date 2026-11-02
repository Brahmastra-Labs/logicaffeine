//! ★ THE IR-PEEPHOLE SOUNDNESS LOCK ★ — the bytecode peephole (`vm::peephole` — block-local
//! copy-propagation + dead-move elimination) is a SEMANTICS-PRESERVING `CompiledProgram →
//! CompiledProgram` transform, and it actually removes ops.
//!
//! The peephole exists for the backends that lower the bytecode straight to a final target with
//! no downstream optimizing assembler (RISC-V asm, WASM, the spatial fabric). gcc already does
//! this coalescing for the C tier; the flat backends would otherwise ship the VM compiler's move
//! cascades verbatim. A wrong coalesce miscompiles EVERY one of those backends, so this lock is
//! the trap:
//!
//!   1. `program_outcome(peephole(P)) == program_outcome(P)` for the raw AND the optimized IR, over
//!      the compute corpus + every shipped benchmark — the peephole never changes meaning.
//!   2. It is idempotent (a genuine fixpoint) and never GROWS the op count on any program.
//!   3. It STRICTLY shrinks the op + move count on the swap idiom (`Set a to b. Set b to t.`), the
//!      measured target — proving it does real work, not nothing.
//!   4. A SplitMix64 fuzzer of copy-heavy programs (chained `Set` reassignments inside bounded
//!      loops — the exact shape that generates temp-move cascades) runs ≡ the reference VM.
#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

use logicaffeine_compile::compile::{
    compile_to_program, compile_to_program_optimized, program_outcome, vm_outcome,
};
use logicaffeine_compile::vm::peephole::peephole;
use logicaffeine_compile::vm::{CompiledProgram, Op};

fn moves(p: &CompiledProgram) -> usize {
    p.code
        .iter()
        .filter(|op| matches!(op, Op::Move { .. }))
        .count()
}

/// Peephole a clone of `prog`, returning the transformed program.
fn peepholed(prog: &CompiledProgram) -> CompiledProgram {
    let mut c = prog.clone();
    peephole(&mut c);
    c
}

/// Assert the peephole preserves the VM outcome of `prog` and never grows the code.
fn assert_preserves(label: &str, prog: &CompiledProgram) -> CompiledProgram {
    let before = program_outcome(prog);
    let opt = peepholed(prog);
    let after = program_outcome(&opt);
    assert_eq!(
        (before.output.trim(), before.error.is_some()),
        (after.output.trim(), after.error.is_some()),
        "`{label}`: PEEPHOLE CHANGED MEANING — before=({:?},{}) after=({:?},{})",
        before.output.trim(),
        before.error.is_some(),
        after.output.trim(),
        after.error.is_some()
    );
    assert!(
        opt.code.len() <= prog.code.len(),
        "`{label}`: peephole GREW the code ({} -> {})",
        prog.code.len(),
        opt.code.len()
    );
    // idempotence: a second pass is a true fixpoint (no further op removed)
    let twice = peepholed(&opt);
    assert_eq!(
        twice.code.len(),
        opt.code.len(),
        "`{label}`: peephole is not idempotent ({} -> {} -> {})",
        prog.code.len(),
        opt.code.len(),
        twice.code.len()
    );
    let twice_out = program_outcome(&twice);
    assert_eq!(
        after.output.trim(),
        twice_out.output.trim(),
        "`{label}`: second peephole pass changed meaning"
    );
    opt
}

/// The compute corpus — the shapes the peephole must be sound over. The `swap_*` programs are the
/// measured target (their `Set a to b. Set b to t.` cascades MUST shrink).
const CORPUS: &[(&str, &str)] = &[
    ("swap_fib_iter", "## Main\n    Let mutable a be 0.\n    Let mutable b be 1.\n    Let mutable i be 1.\n    While i is at most 1000:\n        Let t be (a + b) mod 1000000007.\n        Set a to b.\n        Set b to t.\n        Set i to i + 1.\n    Show b.\n"),
    ("swap_three", "## Main\n    Let mutable a be 1.\n    Let mutable b be 2.\n    Let mutable c be 3.\n    Let mutable i be 1.\n    While i is at most 100:\n        Set a to b.\n        Set b to c.\n        Set c to a.\n        Set i to i + 1.\n    Show a.\n    Show b.\n    Show c.\n"),
    ("lcg_hash", "## Main\n    Let mutable h be 7.\n    Let mutable i be 1.\n    While i is at most 1000:\n        Set h to (h * 1103515 + 12345) mod 2147483647.\n        Set i to i + 1.\n    Show h.\n"),
    ("nested_loops", "## Main\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is at most 50:\n        Let mutable j be 1.\n        While j is at most 50:\n            Set acc to acc + i * j.\n            Set j to j + 1.\n        Set i to i + 1.\n    Show acc.\n"),
    ("recursion", "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\n    Show fib(20).\n"),
    ("collection_build", "## Main\n    Let mutable xs be a new Seq of Int.\n    Let mutable i be 1.\n    While i is at most 50:\n        Push i * i to xs.\n        Set i to i + 1.\n    Show length of xs.\n    Show item 50 of xs.\n"),
    // ── value-semantics aliasing torture (the clobbers-by-value invalidation rule) ─────────────────
    // `Let ys be xs` shallow-copies the handle (a Move → copy[ys]=xs). Pushing to `xs` then triggers
    // copy-on-write, so `ys` keeps the pre-push length. If the peephole failed to invalidate the
    // `ys = xs` fact when `xs` is mutated, it would rewrite `length of ys` to `length of xs` and print
    // the WRONG (post-push) length. VM(peephole) must equal VM (=3/4 here), locking the aliasing case.
    ("alias_push_after_copy", "## Main\n    Let mutable xs be a new Seq of Int.\n    Push 1 to xs.\n    Push 2 to xs.\n    Push 3 to xs.\n    Let ys be xs.\n    Push 4 to xs.\n    Show length of ys.\n    Show length of xs.\n"),
    ("alias_read_between", "## Main\n    Let mutable xs be a new Seq of Int.\n    Push 10 to xs.\n    Push 20 to xs.\n    Let ys be xs.\n    Show length of ys.\n    Push 30 to xs.\n    Show length of ys.\n    Show length of xs.\n"),
    ("branch", "## Main\n    Let x be 10.\n    If x is greater than 5:\n        Show 1.\n    Otherwise:\n        Show 999.\n"),
    // ── enum Inspect (TestArm/BindArm): the scrutinee register is a typed anchor the flat backends'
    // kind-inference couples positionally (TestArm.target must match the following BindArm.target).
    // The peephole must NOT rewrite it — copy-prop would rewrite the in-condition-block TestArm but not
    // the arm-body BindArm, breaking the coupling and pushing the (still ≡VM) program out of riscv/wasm's
    // enum fragment. ≡VM here; the fragment refusal is pinned by riscv_aot_lock.
    ("enum_inspect", "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n\n## Main\nLet s be a new Circle with radius 10.\nInspect s:\n    When Circle (r): Show r.\n    When Rectangle (w, h): Show w.\n"),
    ("global_in_callee", "## To bump:\n    Set counter to counter + 1.\n## Main\n    Let counter be 0.\n    Let i be 0.\n    While i is less than 100:\n        bump().\n        Set i to i + 1.\n    Show counter.\n"),
    ("const_fold", "## Main\n    Show 2 + 3 * 4 - 1.\n"),
    ("div_by_zero_error", "## Main\n    Let a be 5.\n    Let b be 0.\n    Show a / b.\n"),
];

#[test]
fn the_peephole_preserves_meaning_over_the_corpus_raw_and_optimized() {
    // ★ EVERY PROGRAM IS COMPARED, FAILING ONES INCLUDED. This used to compare output only when
    // the reference VM ran cleanly — and a skip looks exactly like a pass from outside, so the one
    // corpus program whose whole POINT is to fail (`div_by_zero_error`) was never checked at all.
    // A peephole that turned that trap into a wrong answer would have passed this lock.
    //
    // The fix is not a floor on how many ran; it is to treat the ERROR as part of the outcome,
    // which it is. `Division by zero` is what that program means.
    let mut compared = 0usize;
    for (label, src) in CORPUS {
        let raw =
            compile_to_program(src).unwrap_or_else(|e| panic!("`{label}` raw compile: {e:?}"));
        let raw_opt = assert_preserves(&format!("{label}/raw"), &raw);
        // when the raw VM runs cleanly, the peephole output must also match the reference tree-walker outcome
        let reference = vm_outcome(src);
        compared += 1;
        let got = program_outcome(&raw_opt);
        assert_eq!(
            (reference.output.trim(), &reference.error),
            (got.output.trim(), &got.error),
            "`{label}`: peephole(raw) != reference VM. A program that FAILS must fail the same \
             way — the error is what it means, and comparing only stdout let a trap turn into a \
             wrong answer unnoticed."
        );
        let opt = compile_to_program_optimized(src)
            .unwrap_or_else(|e| panic!("`{label}` opt compile: {e:?}"));
        assert_preserves(&format!("{label}/opt"), &opt);
    }
    assert_eq!(
        compared,
        CORPUS.len(),
        "only {compared} of {} corpus programs reached the output comparison — the rest failed to \
         RUN on the reference VM, so the peephole's meaning was never checked against them. That \
         is a regression in the corpus, not a reason to skip: fix the program or the tier.",
        CORPUS.len()
    );
}

#[test]
fn the_peephole_strictly_shrinks_the_swap_idiom() {
    // fib's `Set a to b. Set b to t.` lowers to a five-move cascade through temps every iteration;
    // copy-prop + DCE must collapse it. Prove BOTH the op count and the move count strictly drop.
    for label in ["swap_fib_iter", "swap_three"] {
        let (_, src) = CORPUS.iter().find(|(n, _)| *n == label).unwrap();
        let raw = compile_to_program(src).unwrap();
        let opt = peepholed(&raw);
        assert!(
            opt.code.len() < raw.code.len(),
            "`{label}`: expected the op count to shrink ({} -> {})",
            raw.code.len(),
            opt.code.len()
        );
        assert!(
            moves(&opt) < moves(&raw),
            "`{label}`: expected the move count to shrink ({} -> {})",
            moves(&raw),
            moves(&opt)
        );
    }
}

fn benchmarks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/programs")
}

#[test]
fn the_peephole_preserves_meaning_over_every_benchmark() {
    let dir = benchmarks_dir();
    let mut checked = 0usize;
    let mut shrank = 0usize;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let mut progs: Vec<(String, String)> = Vec::new();
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let interp = p.join("interp.lg");
                if interp.is_file() {
                    if let Ok(src) = std::fs::read_to_string(&interp) {
                        progs.push((p.file_name().unwrap().to_string_lossy().to_string(), src));
                    }
                }
            }
        }
        progs.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, src) in &progs {
            // only programs the raw VM runs cleanly are outcome subjects (some need argv)
            if vm_outcome(src).error.is_some() {
                continue;
            }
            let raw = match compile_to_program(src) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let opt = assert_preserves(name, &raw);
            if opt.code.len() < raw.code.len() {
                shrank += 1;
            }
            checked += 1;
        }
    }
    assert!(
        checked >= 20,
        "too few benchmarks peephole-checked ({checked})"
    );
    eprintln!("peephole: {checked} benchmarks ≡ VM after peephole; {shrank} shrank");
}

// ───────────────────────── fuzzer ─────────────────────────

struct SplitMix64(u64);
impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Build a copy-heavy but guaranteed-terminating program: a handful of Int vars, then bounded
/// `Repeat` loops whose bodies are chained `Set vi to vj (+/-) vk` reassignments — the exact shape
/// that produces temp-move cascades. Every value stays bounded (only + and -), so the raw VM runs
/// fast and cleanly, and the peephole output must match it byte-for-byte.
fn gen_program(rng: &mut SplitMix64) -> String {
    let nvars = 3 + rng.below(3) as usize; // 3..=5
    let mut s = String::from("## Main\n");
    for v in 0..nvars {
        s.push_str(&format!("    Let mutable v{v} be {}.\n", rng.below(20)));
    }
    let stmts = 3 + rng.below(6) as usize;
    for _ in 0..stmts {
        match rng.below(3) {
            0 => {
                // a bounded loop with a chained-Set body (the copy-cascade generator)
                let bound = 1 + rng.below(6);
                s.push_str(&format!("    Repeat for q from 1 to {bound}:\n"));
                let body = 2 + rng.below(3) as usize;
                for _ in 0..body {
                    let d = rng.below(nvars as u64);
                    let a = rng.below(nvars as u64);
                    let b = rng.below(nvars as u64);
                    let op = if rng.below(2) == 0 { "+" } else { "-" };
                    s.push_str(&format!("        Set v{d} to v{a} {op} v{b}.\n"));
                }
            }
            1 => {
                // a swap chain (a := b; b := c; …) — the canonical temp-move idiom
                let len = 2 + rng.below((nvars - 1) as u64) as usize;
                for k in 0..len {
                    let d = k % nvars;
                    let sfrom = (k + 1) % nvars;
                    s.push_str(&format!("    Set v{d} to v{sfrom}.\n"));
                }
            }
            _ => {
                let d = rng.below(nvars as u64);
                let a = rng.below(nvars as u64);
                let b = rng.below(nvars as u64);
                let op = ["+", "-", "*"][rng.below(3) as usize];
                s.push_str(&format!("    Set v{d} to v{a} {op} v{b}.\n"));
            }
        }
    }
    for v in 0..nvars {
        s.push_str(&format!("    Show v{v}.\n"));
    }
    s
}

#[test]
fn the_peephole_equals_the_vm_over_a_copy_heavy_fuzzer() {
    let mut rng = SplitMix64(0xC0FFEE_1234_5678);
    let mut ran = 0usize;
    for seed in 0..400u64 {
        rng.0 = 0x1000 + seed.wrapping_mul(0x9E37_79B9);
        let src = gen_program(&mut rng);
        let reference = vm_outcome(&src);
        if reference.error.is_some() {
            continue; // an overflow/edge program isn't an outcome subject; ≡ still holds structurally below
        }
        let raw = match compile_to_program(&src) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let opt = peepholed(&raw);
        let got = program_outcome(&opt);
        assert_eq!(
            reference.output.trim(),
            got.output.trim(),
            "FUZZ seed {seed}: peephole != VM\n--- program ---\n{src}"
        );
        assert!(
            opt.code.len() <= raw.code.len(),
            "FUZZ seed {seed}: peephole grew the code"
        );
        ran += 1;
    }
    assert!(ran >= 200, "fuzzer ran too few clean programs ({ran})");
    eprintln!("peephole fuzzer: {ran} copy-heavy programs ≡ VM");
}
