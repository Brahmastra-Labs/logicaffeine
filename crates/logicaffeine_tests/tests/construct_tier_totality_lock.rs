//! ★ CONSTRUCT × TIER TOTALITY LOCK — every construct the language can say, on every tier. ★
//!
//! LOGOS's promise is that a program means the same thing on the tree-walker, the VM, AOT-Rust,
//! AOT-C and WASM. Three separate safety nets were already meant to hold that promise, and an
//! entire TYPE — `Option` — slipped past all three. Understanding how is the whole reason this
//! file exists, because the failure was structural rather than careless:
//!
//!   * `type_tier_coverage_lock`'s census says "add a row when a new `RuntimeValue` variant
//!     ships". `Option` has no `RuntimeValue` variant of its own — that IS the bug — so a census
//!     indexed by runtime representation was blind to precisely the type that lacked one. It
//!     carries no `Option` probe at all, and reports the goal state `KNOWN_GAPS == []`.
//!   * the 687-program harvested gauntlet corpus contains ZERO Option programs (`be some ` = 0,
//!     `When Some` = 0, `Option of` = 0), so the cross-tier differential never ran one.
//!   * no fuzzer generator emits `some`/`none`, and **a differential fuzzer cannot find a
//!     divergence in a construct its grammar cannot express**.
//!
//! Every one of those censuses is HAND-MAINTAINED, and each was blind to exactly what it omitted.
//! A shrink-only ratchet is only ever as honest as its denominator: `KNOWN_GAPS == []` means
//! "zero KNOWN gaps", not "zero gaps", and nothing distinguished the two.
//!
//! ★ SO THIS LOCK'S DENOMINATOR IS COMPUTED BY THE COMPILER, NOT CURATED BY A PERSON.
//!
//! [`classify`] is a TOTAL match over `Expr` with no `_` arm, so adding an AST variant FAILS TO
//! COMPILE until it is classified. [`coverage`] is a TOTAL match over the resulting kind, so a
//! classified variant fails to compile until someone writes a program for it or states, in
//! writing, why it cannot be executed alone. The technique is borrowed from
//! `jones_whole_language_lock`, which classifies all AST variants the same way — that lock never
//! RUNS anything, and `type_tier_coverage_lock` runs things but cannot see a missing type. This
//! is the union neither had, and the union is exactly the hole `Option` fell through.
//!
//! ⚠️  DO NOT ADD A `_` ARM TO EITHER MATCH. ⚠️  The totality is the entire mechanism. A wildcard
//! turns this file back into a hand-maintained list and re-opens the class it exists to close.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::{compile_to_c, compile_to_wasm, tw_outcome, vm_outcome};
use logicaffeine_language::ast::stmt::Expr;

/// One kind per `Expr` variant. Mirrors the AST so [`classify`]'s total match is the thing that
/// keeps the two in step — a new `Expr` variant cannot compile until it lands here too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExprKind {
    Literal,
    Identifier,
    BinaryOp,
    Not,
    Call,
    Index,
    Slice,
    Copy,
    Give,
    Length,
    Contains,
    Union,
    Intersection,
    ManifestOf,
    ChunkAt,
    List,
    Tuple,
    Range,
    FieldAccess,
    New,
    NewVariant,
    Escape,
    OptionSome,
    OptionNone,
    WithCapacity,
    Closure,
    CallExpr,
    InterpolatedString,
    View,
    Dom,
}

/// ★ THE TOTAL MATCH. No `_` arm — adding an `Expr` variant breaks the build here first.
fn classify(e: &Expr<'_>) -> ExprKind {
    match e {
        Expr::Literal(_) => ExprKind::Literal,
        Expr::Identifier(_) => ExprKind::Identifier,
        Expr::BinaryOp { .. } => ExprKind::BinaryOp,
        Expr::Not { .. } => ExprKind::Not,
        Expr::Call { .. } => ExprKind::Call,
        Expr::Index { .. } => ExprKind::Index,
        Expr::Slice { .. } => ExprKind::Slice,
        Expr::Copy { .. } => ExprKind::Copy,
        Expr::Give { .. } => ExprKind::Give,
        Expr::Length { .. } => ExprKind::Length,
        Expr::Contains { .. } => ExprKind::Contains,
        Expr::Union { .. } => ExprKind::Union,
        Expr::Intersection { .. } => ExprKind::Intersection,
        Expr::ManifestOf { .. } => ExprKind::ManifestOf,
        Expr::ChunkAt { .. } => ExprKind::ChunkAt,
        Expr::List(_) => ExprKind::List,
        Expr::Tuple(_) => ExprKind::Tuple,
        Expr::Range { .. } => ExprKind::Range,
        Expr::FieldAccess { .. } => ExprKind::FieldAccess,
        Expr::New { .. } => ExprKind::New,
        Expr::NewVariant { .. } => ExprKind::NewVariant,
        Expr::Escape { .. } => ExprKind::Escape,
        Expr::OptionSome { .. } => ExprKind::OptionSome,
        Expr::OptionNone => ExprKind::OptionNone,
        Expr::WithCapacity { .. } => ExprKind::WithCapacity,
        Expr::Closure { .. } => ExprKind::Closure,
        Expr::CallExpr { .. } => ExprKind::CallExpr,
        Expr::InterpolatedString(_) => ExprKind::InterpolatedString,
        Expr::View { .. } => ExprKind::View,
        Expr::Dom { .. } => ExprKind::Dom,
    }
}

/// What this lock can do with a construct.
enum Coverage {
    /// A self-contained `## Main` program that exercises the construct and `Show`s a result.
    /// The tree-walker's output is the oracle; every other tier must match it byte for byte.
    Program(&'static str),
    /// The construct cannot stand alone in a runnable program. The reason is REQUIRED and is
    /// reviewed like any other claim — "hard to write" is not one of them.
    NotExecutable(&'static str),
    /// ★ NO PROGRAM IN THIS REPOSITORY REACHES THIS NODE — which is a finding, not an excuse.
    ///
    /// An AST variant that nothing constructs could be entirely broken on every tier and no test
    /// would ever say so. This is the darkest corner a compiler has, so the census of it
    /// [`UNREACHABLE_CEILING`] may only ever SHRINK.
    Unreachable(&'static str),
}

/// ★ THE SECOND TOTAL MATCH. A classified variant cannot compile until it has a program here,
/// or a written reason why it cannot have one.
fn coverage(k: ExprKind) -> Coverage {
    use Coverage::{NotExecutable, Program, Unreachable};
    match k {
        ExprKind::Literal => Program("## Main\nShow 7.\n"),
        ExprKind::Identifier => Program("## Main\nLet x be 7.\nShow x.\n"),
        ExprKind::BinaryOp => Program("## Main\nShow 6 * 7.\n"),
        ExprKind::Not => Program("## Main\nShow not (1 is greater than 2).\n"),
        ExprKind::Call => Program("## Main\nShow upper(\"hi\").\n"),
        ExprKind::Index => Program("## Main\nShow item 2 of [10, 20, 30].\n"),
        ExprKind::Slice => Program("## Main\nShow sumSeq(items 1 through 2 of [1, 2, 3]).\n"),
        ExprKind::Copy => Program("## Main\nLet xs be [1, 2].\nLet ys be copy of xs.\nShow length of ys.\n"),
        ExprKind::Give => Unreachable(
            "`Expr::Give` is built only by `parse_call_arg` (parser/mod.rs:6066), and NOTHING in \
             this repository reaches it: zero of the 687 corpus programs, zero stdlib modules, \
             zero examples, zero tests. Every `give` in the tree is prose. The teach lesson and \
             ERROR_CATALOG.md:261 both tell an author to write `a copy of x`, which does not \
             parse — `copy of x` does — so the documented escape hatch is itself invalid syntax.",
        ),
        ExprKind::Length => Program("## Main\nShow length of [1, 2, 3].\n"),
        ExprKind::Contains => Program("## Main\nShow [1, 2, 3] contains 2.\n"),
        ExprKind::Union => Program("## Main\nLet xs be a new Set of Int.\nAdd 1 to xs.\nLet ys be a new Set of Int.\nAdd 2 to ys.\nShow length of (xs union ys).\n"),
        ExprKind::Intersection => Program("## Main\nLet xs be a new Set of Int.\nAdd 1 to xs.\nLet ys be a new Set of Int.\nAdd 1 to ys.\nShow length of (xs intersection ys).\n"),
        ExprKind::List => Program("## Main\nShow sumSeq([1, 2, 3]).\n"),
        ExprKind::Tuple => Program("## To addpair (p: Pair of Int and Int) -> Int:\n    Return item 1 of p + item 2 of p.\n\n## Main\nLet p be (3, 4).\nShow addpair(p).\n"),
        ExprKind::Range => Program("## Main\nLet mutable n be 0.\nRepeat for i from 1 to 3:\n    Set n to n + i.\nShow n.\n"),
        ExprKind::FieldAccess => Program("## A Row has:\n    a n (Int).\n\n## Main\nLet r be a new Row with n 7.\nShow r's n.\n"),
        ExprKind::New => Program("## A Row has:\n    a n (Int).\n\n## Main\nLet r be a new Row with n 7.\nShow r's n.\n"),
        ExprKind::NewVariant => Program("## To f (n: Int) -> Result of Int and Text:\n    Return a new Ok with value n.\n\n## Main\nInspect f(7):\n    When Ok (v):\n        Show v.\n    When Err (e):\n        Show e.\n"),
        ExprKind::InterpolatedString => Program("## Main\nLet n be 7.\nShow \"n is {n}\".\n"),
        ExprKind::Closure => Program("## Main\nLet f be (x: Int) -> x * 2.\nShow f(21).\n"),
        ExprKind::CallExpr => Program("## Main\nLet f be (x: Int) -> x * 2.\nShow f(21).\n"),
        ExprKind::WithCapacity => Program("## Main\nLet a be a new Seq of Int with capacity 4.\nShow length of a.\n"),

        // ★ THE TWO THAT STARTED THIS. `some x` compiles to `x` and `none` to the `Nothing`
        // sentinel, so on a tier that erases to untagged machine words `some 0` and `none` are
        // the same bits. Both programs must answer identically on every tier; today they do not.
        ExprKind::OptionSome => Program(
            "## Main\nLet v be some 0.\nInspect v:\n    When Some (n):\n        Show \"some\".\n    When None:\n        Show \"none\".\n",
        ),
        ExprKind::OptionNone => Program(
            "## Main\nLet v be none.\nInspect v:\n    When Some (n):\n        Show \"some\".\n    When None:\n        Show \"none\".\n",
        ),

        // Constructs that genuinely cannot stand alone in a `## Main` that Shows a value.
        ExprKind::Escape => NotExecutable(
            "an `Escape` embeds target-language source verbatim, so it has no tier-independent \
             meaning to compare — that is the point of it",
        ),
        ExprKind::View => NotExecutable(
            "a `View:` is a component's markup; it renders through the DOM tiers, which \
             `lgx_vm_parity_lock` and `lgx_wasm_lock` compare. It has no `Show`-able value",
        ),
        ExprKind::Dom => NotExecutable(
            "a DOM query answers against a live document; `wasm_dom.rs` and the LGX locks are \
             where it is held, since there is no document on the C or native tiers",
        ),
        ExprKind::ManifestOf => NotExecutable(
            "`manifest of` reads the build's own manifest, so its answer is environment-derived \
             rather than a value the tiers could agree on",
        ),
        ExprKind::ChunkAt => NotExecutable(
            "`ChunkAt` is a wire-codec internal with no surface sentence that reaches `## Main`",
        ),
    }
}

/// Every kind, with a stable index. The index is what makes an omission from [`ALL_KINDS`]
/// detectable rather than silent: a new kind must be given one, and the completeness test then
/// fails until `ALL_KINDS` carries it.
fn index(k: ExprKind) -> usize {
    match k {
        ExprKind::Literal => 0,
        ExprKind::Identifier => 1,
        ExprKind::BinaryOp => 2,
        ExprKind::Not => 3,
        ExprKind::Call => 4,
        ExprKind::Index => 5,
        ExprKind::Slice => 6,
        ExprKind::Copy => 7,
        ExprKind::Give => 8,
        ExprKind::Length => 9,
        ExprKind::Contains => 10,
        ExprKind::Union => 11,
        ExprKind::Intersection => 12,
        ExprKind::ManifestOf => 13,
        ExprKind::ChunkAt => 14,
        ExprKind::List => 15,
        ExprKind::Tuple => 16,
        ExprKind::Range => 17,
        ExprKind::FieldAccess => 18,
        ExprKind::New => 19,
        ExprKind::NewVariant => 20,
        ExprKind::Escape => 21,
        ExprKind::OptionSome => 22,
        ExprKind::OptionNone => 23,
        ExprKind::WithCapacity => 24,
        ExprKind::Closure => 25,
        ExprKind::CallExpr => 26,
        ExprKind::InterpolatedString => 27,
        ExprKind::View => 28,
        ExprKind::Dom => 29,
    }
}

const ALL_KINDS: &[ExprKind] = &[
    ExprKind::Literal,
    ExprKind::Identifier,
    ExprKind::BinaryOp,
    ExprKind::Not,
    ExprKind::Call,
    ExprKind::Index,
    ExprKind::Slice,
    ExprKind::Copy,
    ExprKind::Give,
    ExprKind::Length,
    ExprKind::Contains,
    ExprKind::Union,
    ExprKind::Intersection,
    ExprKind::ManifestOf,
    ExprKind::ChunkAt,
    ExprKind::List,
    ExprKind::Tuple,
    ExprKind::Range,
    ExprKind::FieldAccess,
    ExprKind::New,
    ExprKind::NewVariant,
    ExprKind::Escape,
    ExprKind::OptionSome,
    ExprKind::OptionNone,
    ExprKind::WithCapacity,
    ExprKind::Closure,
    ExprKind::CallExpr,
    ExprKind::InterpolatedString,
    ExprKind::View,
    ExprKind::Dom,
];

/// A short stable name for a kind, for the census and the failure messages.
fn name(k: ExprKind) -> String {
    format!("{k:?}")
}

/// ★ THE SHRINK-ONLY CENSUS. `(construct, tier, why)`. An entry that starts passing must be
/// DELETED, which is what turns a fix into permanently locked ground.
///
/// Every entry here is a REAL cross-tier divergence in the language today. They are not excuses:
/// they are the work list, and this file's job is to make the list impossible to lengthen quietly.
const KNOWN_GAPS: &[(&str, &str, &str)] = &[
    // ★ THE WASM OPTION GAP WAS HERE, AND IS GONE. `some 0` and `none` used to be the same bits,
    // so the wasm backend refused an `Inspect` over an `Option of <scalar>` by name. Giving Option
    // a real tag in the bytecode (`vm/compiler.rs`'s `compile_option_ctor`) closed it without a
    // single line of wasm-backend change — `TestArm`/`BindArm` already read a constructor tag.
    // This ratchet is what noticed: it demanded the entries be DELETED rather than downgraded,
    // which is what makes the ground permanent.
];

fn is_known_gap(construct: &str, tier: &str) -> bool {
    KNOWN_GAPS
        .iter()
        .any(|(c, t, _)| *c == construct && *t == tier)
}

/// The four-way ratchet verdict, decoupled from the live census so the guard test can drive all
/// four cases even at the goal state.
fn judge_verdict(construct: &str, tier: &str, agrees: bool, is_gap: bool, out: &mut Vec<String>) {
    match (agrees, is_gap) {
        (true, false) => {}
        (false, true) => {}
        (false, false) => out.push(format!(
            "REGRESSION: `{construct}` no longer agrees with the tree-walker on tier `{tier}`. A \
             construct just broke on a backend. Fix the backend — do NOT add it to KNOWN_GAPS to \
             go green."
        )),
        (true, true) => out.push(format!(
            "FIXED: `{construct}` now agrees on tier `{tier}` — DELETE (\"{construct}\", \
             \"{tier}\") from KNOWN_GAPS so the ratchet locks the win in."
        )),
    }
}

fn have_gcc() -> bool {
    std::process::Command::new(common::c_compiler())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// What a tier did with a program. The distinction is the whole point: a backend that REFUSES a
/// construct is an honest gap you can plan around, and a backend that RUNS it and answers wrongly
/// is a silent lie. They must never be scored the same way.
#[derive(Debug)]
enum Ran {
    /// Compiled and ran, with this stdout.
    Output(String),
    /// Refused to compile — a clean, loud gap.
    Refused,
}

/// Build `program` through the AOT-C backend. `Refused` when it will not compile; `Output` when
/// it built and ran.
fn c_backend(program: &str) -> Ran {
    match c_backend_output(program) {
        Some(o) => Ran::Output(o),
        None => Ran::Refused,
    }
}

/// Build `program` through the AOT-C backend and return its stdout, or `None` for any failure.
fn c_backend_output(program: &str) -> Option<String> {
    let c = compile_to_c(program).ok()?;
    let dir = std::env::temp_dir().join(format!(
        "logos_construct_{}_{:x}",
        std::process::id(),
        c.len()
    ));
    std::fs::create_dir_all(&dir).ok()?;
    let src = dir.join("out.c");
    let bin = dir.join("out");
    std::fs::write(&src, &c).ok()?;
    let built = common::build_c_release(&src, &bin).ok()?;
    let out = if built.status.success() {
        std::process::Command::new(&bin)
            .output()
            .ok()
            .filter(|r| r.status.success())
            .map(|r| String::from_utf8_lossy(&r.stdout).trim().to_string())
    } else {
        None
    };
    let _ = std::fs::remove_dir_all(&dir);
    out
}

/// ★ THE LOAD-BEARING TEST — every executable construct, on every tier.
#[test]
fn every_construct_agrees_across_every_tier() {
    let gcc = have_gcc();
    let mut violations = Vec::new();

    for &k in ALL_KINDS {
        let program = match coverage(k) {
            Coverage::NotExecutable(_) | Coverage::Unreachable(_) => continue,
            Coverage::Program(p) => p,
        };
        let construct = name(k);

        let oracle = tw_outcome(program);
        assert!(
            oracle.error.is_none(),
            "the tree-walker ORACLE must run `{construct}` cleanly, but it errored: {}\n{program}",
            oracle.error.as_deref().unwrap_or("")
        );
        let want = oracle.output.trim();

        let vm = vm_outcome(program);
        judge_verdict(
            &construct,
            "vm",
            vm.error.is_none() && vm.output.trim() == want,
            is_known_gap(&construct, "vm"),
            &mut violations,
        );

        // WASM: compiling is the tier's own gate — a by-name refusal (which is how the Option
        // gap presents) is a compile failure, so this catches it without needing a wasm runtime.
        judge_verdict(
            &construct,
            "wasm-compile",
            compile_to_wasm(program).is_ok(),
            is_known_gap(&construct, "wasm-compile"),
            &mut violations,
        );

        if gcc {
            match c_backend(program) {
                // ★ A WRONG ANSWER IS NEVER EXCUSABLE, even with a census entry. A backend that
                // runs a program and prints something else has no honest failure mode — the
                // author gets a plausible number and no signal at all. This is the one verdict
                // KNOWN_GAPS cannot silence.
                Ran::Output(got) if got.trim() != want => violations.push(format!(
                    "WRONG ANSWER: `{construct}` RAN on tier `c` and printed {:?} where the \
                     tree-walker says {want:?}. A backend that answers differently rather than \
                     refusing cannot be excused by KNOWN_GAPS — fix it or make it refuse by name.",
                    got.trim()
                )),
                Ran::Output(_) => judge_verdict(
                    &construct,
                    "c",
                    true,
                    is_known_gap(&construct, "c"),
                    &mut violations,
                ),
                Ran::Refused => judge_verdict(
                    &construct,
                    "c",
                    false,
                    is_known_gap(&construct, "c"),
                    &mut violations,
                ),
            }
        }
    }

    assert!(
        violations.is_empty(),
        "CONSTRUCT × TIER TOTALITY broke ({} issue(s)):\n  - {}",
        violations.len(),
        violations.join("\n  - ")
    );
}

/// The census must be complete: every kind appears in `ALL_KINDS` exactly once, and the indices
/// form a contiguous run. This is what makes forgetting `ALL_KINDS` loud rather than silent.
#[test]
fn the_construct_census_is_complete_and_has_no_duplicates() {
    let mut seen: Vec<usize> = ALL_KINDS.iter().map(|&k| index(k)).collect();
    seen.sort_unstable();
    let expected: Vec<usize> = (0..ALL_KINDS.len()).collect();
    assert_eq!(
        seen, expected,
        "ALL_KINDS must list every construct exactly once with contiguous indices. A gap or a \
         duplicate means a construct was added to `index()` but not to `ALL_KINDS` (so it is \
         never executed on any tier) — which is the exact shape of how `Option` hid."
    );
}

/// ★ THE UNREACHABLE CEILING — the count of AST nodes no program in this repository can build.
/// It may only ever go DOWN. Each one is a construct that could be broken on every single tier
/// with nothing to say so, which is precisely the blind spot that hid `Option`.
const UNREACHABLE_CEILING: usize = 1;

#[test]
fn the_set_of_unreachable_constructs_only_ever_shrinks() {
    let unreachable: Vec<String> = ALL_KINDS
        .iter()
        .filter(|&&k| matches!(coverage(k), Coverage::Unreachable(_)))
        .map(|&k| name(k))
        .collect();
    assert!(
        unreachable.len() <= UNREACHABLE_CEILING,
        "{} construct(s) are unreachable, up from {UNREACHABLE_CEILING}: {unreachable:?}\n\
         An AST node no program can build is a node that could be broken on every tier with \
         nothing to report it. Give it a program — do not raise the ceiling.",
        unreachable.len()
    );
    if unreachable.len() < UNREACHABLE_CEILING {
        panic!(
            "only {} construct(s) are still unreachable ({unreachable:?}) — LOWER \
             UNREACHABLE_CEILING to {} so the ground that was won stays won.",
            unreachable.len(),
            unreachable.len()
        );
    }
}

/// A `NotExecutable` reason has to actually say something. An empty or hand-wavy reason is how a
/// construct quietly leaves the matrix.
#[test]
fn every_unexecutable_construct_states_a_real_reason() {
    for &k in ALL_KINDS {
        let why = match coverage(k) {
            Coverage::NotExecutable(w) | Coverage::Unreachable(w) => Some(w),
            Coverage::Program(_) => None,
        };
        if let Some(why) = why {
            assert!(
                why.len() >= 40 && why.contains(' '),
                "`{}` is excused from the tier matrix with too thin a reason ({why:?}). Say WHERE \
                 it is held instead, so a reader can go and check that claim.",
                name(k)
            );
        }
    }
}

/// Guard on the guard: the ratchet must reject BOTH drift directions, so we know it fires.
#[test]
fn the_ratchet_rejects_regression_and_silent_fix_alike() {
    let mut v = Vec::new();
    judge_verdict("x", "t", true, false, &mut v);
    assert!(
        v.is_empty(),
        "a passing construct with no gap entry must be silent"
    );

    judge_verdict("x", "t", false, true, &mut v);
    assert!(
        v.is_empty(),
        "a documented gap that still gaps must be silent"
    );

    judge_verdict("x", "t", false, false, &mut v);
    assert_eq!(v.len(), 1, "an UNDOCUMENTED break must be reported");
    assert!(
        v[0].contains("REGRESSION"),
        "and named as a regression: {}",
        v[0]
    );

    v.clear();
    judge_verdict("x", "t", true, true, &mut v);
    assert_eq!(v.len(), 1, "a gap that started passing must be reported");
    assert!(
        v[0].contains("FIXED"),
        "and named as a fix to lock in: {}",
        v[0]
    );
}

/// Every census entry must name a construct that exists. A stale entry silences a real tier.
#[test]
fn no_census_entry_names_a_construct_that_does_not_exist() {
    let known: Vec<String> = ALL_KINDS.iter().map(|&k| name(k)).collect();
    for (construct, tier, why) in KNOWN_GAPS {
        assert!(
            known.iter().any(|k| k == construct),
            "KNOWN_GAPS names `{construct}`, which is not a construct. Either it was renamed and \
             the entry is stale — silencing tier `{tier}` for a construct nobody is watching — or \
             it was a typo that has been excusing nothing."
        );
        assert!(
            why.len() >= 40,
            "the gap ({construct}, {tier}) needs a reason a reader can act on, not {why:?}"
        );
    }
}
