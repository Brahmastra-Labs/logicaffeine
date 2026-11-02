//! ★ THE IR-PEEPHOLE AUDIT LOCK ★ — the deep QA companion to `peephole_soundness_lock`. Where that
//! lock proves the peephole is `== VM`, this one proves the pass's MECHANICS are correct and that it
//! actually does work:
//!
//!   1. STRUCTURAL VALIDITY — after the peephole, every jump/branch/iterator target, every function
//!      entry, and every `loop_locals` key is in bounds, `register_count` is preserved, and the code
//!      never grows. This is what pins the `remove_ops` renumbering (a dangling jump target is a
//!      silent miscompile the `== VM` corpus check could miss if it never hit that exact deletion).
//!   2. THE ENUM-SCRUTINEE INVARIANT — the peephole NEVER alters a `TestArm`/`BindArm` op (their
//!      target is a typed anchor the flat backends couple positionally). Proven directly on the op
//!      stream, not just via a backend refusal.
//!   3. IT MEASURABLY OPTIMIZES — the swap idiom loses a floor of moves. Catches a future change that
//!      silently turns the pass into a no-op (which `== VM` would happily accept).
//!   4. A STRUCTURAL FUZZER — programs with FUNCTIONS (multi-region frames + `Call` arg windows),
//!      COLLECTIONS (value-semantics mutation), BRANCHES and bounded loops, run `== VM` AND stay
//!      structurally valid. The soundness lock's fuzzer is scalar-only; this exercises the CFG,
//!      cross-frame isolation, and the range-operand liveness the mechanics depend on.
#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

use logicaffeine_compile::compile::{compile_to_program, program_outcome, vm_outcome};
use logicaffeine_compile::vm::peephole::peephole;
use logicaffeine_compile::vm::{CompiledProgram, Op};

fn peepholed(prog: &CompiledProgram) -> CompiledProgram {
    let mut c = prog.clone();
    peephole(&mut c);
    c
}

fn moves(p: &CompiledProgram) -> usize {
    p.code
        .iter()
        .filter(|op| matches!(op, Op::Move { .. }))
        .count()
}

/// Every position an op refers to must land on a real instruction; the frame/loop metadata must too.
fn assert_structurally_valid(label: &str, p: &CompiledProgram) {
    let n = p.code.len();
    for (pc, op) in p.code.iter().enumerate() {
        match op {
            Op::Jump { target }
            | Op::JumpIfFalse { target, .. }
            | Op::JumpIfTrue { target, .. } => {
                assert!(
                    *target < n,
                    "`{label}`: pc {pc} {op:?} — target {target} out of bounds (n={n})"
                );
            }
            Op::IterNext { exit, .. } => {
                assert!(
                    *exit < n,
                    "`{label}`: pc {pc} IterNext exit {exit} out of bounds (n={n})"
                );
            }
            _ => {}
        }
    }
    for (i, f) in p.functions.iter().enumerate() {
        assert!(
            f.entry_pc < n,
            "`{label}`: function {i} entry_pc {} out of bounds (n={n})",
            f.entry_pc
        );
    }
    for &k in p.loop_locals.keys() {
        assert!(
            k < n,
            "`{label}`: loop_locals key {k} out of bounds (n={n})"
        );
    }
}

/// The `(target, variant, field, index)` tuple of every enum-arm op, in program order — the sequence
/// the peephole must leave byte-identical.
fn enum_arm_fingerprint(p: &CompiledProgram) -> Vec<(u16, u32, u32, u16)> {
    p.code
        .iter()
        .filter_map(|op| match op {
            Op::TestArm {
                target, variant, ..
            } => Some((*target, *variant, u32::MAX, u16::MAX)),
            Op::BindArm {
                target,
                field,
                index,
                ..
            } => Some((*target, u32::MAX, *field, *index)),
            _ => None,
        })
        .collect()
}

const STRUCT_CORPUS: &[(&str, &str)] = &[
    ("enum_inspect", "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n\n## Main\nLet s be a new Circle with radius 10.\nInspect s:\n    When Circle (r): Show r.\n    When Rectangle (w, h): Show w.\n"),
    ("enum_in_loop", "## A Box is one of:\n    A Full with n Int.\n    An Empty.\n\n## Main\n    Let mutable total be 0.\n    Let mutable i be 1.\n    While i is at most 20:\n        Let b be a new Full with n i.\n        Inspect b:\n            When Full (n): Set total to total + n.\n            When Empty: Set total to total + 0.\n        Set i to i + 1.\n    Show total.\n"),
];

#[test]
fn the_peephole_never_alters_an_enum_arm_op() {
    let mut checked_with_arms = 0usize;
    for (label, src) in STRUCT_CORPUS {
        // Skip an entry the parser rejects (belt-and-suspenders for a hand-written enum shape); the
        // assert below still requires at least the canonical `enum_inspect` program to carry the check.
        let raw = match compile_to_program(src) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("`{label}`: skipped (does not compile: {e:?})");
                continue;
            }
        };
        let before = enum_arm_fingerprint(&raw);
        let opt = peepholed(&raw);
        let after = enum_arm_fingerprint(&opt);
        assert_eq!(
            before, after,
            "`{label}`: the peephole ALTERED an enum-arm op — scrutinee anchors must be inviolate"
        );
        assert_structurally_valid(label, &opt);
        let (b, a) = (program_outcome(&raw), program_outcome(&opt));
        assert_eq!(
            b.output.trim(),
            a.output.trim(),
            "`{label}`: peephole changed meaning"
        );
        if !before.is_empty() {
            checked_with_arms += 1;
        }
    }
    assert!(
        checked_with_arms >= 1,
        "no enum-arm program was actually checked"
    );
}

#[test]
fn the_peephole_measurably_removes_moves_from_the_swap_idiom() {
    // The five-move swap cascade (`Set a to b. Set b to t.`) must lose a real floor of moves, or the
    // pass has silently become a no-op (which the `== VM` checks would not catch).
    let src = "## Main\n    Let mutable a be 0.\n    Let mutable b be 1.\n    Let mutable i be 1.\n    While i is at most 1000:\n        Let t be (a + b) mod 1000000007.\n        Set a to b.\n        Set b to t.\n        Set i to i + 1.\n    Show b.\n";
    let raw = compile_to_program(src).unwrap();
    let opt = peepholed(&raw);
    let removed_moves = moves(&raw) - moves(&opt);
    let removed_ops = raw.code.len() - opt.code.len();
    assert!(
        removed_moves >= 3,
        "swap idiom should shed >= 3 moves (five-move cascade -> two), shed {removed_moves} ({} -> {})",
        moves(&raw), moves(&opt)
    );
    assert!(
        removed_ops >= 3,
        "op count should drop by >= 3, dropped {removed_ops}"
    );
    assert_structurally_valid("swap_idiom", &opt);
}

fn benchmarks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/programs")
}

#[test]
fn the_peephole_output_is_structurally_valid_over_every_benchmark() {
    let dir = benchmarks_dir();
    let mut checked = 0usize;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let mut progs: Vec<(String, String)> = Vec::new();
        for e in entries.flatten() {
            let interp = e.path().join("interp.lg");
            if interp.is_file() {
                if let Ok(src) = std::fs::read_to_string(&interp) {
                    progs.push((
                        e.path().file_name().unwrap().to_string_lossy().to_string(),
                        src,
                    ));
                }
            }
        }
        for (name, src) in &progs {
            let raw = match compile_to_program(src) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let opt = peepholed(&raw);
            assert_structurally_valid(name, &opt);
            assert_eq!(
                opt.register_count, raw.register_count,
                "`{name}`: register_count changed"
            );
            assert!(opt.code.len() <= raw.code.len(), "`{name}`: code grew");
            // the enum-arm invariant holds on real programs too
            assert_eq!(
                enum_arm_fingerprint(&raw),
                enum_arm_fingerprint(&opt),
                "`{name}`: enum arm altered"
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 20,
        "expected the benchmark suite, checked {checked}"
    );
    eprintln!("peephole audit: {checked} benchmarks structurally valid after peephole");
}

// ───────────────────────── structural fuzzer ─────────────────────────

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

/// A diverse, guaranteed-terminating program: an optional helper FUNCTION (multi-region frames +
/// `Call` arg windows), an optional COLLECTION (value-semantics mutation the copy-invalidation must
/// respect), plus BRANCHES, bounded loops, swap chains and calls — the CFG + cross-frame shapes the
/// peephole mechanics depend on. Only `+`/`-` keeps values bounded so the VM stays fast and clean.
fn gen_structural(rng: &mut SplitMix64) -> String {
    let mut s = String::new();
    let has_fn = rng.below(2) == 0;
    if has_fn {
        s.push_str("## To helper (a: Int) (b: Int) -> Int:\n    Let mutable r be a.\n");
        for _ in 0..(1 + rng.below(3)) {
            let op = if rng.below(2) == 0 { "+" } else { "-" };
            let x = if rng.below(2) == 0 { "a" } else { "b" };
            s.push_str(&format!("    Set r to r {op} {x}.\n"));
        }
        s.push_str("    Return r.\n");
    }
    s.push_str("## Main\n");
    let nvars = 3 + rng.below(3) as usize;
    for v in 0..nvars {
        s.push_str(&format!("    Let mutable v{v} be {}.\n", rng.below(20)));
    }
    let has_seq = rng.below(2) == 0;
    if has_seq {
        s.push_str("    Let mutable xs be a new Seq of Int.\n");
    }
    for _ in 0..(3 + rng.below(6)) {
        match rng.below(6) {
            0 => {
                let (d, a, b) = (
                    rng.below(nvars as u64),
                    rng.below(nvars as u64),
                    rng.below(nvars as u64),
                );
                let op = if rng.below(2) == 0 { "+" } else { "-" };
                s.push_str(&format!("    Set v{d} to v{a} {op} v{b}.\n"));
            }
            1 => {
                for k in 0..(2 + rng.below(2) as usize) {
                    s.push_str(&format!(
                        "    Set v{} to v{}.\n",
                        k % nvars,
                        (k + 1) % nvars
                    ));
                }
            }
            2 => {
                let (a, b, d) = (
                    rng.below(nvars as u64),
                    rng.below(nvars as u64),
                    rng.below(nvars as u64),
                );
                s.push_str(&format!(
                    "    If v{a} is greater than v{b}:\n        Set v{d} to v{a} + 1.\n    Otherwise:\n        Set v{d} to v{b} - 1.\n"
                ));
            }
            3 => {
                let bound = 1 + rng.below(5);
                s.push_str(&format!("    Repeat for q from 1 to {bound}:\n"));
                for _ in 0..(1 + rng.below(2)) {
                    let (d, a) = (rng.below(nvars as u64), rng.below(nvars as u64));
                    s.push_str(&format!("        Set v{d} to v{d} + v{a}.\n"));
                }
            }
            4 if has_seq => {
                let a = rng.below(nvars as u64);
                s.push_str(&format!("    Push v{a} to xs.\n"));
                let d = rng.below(nvars as u64);
                s.push_str(&format!("    Set v{d} to length of xs.\n"));
            }
            5 if has_fn => {
                let (d, a, b) = (
                    rng.below(nvars as u64),
                    rng.below(nvars as u64),
                    rng.below(nvars as u64),
                );
                s.push_str(&format!("    Set v{d} to helper(v{a}, v{b}).\n"));
            }
            _ => {
                let (d, a) = (rng.below(nvars as u64), rng.below(nvars as u64));
                s.push_str(&format!("    Set v{d} to v{a}.\n"));
            }
        }
    }
    for v in 0..nvars {
        s.push_str(&format!("    Show v{v}.\n"));
    }
    if has_seq {
        s.push_str("    Show length of xs.\n");
    }
    s
}

/// Curated diverse programs, each exercising an op path the scalar/structural fuzzers miss: struct
/// value-semantics field mutation under aliasing (`StructInsert`/`ListPushField`), map insert/read
/// aliasing (`SetIndex`/`Index`), tuple construction (`NewTuple`), a function with MULTIPLE early
/// returns (a per-function CFG with several `Return` points), and amortized text build (`AddAssign`
/// on Text). Each is checked ≡VM + structurally valid + idempotent.
const COMPOSITE_CORPUS: &[(&str, &str)] = &[
    // struct with a Seq field: aliasing `y = b` then mutating `b`'s field must not leak into `y`
    ("struct_seq_field_alias", "## A Bag has:\n    An items: Seq of Int.\n## Main\n    Let mutable b be a new Bag with items [1, 2].\n    Let y be b.\n    Push 5 to b's items.\n    Show length of b's items.\n    Show length of y's items.\n"),
    // struct scalar fields + GetField through an alias
    ("struct_scalar_fields", "## A Vec has:\n    x: Int.\n    y: Int.\n## Main\n    Let a be a new Vec with x 3 and y 10.\n    Let b be a.\n    Show a's x.\n    Show b's y.\n    Show a's x + b's y.\n"),
    // map insert/read with an alias between two writes
    ("map_insert_alias", "## Main\n    Let mutable m be a new Map of Int to Int.\n    Set item 1 of m to 5.\n    Let y be m.\n    Set item 2 of m to 9.\n    Show item 1 of y.\n    Show item 1 of m.\n"),
    ("tuple_build", "## Main\n    Let a be 7.\n    Let b be 8.\n    Let t be (a, b).\n    Show t.\n"),
    // DestructureTuple — the ONLY range-WRITE op, a distinct liveness path
    ("tuple_destructure", "## Main\n    Let a be 7.\n    Let b be 8.\n    Let (p, q) be (a, b).\n    Show p.\n    Show q.\n    Show p + q.\n"),
    // destructure then swap the sources through the bound halves — moves around a range write
    ("tuple_destructure_swap", "## Main\n    Let mutable a be 1.\n    Let mutable b be 2.\n    Let (p, q) be (a, b).\n    Set a to q.\n    Set b to p.\n    Show a.\n    Show b.\n"),
    // a function with THREE Return points — the per-function liveness must isolate the frame
    ("multi_early_return", "## To classify (n: Int) -> Int:\n    If n is less than 0:\n        Return 0.\n    If n is greater than 100:\n        Return 100.\n    Return n.\n## Main\n    Let mutable total be 0.\n    Let mutable i be 1.\n    While i is at most 30:\n        Set total to total + classify(i * 7).\n        Set i to i + 1.\n    Show total.\n"),
    // amortized Text build — AddAssign on a string
    ("text_build", "## Main\n    Let mutable s be \"\".\n    Let mutable i be 1.\n    While i is at most 6:\n        Set s to s + \"x\".\n        Set i to i + 1.\n    Show s.\n"),
    // deep nesting: if inside while inside a function called in a loop
    ("deep_nesting", "## To step (a: Int) (b: Int) -> Int:\n    Let mutable r be a.\n    If a is greater than b:\n        Set r to a - b.\n    Otherwise:\n        Set r to b - a.\n    Return r.\n## Main\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is at most 20:\n        Let mutable j be 1.\n        While j is at most 5:\n            Set acc to acc + step(i, j).\n            Set j to j + 1.\n        Set i to i + 1.\n    Show acc.\n"),
    // Sets — the last collection type: add/remove/contains + mutation aliasing (SetAdd mutates)
    ("set_ops", "## Main\n    Let mutable s be a new Set of Int.\n    Add 1 to s.\n    Add 2 to s.\n    Add 2 to s.\n    Remove 1 from s.\n    If s contains 2:\n        Show 1.\n    Otherwise:\n        Show 0.\n    Show length of s.\n"),
    ("set_alias", "## Main\n    Let mutable s be a new Set of Int.\n    Add 1 to s.\n    Let y be s.\n    Add 2 to s.\n    Show length of s.\n    Show length of y.\n"),
];

#[test]
fn the_peephole_handles_composites_and_early_returns() {
    let mut checked = 0usize;
    for (label, src) in COMPOSITE_CORPUS {
        let raw = match compile_to_program(src) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("`{label}`: skipped (does not compile: {e:?})");
                continue;
            }
        };
        // only a program the VM runs cleanly is an outcome subject
        let reference = vm_outcome(src);
        if reference.error.is_some() {
            eprintln!("`{label}`: skipped (VM errors: {:?})", reference.error);
            continue;
        }
        let opt = peepholed(&raw);
        assert_structurally_valid(label, &opt);
        assert_eq!(
            opt.register_count, raw.register_count,
            "`{label}`: register_count changed"
        );
        assert!(opt.code.len() <= raw.code.len(), "`{label}`: code grew");
        assert_eq!(
            enum_arm_fingerprint(&raw),
            enum_arm_fingerprint(&opt),
            "`{label}`: enum arm altered"
        );
        let got = program_outcome(&opt);
        assert_eq!(
            reference.output.trim(),
            got.output.trim(),
            "`{label}`: peephole != VM\n{src}"
        );
        // idempotence
        let twice = peepholed(&opt);
        assert_eq!(
            twice.code.len(),
            opt.code.len(),
            "`{label}`: not idempotent"
        );
        checked += 1;
    }
    assert!(
        checked >= 5,
        "too few composite programs checked ({checked}) — a syntax guess regressed"
    );
    eprintln!(
        "peephole composites: {checked}/{} programs ≡ VM",
        COMPOSITE_CORPUS.len()
    );
}

#[test]
fn the_peephole_equals_the_vm_over_a_structural_fuzzer() {
    let mut rng = SplitMix64(0);
    let mut ran = 0usize;
    let mut with_fn = 0usize;
    let mut with_seq = 0usize;
    for seed in 0..500u64 {
        rng.0 = 0xA11CE + seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let src = gen_structural(&mut rng);
        let reference = vm_outcome(&src);
        if reference.error.is_some() {
            continue;
        }
        let raw = match compile_to_program(&src) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let opt = peepholed(&raw);
        assert_structurally_valid(&format!("fuzz#{seed}"), &opt);
        assert_eq!(
            opt.register_count, raw.register_count,
            "fuzz#{seed}: register_count changed\n{src}"
        );
        assert!(
            opt.code.len() <= raw.code.len(),
            "fuzz#{seed}: code grew\n{src}"
        );
        let got = program_outcome(&opt);
        assert_eq!(
            reference.output.trim(),
            got.output.trim(),
            "fuzz#{seed}: peephole != VM\n--- program ---\n{src}"
        );
        if src.contains("## To helper") {
            with_fn += 1;
        }
        if src.contains("new Seq") {
            with_seq += 1;
        }
        ran += 1;
    }
    assert!(
        ran >= 200,
        "structural fuzzer ran too few clean programs ({ran})"
    );
    assert!(
        with_fn >= 40,
        "too few function programs exercised ({with_fn})"
    );
    assert!(
        with_seq >= 40,
        "too few collection programs exercised ({with_seq})"
    );
    eprintln!("peephole structural fuzzer: {ran} programs ≡ VM ({with_fn} with functions, {with_seq} with collections)");
}

/// Heavy COLLECTION ALIASING — the highest-risk soundness area. Several `Seq`s, several mutable
/// aliases, and interleaved `Push` mutations, direct re-aliases (`Set a to xs`) and alias-to-alias
/// copy chains (`Set a to b`). Under value semantics a `Push` to `xs` must NOT leak into an alias
/// snapshotted before it, so the peephole's copy invalidation (`clobbers = kills ∪ mutates`, both
/// key and value side) is exercised across a dense web of `Move`s the scalar fuzzers never build.
/// The alias lengths are Shown, so any divergence in the ≡VM check is observable.
fn gen_seq_aliasing(rng: &mut SplitMix64) -> String {
    let nseq = 2 + rng.below(2) as usize;
    let nalias = 2 + rng.below(3) as usize;
    let mut s = String::from("## Main\n");
    for j in 0..nseq {
        s.push_str(&format!("    Let mutable xs{j} be a new Seq of Int.\n"));
        s.push_str(&format!("    Push {} to xs{j}.\n", rng.below(9)));
    }
    s.push_str("    Let mutable v be 0.\n");
    for k in 0..nalias {
        s.push_str(&format!("    Let mutable a{k} be xs0.\n"));
    }
    for _ in 0..(5 + rng.below(9)) {
        match rng.below(4) {
            0 => {
                let j = rng.below(nseq as u64);
                s.push_str(&format!("    Push {} to xs{j}.\n", rng.below(9)));
            }
            1 => {
                let (k, j) = (rng.below(nalias as u64), rng.below(nseq as u64));
                s.push_str(&format!("    Set a{k} to xs{j}.\n"));
            }
            2 => {
                let (k, o) = (rng.below(nalias as u64), rng.below(nalias as u64));
                s.push_str(&format!("    Set a{k} to a{o}.\n"));
            }
            _ => {
                let k = rng.below(nalias as u64);
                s.push_str(&format!("    Set v to v + length of a{k}.\n"));
            }
        }
    }
    for j in 0..nseq {
        s.push_str(&format!("    Show length of xs{j}.\n"));
    }
    for k in 0..nalias {
        s.push_str(&format!("    Show length of a{k}.\n"));
    }
    s.push_str("    Show v.\n");
    s
}

#[test]
fn the_peephole_equals_the_vm_over_a_collection_aliasing_fuzzer() {
    let mut rng = SplitMix64(0);
    let mut ran = 0usize;
    for seed in 0..400u64 {
        rng.0 = 0xA11A5 + seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let src = gen_seq_aliasing(&mut rng);
        let reference = vm_outcome(&src);
        if reference.error.is_some() {
            continue;
        }
        let raw = match compile_to_program(&src) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let opt = peepholed(&raw);
        assert_structurally_valid(&format!("alias#{seed}"), &opt);
        let got = program_outcome(&opt);
        assert_eq!(
            reference.output.trim(), got.output.trim(),
            "alias#{seed}: peephole != VM (value-semantics invalidation bug?)\n--- program ---\n{src}"
        );
        ran += 1;
    }
    assert!(
        ran >= 250,
        "collection-aliasing fuzzer ran too few clean programs ({ran})"
    );
    eprintln!("peephole collection-aliasing fuzzer: {ran} programs ≡ VM");
}

/// TUPLE DESTRUCTURING — `DestructureTuple` is the ONLY op that WRITES a register RANGE, a distinct
/// liveness path (`kills` expands `[start, start+count)`). This fuzzer interleaves destructures with
/// copies and arithmetic on the bound halves so a range-write mis-kill (deleting a Move whose dst the
/// destructure overwrites, or failing to kill a half) would diverge from the VM.
fn gen_tuple_destructure(rng: &mut SplitMix64) -> String {
    let nvars = 3 + rng.below(3) as usize;
    let mut s = String::from("## Main\n");
    for v in 0..nvars {
        s.push_str(&format!("    Let mutable v{v} be {}.\n", rng.below(20)));
    }
    let ndestr = 2 + rng.below(3) as usize;
    for d in 0..ndestr {
        let (i, j) = (rng.below(nvars as u64), rng.below(nvars as u64));
        s.push_str(&format!("    Let (d{d}a, d{d}b) be (v{i}, v{j}).\n"));
        let (t, op) = (
            rng.below(nvars as u64),
            if rng.below(2) == 0 { "+" } else { "-" },
        );
        s.push_str(&format!("    Set v{t} to d{d}a {op} d{d}b.\n"));
        // a copy chain touching the destructured slots
        if rng.below(2) == 0 {
            let t2 = rng.below(nvars as u64);
            s.push_str(&format!("    Set v{t2} to d{d}a.\n"));
        }
    }
    for _ in 0..(2 + rng.below(3)) {
        let (d, a, b) = (
            rng.below(nvars as u64),
            rng.below(nvars as u64),
            rng.below(nvars as u64),
        );
        let op = if rng.below(2) == 0 { "+" } else { "-" };
        s.push_str(&format!("    Set v{d} to v{a} {op} v{b}.\n"));
    }
    for v in 0..nvars {
        s.push_str(&format!("    Show v{v}.\n"));
    }
    for d in 0..ndestr {
        s.push_str(&format!("    Show d{d}a.\n    Show d{d}b.\n"));
    }
    s
}

#[test]
fn the_peephole_equals_the_vm_over_a_tuple_destructure_fuzzer() {
    let mut rng = SplitMix64(0);
    let mut ran = 0usize;
    for seed in 0..400u64 {
        rng.0 = 0x7071E + seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let src = gen_tuple_destructure(&mut rng);
        let reference = vm_outcome(&src);
        if reference.error.is_some() {
            continue;
        }
        let raw = match compile_to_program(&src) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let opt = peepholed(&raw);
        assert_structurally_valid(&format!("tup#{seed}"), &opt);
        let got = program_outcome(&opt);
        assert_eq!(
            reference.output.trim(),
            got.output.trim(),
            "tup#{seed}: peephole != VM (range-write liveness bug?)\n--- program ---\n{src}"
        );
        ran += 1;
    }
    assert!(
        ran >= 200,
        "tuple-destructure fuzzer ran too few clean programs ({ran})"
    );
    eprintln!("peephole tuple-destructure fuzzer: {ran} programs ≡ VM");
}

/// SET aliasing — the last collection type. `Add` mutates the set in place, so an alias snapshotted
/// before it must not see the new element (value semantics), exercising the same copy-invalidation as
/// the Seq fuzzer over a different mutating op (`SetAdd`). Add-only keeps every program trap-free.
fn gen_set_aliasing(rng: &mut SplitMix64) -> String {
    let nset = 2 + rng.below(2) as usize;
    let nalias = 2 + rng.below(3) as usize;
    let mut s = String::from("## Main\n");
    for j in 0..nset {
        s.push_str(&format!("    Let mutable s{j} be a new Set of Int.\n"));
        s.push_str(&format!("    Add {} to s{j}.\n", rng.below(6)));
    }
    s.push_str("    Let mutable v be 0.\n");
    for k in 0..nalias {
        s.push_str(&format!("    Let mutable a{k} be s0.\n"));
    }
    for _ in 0..(5 + rng.below(9)) {
        match rng.below(4) {
            0 => {
                let j = rng.below(nset as u64);
                s.push_str(&format!("    Add {} to s{j}.\n", rng.below(6)));
            }
            1 => {
                let (k, j) = (rng.below(nalias as u64), rng.below(nset as u64));
                s.push_str(&format!("    Set a{k} to s{j}.\n"));
            }
            2 => {
                let (k, o) = (rng.below(nalias as u64), rng.below(nalias as u64));
                s.push_str(&format!("    Set a{k} to a{o}.\n"));
            }
            _ => {
                let k = rng.below(nalias as u64);
                s.push_str(&format!("    Set v to v + length of a{k}.\n"));
            }
        }
    }
    for j in 0..nset {
        s.push_str(&format!("    Show length of s{j}.\n"));
    }
    for k in 0..nalias {
        s.push_str(&format!("    Show length of a{k}.\n"));
    }
    s.push_str("    Show v.\n");
    s
}

#[test]
fn the_peephole_equals_the_vm_over_a_set_aliasing_fuzzer() {
    let mut rng = SplitMix64(0);
    let mut ran = 0usize;
    for seed in 0..350u64 {
        rng.0 = 0x5E7A1 + seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let src = gen_set_aliasing(&mut rng);
        let reference = vm_outcome(&src);
        if reference.error.is_some() {
            continue;
        }
        let raw = match compile_to_program(&src) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let opt = peepholed(&raw);
        assert_structurally_valid(&format!("set#{seed}"), &opt);
        let got = program_outcome(&opt);
        assert_eq!(
            reference.output.trim(),
            got.output.trim(),
            "set#{seed}: peephole != VM\n--- program ---\n{src}"
        );
        ran += 1;
    }
    assert!(
        ran >= 200,
        "set-aliasing fuzzer ran too few clean programs ({ran})"
    );
    eprintln!("peephole set-aliasing fuzzer: {ran} programs ≡ VM");
}
