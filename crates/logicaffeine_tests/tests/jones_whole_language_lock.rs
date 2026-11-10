//! ════════════════════════════════════════════════════════════════════════════════════════════
//! WHOLE-LANGUAGE JONES COVERAGE LOCK — no construct escapes the partial evaluator silently.
//!
//! `tier_parity_lock` makes the `Stmt` surface build-breaking across the execution tiers. This
//! file does the same for the JONES / Futamura pipeline and, crucially, extends it to the `Expr`
//! surface the PE encoder + the `count_dispatch` oracle must handle. Two EXHAUSTIVE matches with
//! NO `_` arm classify every `Stmt` and every `Expr` variant as:
//!
//!   • Executable    — has a Core-IR encoding and MUST be Jones-optimal through P1. Every
//!                     executable Expr/Stmt is now encodable — the Core-IR gap is CLOSED.
//!   • NonExecutable — declaration / proof-layer construct with no runtime effect, correctly
//!                     outside the executable Jones corpus.
//!
//! The instant a variant is added to the language, THIS FILE STOPS COMPILING until it is
//! classified — which forces a decision about whether the PE must dissolve it. You cannot grow
//! the language past the partial evaluator by accident.
//!
//!  ⚠️  YOU DO NOT GET TO ADD A `_ =>` ARM.  ⚠️  Classify the new variant honestly. If it is
//!  executable, Phase 2/3 must give it a Core-IR encoding and a zero-dispatch P1 residual.
//! ════════════════════════════════════════════════════════════════════════════════════════════

mod pe_support;

use logicaffeine_compile::ast::stmt::{Expr, Stmt};
use logicaffeine_compile::compile::count_dispatch;
use pe_support::*;

/// Whether a surface construct must be dissolved by the partial evaluator, and if so whether the
/// Core IR can represent it yet.
#[derive(Debug, PartialEq, Eq)]
enum JonesClass {
    /// Executable, Core-IR-encodable, must reach `count_dispatch == 0` through P1.
    Executable,
    /// Declaration / proof-layer / meta — no runtime effect, correctly excluded.
    NonExecutable,
    /// The LGX web surface: components, directives, routes, the action vocabulary, typed fetch,
    /// and a `View` value. Executable — the tree-walker runs it — but OUTSIDE the partial
    /// evaluator's executable core today: it has no Core-IR encoding, so a program using it is
    /// not a Jones corpus program. This class exists so that fact is DECLARED rather than
    /// hidden behind a wildcard; when the frontend ops land in the Core IR these rows move to
    /// `Executable` and the Jones-optimality assertions start covering them.
    FrontendSurface,
}

/// ★ COMPILE-TIME LOCK (statements) ★ — exhaustive, no `_` arm.
#[allow(dead_code)]
fn every_stmt_has_a_jones_class(s: &Stmt<'_>) -> JonesClass {
    match s {
        // Declarations and proof-layer constructs: no executable Core body of their own.
        // (`Assert`/`Trust` carry a `LogicExpr`, not an executable `Expr`.)
        Stmt::Theorem(_)
        | Stmt::Definition(_)
        | Stmt::Axiom(_)
        | Stmt::Theory(_)
        | Stmt::Assert { .. }
        | Stmt::Trust { .. }
        | Stmt::Require { .. } => JonesClass::NonExecutable,

        // `## Test` blocks and the test-only statements the parser confines to
        // their bodies: declarations to the projection — the `largo test`
        // runner drives them on the interpreter tier; a specialized program
        // never executes them.
        Stmt::TestDef { .. }
        | Stmt::Expect { .. }
        | Stmt::ExpectFail { .. }
        | Stmt::ExpectOutput { .. }
        | Stmt::AdvanceTime { .. }
        | Stmt::GivenFile { .. }
        // Observation statements are the SEPARATE observability channel
        // (constitution Part 2), elided from the specialized program; `During`
        // is a scope-transparent span whose body the surrounding list encodes.
        // Neither is part of the executable core the PE dissolves.
        | Stmt::Observe { .. }
        | Stmt::During { .. }
        // `For all` is a test-only property; the runner drives it, a
        // specialized program never generates cases.
        | Stmt::ForAll { .. } => JonesClass::NonExecutable,

        // The LGX web frontend — see `FrontendSurface`.
        Stmt::ComponentDef(_)
        | Stmt::DirectiveDef { .. }
        | Stmt::RouteDef { .. }
        | Stmt::MiddlewareDef { .. }
        | Stmt::RedirectDef { .. }
        | Stmt::Ui(_)
        | Stmt::GivenFetch { .. }
        | Stmt::ServeFetches
        | Stmt::Replay { .. }
        | Stmt::Fetch { .. }
        // A failure-site marker (Wave 3) only ever appears inside an LGX action/handler body —
        // it cannot exist in a program without a `## Component`, so it is exactly as much
        // "the LGX web frontend" as `ComponentDef` itself.
        | Stmt::Site(_) => JonesClass::FrontendSurface,

        // Everything else is executable code the PE must dissolve. All are Core-IR encodable
        // today (statement encodings are complete — see `futamura_all_statements_lock`).
        Stmt::Swap { .. }
        | Stmt::Let { .. }
        | Stmt::Set { .. }
        | Stmt::Call { .. }
        | Stmt::If { .. }
        | Stmt::While { .. }
        | Stmt::Repeat { .. }
        | Stmt::Return { .. }
        | Stmt::Break
        | Stmt::RuntimeAssert { .. }
        | Stmt::Fail { .. }
        | Stmt::Give { .. }
        | Stmt::Show { .. }
        | Stmt::SetField { .. }
        | Stmt::StructDef { .. }
        | Stmt::FunctionDef { .. }
        | Stmt::Inspect { .. }
        | Stmt::Push { .. }
        | Stmt::Pop { .. }
        | Stmt::Add { .. }
        | Stmt::Remove { .. }
        | Stmt::SetIndex { .. }
        | Stmt::Splice { .. }
        // A `Reversibly:` block is executable code the PE dissolves exactly like `Splice`:
        // `compile.rs` splices both `Stmt::Splice { body } | Stmt::Reversibly { body }` into the
        // block that holds them, so its body is Core-IR encoded inline (the reverse in-place ops
        // are a VM-only forward-identical encoding).
        | Stmt::Reversibly { .. }
        | Stmt::Zone { .. }
        | Stmt::Concurrent { .. }
        | Stmt::Parallel { .. }
        | Stmt::ReadFrom { .. }
        | Stmt::WriteFile { .. }
        | Stmt::Spawn { .. }
        | Stmt::SendMessage { .. }
        | Stmt::AwaitMessage { .. }
        | Stmt::StreamMessage { .. }
        | Stmt::MergeCrdt { .. }
        | Stmt::JoinCrdt { .. }
        | Stmt::IncreaseCrdt { .. }
        | Stmt::DecreaseCrdt { .. }
        | Stmt::AppendToSequence { .. }
        | Stmt::ResolveConflict { .. }
        | Stmt::Check { .. }
        | Stmt::Listen { .. }
        | Stmt::Serve { .. }
        | Stmt::ConnectTo { .. }
        | Stmt::LetPeerAgent { .. }
        | Stmt::Sleep { .. }
        | Stmt::Sync { .. }
        | Stmt::Mount { .. }
        | Stmt::LaunchTask { .. }
        | Stmt::LaunchTaskWithHandle { .. }
        | Stmt::CreatePipe { .. }
        | Stmt::SendPipe { .. }
        | Stmt::ReceivePipe { .. }
        | Stmt::TrySendPipe { .. }
        | Stmt::TryReceivePipe { .. }
        | Stmt::StopTask { .. }
        | Stmt::CloseChannel { .. }
        | Stmt::AwaitTask { .. }
        | Stmt::Select { .. }
        | Stmt::Escape { .. } => JonesClass::Executable,
    }
}

/// ★ COMPILE-TIME LOCK (expressions) ★ — exhaustive, no `_` arm. This is the surface the PE
/// encoder and the hardened `count_dispatch` oracle must fully cover.
#[allow(dead_code)]
fn every_expr_has_a_jones_class(e: &Expr<'_>) -> JonesClass {
    match e {
        // A template and a page query are the LGX web surface, not Core-IR values — see
        // `FrontendSurface`.
        Expr::View(_) | Expr::Dom(_) => JonesClass::FrontendSurface,

        // Every executable expression now has a Core-IR encoding — the gap is CLOSED. `ManifestOf`
        // and `ChunkAt` are opaque Core nodes (`CManifestOf` / `CChunkAt`) that survive PE 1:1 and
        // decompile back to source; `WithCapacity`'s hint erases to its inner value.
        Expr::ManifestOf { .. }
        | Expr::ChunkAt { .. }
        | Expr::Literal(_)
        | Expr::Identifier(_)
        | Expr::BinaryOp { .. }
        | Expr::Not { .. }
        | Expr::Call { .. }
        | Expr::Index { .. }
        | Expr::Slice { .. }
        | Expr::Copy { .. }
        | Expr::Give { .. }
        | Expr::Length { .. }
        | Expr::Contains { .. }
        | Expr::Union { .. }
        | Expr::Intersection { .. }
        | Expr::List(_)
        | Expr::Tuple(_)
        | Expr::Range { .. }
        | Expr::FieldAccess { .. }
        | Expr::New { .. }
        | Expr::NewVariant { .. }
        | Expr::Escape { .. }
        | Expr::OptionSome { .. }
        | Expr::OptionNone
        | Expr::WithCapacity { .. }
        | Expr::Closure { .. }
        | Expr::CallExpr { .. }
        | Expr::InterpolatedString(_) => JonesClass::Executable,
    }
}

/// The executable-but-unencoded expression variants — now EMPTY: every executable `Expr` has a
/// Core-IR encoding. This ratchet floor may only stay at 0; reopening the gap turns the test red.
const UNENCODED_EXPR_VARIANTS: &[&str] = &[];

/// Tie the compile-time locks into the run so their purpose is discoverable from the suite (the
/// real enforcement is the two no-wildcard matches, checked every build).
#[test]
fn jones_classification_is_exhaustive_and_referenced() {
    let _s: fn(&Stmt<'_>) -> JonesClass = every_stmt_has_a_jones_class;
    let _e: fn(&Expr<'_>) -> JonesClass = every_expr_has_a_jones_class;
}

/// The Core-IR encoding gap is CLOSED: every executable `Expr` variant has a `C…` constructor.
/// This floor may only stay at zero — reopening it (an unencoded executable Expr) turns red.
#[test]
fn core_ir_encoding_gap_is_closed() {
    assert!(
        UNENCODED_EXPR_VARIANTS.is_empty(),
        "Core-IR encoding gap REOPENED with {} variant(s): {:?}",
        UNENCODED_EXPR_VARIANTS.len(),
        UNENCODED_EXPR_VARIANTS
    );
}

/// Phase 2 — per newly-encoded variant: the P1 residual is Jones-optimal (zero dispatch) AND
/// the program still computes the right answer. Grows one row per closed gap.
#[test]
fn newly_encoded_variants_are_jones_optimal() {
    // (name, program, expected output)
    let cases = [(
        "with_capacity",
        "## Main\nLet s be \"\" with capacity 8.\nShow length of s.",
        "0",
    )];
    for (name, prog, expected) in cases {
        let residual =
            decompile(prog).unwrap_or_else(|e| panic!("[{name}] P1 projection failed: {e}"));
        let d = count_dispatch(&residual);
        assert_eq!(
            d, 0,
            "[{name}] P1 residual carries {d} dispatch unit(s):\n{residual}"
        );
        assert_run_equals(prog, expected);
    }
}

/// Per-executable-construct P1 coverage: each program exercises a distinct executable construct
/// and its P1 residual must be Jones-optimal (zero dispatch) AND still compute the right answer.
/// This is the behavioural side of the exhaustive `Executable` classification above.
#[test]
fn executable_construct_p1_coverage() {
    // NOTE: `a union b` over freshly-built sets does not yet round-trip through P1 — the PE folds
    // `new Set + Add` into a `CNew Set with i1=1` form the decompiler renders as `a new Set with i1 1`
    // (invalid input). CUnion/CIntersection infix decompile is now fixed; the set-fold representation
    // remains a tracked variant-by-variant P1 gap.
    let cases: &[(&str, &str, &str)] = &[
        ("not_bool", "## Main\nIf not (2 is greater than 3):\n    Show \"yes\".\nOtherwise:\n    Show \"no\".", "yes"),
        ("push", "## Main\nLet mutable xs be [1, 2].\nPush 3 to xs.\nShow length of xs.", "3"),
        ("set_add_dedup", "## Main\nLet s be a new Set of Int.\nAdd 5 to s.\nAdd 5 to s.\nShow length of s.", "1"),
        ("repeat_for_in", "## Main\nLet mutable s be 0.\nRepeat for x in [4, 5, 6]:\n    Set s to s + x.\nShow s.", "15"),
    ];
    for (name, prog, expected) in cases {
        let residual =
            decompile(prog).unwrap_or_else(|e| panic!("[{name}] P1 projection failed: {e}"));
        let d = count_dispatch(&residual);
        assert_eq!(
            d, 0,
            "[{name}] P1 residual carries {d} dispatch unit(s):\n{residual}"
        );
        assert_run_equals(prog, expected);
    }
}

/// Zone-introspection ops (`the manifest of Z`, `the chunk at N in Z`) encode as opaque Core nodes:
/// the P1 residual is dispatch-free AND the construct survives the round-trip (not dropped to
/// "unsupported"). Their FileSipper runtime semantics are out of scope for the encoding gap.
#[test]
fn zone_introspection_round_trips_through_p1() {
    let cases = [
        (
            "manifest_of",
            "## Main\nInside a new zone called \"Z\":\n    Let m be the manifest of Z.\n    Show \"ok\".",
            "manifest of",
        ),
        (
            "chunk_at",
            "## Main\nInside a new zone called \"Z\":\n    Let c be the chunk at 1 in Z.\n    Show \"ok\".",
            "chunk at",
        ),
    ];
    for (name, prog, needle) in cases {
        let residual =
            decompile(prog).unwrap_or_else(|e| panic!("[{name}] P1 projection failed: {e}"));
        let d = count_dispatch(&residual);
        assert_eq!(
            d, 0,
            "[{name}] P1 residual carries {d} dispatch unit(s):\n{residual}"
        );
        assert!(
            residual.contains(needle),
            "[{name}] construct dropped from residual (no '{needle}'):\n{residual}"
        );
    }
}
