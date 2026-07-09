//! ════════════════════════════════════════════════════════════════════════════════════════════
//! FUTAMURA ALL-STATEMENTS LOCK — every self-interpreter statement constructor must survive EVERY
//! partial-evaluator dialect AND the decompiler. This is the COMPLETENESS axis of the Futamura
//! coverage (the dual of Jones optimality, which is the QUALITY axis): Jones optimality proves the
//! residual is GOOD (fewer `Inspect` dispatch nodes than the PE — the interpretation layer was
//! removed); THIS file proves the residual is COMPLETE (no statement is silently `Otherwise: skip`-ed
//! out of existence). The `CStreamMessage` regression that opened this campaign was Jones-optimal AND
//! YET dropped a whole construct — optimal but incomplete. You need both axes locked.
//!
//! TWO complementary locks:
//!
//!   • STATIC, CATALOG-COMPLETE (`every_self_interpreter_source_constructor_…`). Each of the THREE PE
//!     dialects (`peBlock`/`peBlockM`/`peBlockB` for statements, `peExpr`/`peExprM`/`peExprB` for
//!     expressions) ends its `Inspect` in `Otherwise: skip`, so ANY source constructor with NO `When`
//!     arm is SILENTLY DROPPED when that dialect specializes a program using it.
//!     `tier_parity_lock::all_three_pe_dialects_dispatch_on_identical_constructors` proves the three
//!     dialects AGREE WITH EACH OTHER — but three dialects that ALL miss the same constructor agree
//!     vacuously and still drop it. This lock closes that gap over the WHOLE source-AST surface: every
//!     `CExpr` / `CStmt` / `CSelectBranch` / `CMatchArm` / `CStringPart` constructor DECLARED IN THE
//!     SELF-INTERPRETER TYPE CATALOG (`core_types_for_pe_source`) must have a `When` arm in pe_source,
//!     pe_mini, pe_bti AND the decompiler. The TYPE CATALOG is the spec; every projection surface must
//!     cover it — expressions as much as statements.
//!
//!   • BEHAVIOURAL, END-TO-END (`every_opaque_statement_survives_residualization_through_every_dialect`).
//!     Statically having a `When` arm is necessary but not sufficient — the arm must actually RE-EMIT
//!     the statement, not eat it. So every networking / concurrency / CRDT / I-O / meta / data
//!     statement (the constructors the PE treats as opaque pass-throughs) is residualized through all
//!     three real dialects and must come back out of the residual 1:1 — none dropped. Control-flow
//!     constructors (`CIf`/`CWhile`/`CRepeat`/…) are excluded from the 1:1 batch ONLY because the PE
//!     legitimately TRANSFORMS them (that is its job, and it is covered behaviourally by the
//!     Jones/`the_trick`/factorial PE tests); they remain covered HERE by the static lock. A
//!     catalog-tied guard (`behavioural ∪ excluded == catalog`) makes it impossible to add a new
//!     statement and silently leave it out of this behavioural sweep.
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS FILE TO MAKE A RED CASE PASS.  ⚠️
//!  A RED means a constructor is dropped from a projection (no `When` arm, or an arm that eats it) or
//!  a new statement slipped past the coverage guard. The fix is in the DIALECT
//!  (`optimize/pe_source.logos`, `pe_mini_source.logos`, `pe_bti_source.logos`,
//!  `decompile_source.logos`) or in the encoder (`compile::encode_stmt_src`) — NEVER by relaxing an
//!  assertion, deleting a row, or adding an `_ =>`/`Otherwise` escape hatch here. Strictly monotone:
//!  add coverage, never remove it.
//! ════════════════════════════════════════════════════════════════════════════════════════════

use std::collections::BTreeSet;

mod common;

use logicaffeine_compile::compile::{
    core_types_for_pe_source, decompile_source_text, pe_bti_source_text, pe_mini_source_text,
    pe_source_text,
};

/// Every `When C…` constructor an `Inspect`-dispatching self-interpreter source handles. Mirrors the
/// extraction in `tier_parity_lock` so the two locks read the dialect sources the same way.
fn when_constructors(src: &str) -> BTreeSet<String> {
    src.lines()
        .filter_map(|l| {
            l.trim().strip_prefix("When C").map(|rest| {
                let name: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
                format!("C{name}")
            })
        })
        .collect()
}

/// The `C…` constructors declared under a `## A <TypeName> is one of:` block of the self-interpreter
/// type catalog (the inductive `CStmt` / `CSelectBranch` definitions). Parsed from the SHIPPED runtime
/// catalog (`core_types_for_pe_source`), not a test fixture, so the lock is grounded in what the
/// projections actually run against.
fn catalog_constructors(catalog: &str, type_name: &str) -> BTreeSet<String> {
    let header = format!("## A {type_name} is one of:");
    let mut out = BTreeSet::new();
    let mut in_block = false;
    for line in catalog.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") {
            in_block = trimmed == header;
            continue;
        }
        if in_block {
            if let Some(rest) = trimmed.strip_prefix("A C") {
                let name: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
                if !name.is_empty() {
                    out.insert(format!("C{name}"));
                }
            }
        }
    }
    out
}

/// The statement-level constructors: every `CStmt` plus every `CSelectBranch`. Used by the
/// behavioural opaque batch + its partition guard (the 1:1-survival sweep is statement-scoped because
/// the PE legitimately *folds* expressions but must *preserve* opaque statements).
fn statement_catalog() -> BTreeSet<String> {
    let catalog = core_types_for_pe_source();
    let mut s = catalog_constructors(catalog, "CStmt");
    s.extend(catalog_constructors(catalog, "CSelectBranch"));
    s
}

/// The COMPLETE source-AST surface the PE walks and the decompiler renders: expressions, statements,
/// select arms, match arms, and interpolated-string parts. Excludes `CProg`/`CFuncDef` (handled by
/// the dedicated program/function specialization path, not the `Inspect` body/expression dispatch)
/// and the runtime `CVal`/`PEState` types (machine state, not source AST). EVERY one of these must be
/// dispatched by every projection surface — an expression with no `When` arm is dropped just as surely
/// as a statement.
fn source_ast_catalog() -> BTreeSet<String> {
    let catalog = core_types_for_pe_source();
    let mut s = BTreeSet::new();
    for ty in ["CExpr", "CStmt", "CSelectBranch", "CMatchArm", "CStringPart"] {
        s.extend(catalog_constructors(catalog, ty));
    }
    s
}

/// ★ STATIC, CATALOG-COMPLETE LOCK ★ — every SOURCE-AST constructor in the self-interpreter type
/// catalog (expression OR statement OR select-arm OR match-arm OR string-part) has a `When` arm in
/// ALL THREE PE dialects AND the decompiler. A missing arm = the construct is `Otherwise: skip`-
/// dropped from that projection (or cannot be decompiled back to surface syntax). This is the
/// COMPLETENESS half of Futamura coverage over the WHOLE language surface, not just statements.
#[test]
fn every_self_interpreter_source_constructor_is_handled_by_every_dialect_and_decompile() {
    let catalog = source_ast_catalog();
    assert!(
        catalog.len() >= 90,
        "CATALOG-COMPLETE LOCK is vacuous: only {} source-AST constructors parsed from the runtime \
         catalog (expected ~96) — the parser or the catalog format changed.",
        catalog.len()
    );

    for (name, src) in [
        ("pe_source", pe_source_text()),
        ("pe_mini", pe_mini_source_text()),
        ("pe_bti", pe_bti_source_text()),
        ("decompile", decompile_source_text()),
    ] {
        let handled = when_constructors(src);
        let missing: Vec<&String> = catalog.iter().filter(|c| !handled.contains(*c)).collect();
        assert!(
            missing.is_empty(),
            "FUTAMURA COMPLETENESS REGRESSION: the `{name}` projection surface is MISSING a `When` \
             arm for these self-interpreter source constructor(s): {missing:?}. Every constructor \
             declared in the `CExpr`/`CStmt`/`CSelectBranch`/`CMatchArm`/`CStringPart` catalog MUST be \
             handled by every PE dialect and the decompiler, or it is silently `Otherwise: skip`- \
             dropped from that projection. Add the missing `When` arm to `optimize/{name}*.logos` — \
             NEVER weaken this lock.\n\nCatalog constructors: {catalog:?}"
        );
    }
}

/// The opaque, pass-through statement constructors — the ones the PE must residualize 1:1 (it neither
/// folds nor restructures them). Each entry is `(constructor name, builder expression)`. Every
/// operand is constructed FRESH inline (`(a new CVar with name "src")` etc.) rather than sharing a
/// let-bound value: the self-interpreter under test is COMPILED to Rust (value-move semantics on
/// aggregates), so sharing a non-`Copy` aggregate across two constructors would be a use-after-move in
/// the generated code, not a PE behaviour. Operands are DYNAMIC (`CVar "src"`, unbound in the PE env)
/// so the PE cannot constant-fold the statement away. Aggregate operands that need contents
/// (`branches`, `body`, select arms) are pre-built ONCE-USED in `build_setup` and named here. Field
/// names + shapes match the `CStmt` catalog exactly.
fn opaque_statement_builders() -> Vec<(&'static str, &'static str)> {
    vec![
        ("CLet", "a new CLet with name \"v0\" and expr (a new CVar with name \"src\")"),
        ("CSet", "a new CSet with name \"v1\" and expr (a new CVar with name \"src\")"),
        ("CShow", "a new CShow with expr (a new CVar with name \"src\")"),
        ("CCallS", "a new CCallS with name \"f\" and args (a new Seq of CExpr)"),
        ("CPush", "a new CPush with expr (a new CVar with name \"src\") and target \"items\""),
        ("CPop", "a new CPop with target \"items\""),
        ("CAdd", "a new CAdd with elem (a new CVar with name \"src\") and target \"s\""),
        ("CRemove", "a new CRemove with elem (a new CVar with name \"src\") and target \"s\""),
        ("CSetIdx", "a new CSetIdx with target \"a\" and idx (a new CVar with name \"src\") and val (a new CVar with name \"src\")"),
        ("CMapSet", "a new CMapSet with target \"m\" and key (a new CVar with name \"src\") and val (a new CVar with name \"src\")"),
        ("CSetField", "a new CSetField with target \"o\" and field \"f\" and val (a new CVar with name \"src\")"),
        ("CRuntimeAssert", "a new CRuntimeAssert with cond (a new CVar with name \"src\") and msg (a new CVar with name \"src\")"),
        ("CHardAssert", "a new CHardAssert with cond (a new CVar with name \"src\") and msg (a new CVar with name \"src\")"),
        ("CGive", "a new CGive with expr (a new CVar with name \"src\") and target \"r\""),
        ("CEscStmt", "a new CEscStmt with code \"raw\""),
        ("CSleep", "a new CSleep with duration (a new CVar with name \"src\")"),
        ("CReadConsole", "a new CReadConsole with target \"x\""),
        ("CReadFile", "a new CReadFile with path (a new CVar with name \"src\") and target \"x\""),
        ("CWriteFile", "a new CWriteFile with path (a new CVar with name \"src\") and content (a new CVar with name \"src\")"),
        ("CCheck", "a new CCheck with predicate (a new CVar with name \"src\") and msg (a new CVar with name \"src\")"),
        ("CAssert", "a new CAssert with proposition (a new CVar with name \"src\")"),
        ("CTrust", "a new CTrust with proposition (a new CVar with name \"src\") and justification \"axiom\""),
        ("CRequire", "a new CRequire with dependency \"math\""),
        ("CMerge", "a new CMerge with target \"x\" and other (a new CVar with name \"src\")"),
        ("CIncrease", "a new CIncrease with target \"x\" and amount (a new CVar with name \"src\")"),
        ("CDecrease", "a new CDecrease with target \"x\" and amount (a new CVar with name \"src\")"),
        ("CAppendToSeq", "a new CAppendToSeq with target \"items\" and value (a new CVar with name \"src\")"),
        ("CResolve", "a new CResolve with target \"c's active\""),
        ("CSync", "a new CSync with target \"x\" and channel (a new CVar with name \"src\")"),
        ("CMount", "a new CMount with target \"x\" and path (a new CVar with name \"src\")"),
        ("CConcurrent", "a new CConcurrent with branches concBranches"),
        ("CParallel", "a new CParallel with branches parBranches"),
        ("CLaunchTask", "a new CLaunchTask with body launchBody and handle \"_task\""),
        ("CStopTask", "a new CStopTask with handle (a new CVar with name \"src\")"),
        ("CSelect", "a new CSelect with branches selBranches"),
        ("CCreatePipe", "a new CCreatePipe with name \"ch\" and capacity (a new CVar with name \"src\")"),
        ("CSendPipe", "a new CSendPipe with chan \"ch\" and value (a new CVar with name \"src\")"),
        ("CReceivePipe", "a new CReceivePipe with chan \"ch\" and target \"val\""),
        ("CTrySendPipe", "a new CTrySendPipe with chan \"ch\" and value (a new CVar with name \"src\")"),
        ("CTryReceivePipe", "a new CTryReceivePipe with chan \"ch\" and target \"val\""),
        ("CSpawn", "a new CSpawn with agentType \"Worker\" and target \"w\""),
        ("CSendMessage", "a new CSendMessage with target (a new CVar with name \"src\") and msg (a new CVar with name \"src\")"),
        ("CStreamMessage", "a new CStreamMessage with target (a new CVar with name \"src\") and values (a new CVar with name \"src\")"),
        ("CAwaitMessage", "a new CAwaitMessage with target \"reply\""),
        ("CListen", "a new CListen with addr (a new CVar with name \"src\") and handler \"default\""),
        ("CConnectTo", "a new CConnectTo with addr (a new CVar with name \"src\") and target \"conn\""),
        ("CZone", "a new CZone with name \"critical\" and kind \"mutex\" and body zoneBody"),
    ]
}

/// Constructors deliberately NOT in the opaque 1:1 batch, each with the reason it is covered
/// elsewhere. The guard below proves `behavioural ∪ excluded == catalog`, so a NEW statement cannot
/// be silently omitted from the behavioural sweep — it must be added to one list or the other.
const EXCLUDED_FROM_OPAQUE_BATCH: &[(&str, &str)] = &[
    ("CIf", "control flow — PE legitimately specializes branches; covered by static lock + the_trick"),
    ("CWhile", "control flow — PE may unroll/transform; covered by static lock + factorial PE tests"),
    ("CRepeat", "control flow — PE may unroll a static collection; covered by static lock"),
    ("CRepeatRange", "control flow — PE may unroll a static range; covered by static lock"),
    ("CInspect", "the surface encoder desugars Inspect to flat CIf chains; covered by static lock"),
    ("CForceDynamic", "PE-internal binding-time hint, never an opaque body statement; static lock"),
    ("CStructDef", "declaration encoded at program level, not a body statement; static lock"),
    ("CEnumDef", "declaration encoded at program level, not a body statement; static lock"),
    ("CReturn", "block-terminating — dead-code-after-return changes residual count; static lock"),
    ("CBreak", "loop-terminating control flow; covered by static lock"),
    ("CSelectRecv", "select arm — exercised NESTED inside the CSelect batch entry, not at top level"),
    ("CSelectTimeout", "select arm — exercised NESTED inside the CSelect batch entry, not at top level"),
];

/// Build the `## Main` setup that constructs `testStmts` (a `Seq of CStmt`) holding one of every
/// opaque constructor, plus the shared sub-values their builders reference.
fn build_setup(builders: &[(&'static str, &'static str)]) -> String {
    let mut s = String::new();
    s.push_str("    Let testStmts be a new Seq of CStmt.\n");
    // Aggregate operands that need NON-empty contents are each built ONCE and consumed by exactly one
    // constructor below — so the compiled self-interpreter never re-uses a moved aggregate.
    s.push_str("    Let concBranches be a new Seq of Seq of CStmt.\n");
    s.push_str("    Let concInner be a new Seq of CStmt.\n");
    s.push_str("    Push (a new CShow with expr (a new CVar with name \"src\")) to concInner.\n");
    s.push_str("    Push concInner to concBranches.\n");
    s.push_str("    Let parBranches be a new Seq of Seq of CStmt.\n");
    s.push_str("    Let parInner be a new Seq of CStmt.\n");
    s.push_str("    Push (a new CShow with expr (a new CVar with name \"src\")) to parInner.\n");
    s.push_str("    Push parInner to parBranches.\n");
    s.push_str("    Let launchBody be a new Seq of CStmt.\n");
    s.push_str("    Push (a new CShow with expr (a new CVar with name \"src\")) to launchBody.\n");
    s.push_str("    Let zoneBody be a new Seq of CStmt.\n");
    s.push_str("    Push (a new CShow with expr (a new CVar with name \"src\")) to zoneBody.\n");
    s.push_str("    Let recvBody be a new Seq of CStmt.\n");
    s.push_str("    Let timeoutBody be a new Seq of CStmt.\n");
    s.push_str("    Let selBranches be a new Seq of CSelectBranch.\n");
    s.push_str("    Push (a new CSelectRecv with chan \"ch\" and var \"v\" and body recvBody) to selBranches.\n");
    s.push_str("    Push (a new CSelectTimeout with duration (a new CInt with value 100) and body timeoutBody) to selBranches.\n");
    for (_name, builder) in builders {
        s.push_str(&format!("    Push ({builder}) to testStmts.\n"));
    }
    s
}

/// Residualize `testStmts` through one dialect's block specializer and return the residual statement
/// count (`length of residual`). `block_call` is the dialect's entry point; `catalog` is the type
/// catalog the dialect is paired with (pe_bti needs the memoizing-PEState rename).
fn residual_count(catalog: &str, dialect_text: &str, block_call: &str, setup: &str) -> (bool, String) {
    let source = format!(
        "{catalog}\n{dialect_text}\n## Main\n{setup}\
         \x20   Let state be makePeState(a new Map of Text to CVal, a new Map of Text to CFunc, 200).\n\
         \x20   Let residual be {block_call}(testStmts, state).\n\
         \x20   Show length of residual.\n"
    );
    let r = common::run_logos(&source);
    (r.success, if r.success { r.stdout.trim().to_string() } else { r.stderr })
}

/// Guard: the opaque behavioural batch plus the documented exclusions EXACTLY partition the statement
/// catalog. Adding a statement to the language (a new `CStmt` catalog entry) forces it into one list
/// or the other — it can never silently escape the behavioural sweep.
#[test]
fn opaque_batch_plus_exclusions_partition_the_statement_catalog() {
    let catalog = statement_catalog();
    let behavioural: BTreeSet<String> =
        opaque_statement_builders().iter().map(|(n, _)| n.to_string()).collect();
    let excluded: BTreeSet<String> =
        EXCLUDED_FROM_OPAQUE_BATCH.iter().map(|(n, _)| n.to_string()).collect();

    // Disjoint: a constructor is either residualized 1:1 or explicitly excluded, never both.
    let overlap: Vec<&String> = behavioural.intersection(&excluded).collect();
    assert!(overlap.is_empty(), "a constructor is in BOTH the opaque batch and the exclusions: {overlap:?}");

    // Every builder/exclusion names a REAL catalog constructor (no typos).
    let union: BTreeSet<String> = behavioural.union(&excluded).cloned().collect();
    let phantom: Vec<&String> = union.difference(&catalog).collect();
    assert!(
        phantom.is_empty(),
        "the opaque batch / exclusions reference constructor(s) NOT in the self-interpreter catalog \
         (typo?): {phantom:?}"
    );

    // Complete: nothing in the catalog is left uncovered by both lists.
    let uncovered: Vec<&String> = catalog.difference(&union).collect();
    assert!(
        uncovered.is_empty(),
        "FUTAMURA COVERAGE GAP: statement constructor(s) {uncovered:?} are in the catalog but neither \
         in the opaque behavioural batch nor the documented exclusions. A new statement must be added \
         to `opaque_statement_builders` (if the PE treats it as an opaque pass-through) or to \
         `EXCLUDED_FROM_OPAQUE_BATCH` with a reason — never left to silently escape the sweep."
    );
}

/// ★ BEHAVIOURAL, END-TO-END LOCK ★ — every opaque statement constructor survives residualization
/// through ALL THREE real PE dialects 1:1 (none dropped, none duplicated). This proves the `When`
/// arms (verified present by the static lock) actually RE-EMIT the statement rather than eat it.
#[test]
fn every_opaque_statement_survives_residualization_through_every_dialect() {
    let builders = opaque_statement_builders();
    let expected = builders.len();
    let setup = build_setup(&builders);
    let base_catalog = core_types_for_pe_source().to_string();
    // pe_bti pairs with the memoizing PEState (specResults→memoCache, onStack→callGuard).
    let bti_catalog = base_catalog.replace("specResults", "memoCache").replace("onStack", "callGuard");

    for (name, catalog, dialect, block_call) in [
        ("pe_source", base_catalog.as_str(), pe_source_text(), "peBlock"),
        ("pe_mini", base_catalog.as_str(), pe_mini_source_text(), "peBlockM"),
        ("pe_bti", bti_catalog.as_str(), pe_bti_source_text(), "peBlockB"),
    ] {
        let (ok, out) = residual_count(catalog, dialect, block_call, &setup);
        assert!(
            ok,
            "FUTAMURA SURVIVAL REGRESSION: the `{name}` dialect failed to COMPILE+RUN the opaque \
             all-statements batch (an undefined per-dialect helper, or a malformed `When` arm). Fix \
             the dialect, not this lock.\n\nstderr:\n{out}"
        );
        let count: usize = out.parse().unwrap_or(0);
        assert_eq!(
            count, expected,
            "FUTAMURA SURVIVAL REGRESSION: the `{name}` dialect residualized {count} statements but \
             {expected} opaque statements were fed in — a constructor was DROPPED (a `When` arm that \
             eats the statement instead of re-emitting it) or duplicated. Every opaque statement must \
             survive 1:1. Fix the dialect (`optimize/{name}*.logos`), not this lock."
        );
    }
}
