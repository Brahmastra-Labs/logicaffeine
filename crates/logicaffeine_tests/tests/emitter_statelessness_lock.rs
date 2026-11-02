//! ★ LOCK — codegen must not depend on what was compiled before it. ★
//!
//! The emitter keeps state in thread-locals. Most of it is a whole-program FACT installed once per
//! compile (which functions return a bignum, which fields are exact, the range oracle) and safely
//! overwritten by the next one. But `BIGNUM_TMP`, the fresh-temp counter for bignum operands, was
//! monotonic across a program and **never reset** — so compiling the same source twice in one
//! process named its temps `__lbop0` the first time and `__lbop27` the second.
//!
//! ★ WHY IT HID FOR SO LONG. The only vantage that can see it is a caller that compiles more than
//! once in one process, and almost nothing does: a `largo build` of a single program compiles once
//! and exits. `phase_codegen_determinism` and `phase_opt_trace` compile twice on purpose and caught
//! it — but only once a corpus program started using the bignum path at all, which happened by
//! accident, years after the counter was written.
//!
//! ★ AND `largo build` OF A SITE IS EXACTLY THAT CALLER. A `--web` project compiles every
//! `pages/*.lg` in one process, so page two's output depended on page one. Reproducible builds,
//! content-addressed asset names and the optimisation-trace soundness check all rest on this.
//!
//! Two halves, because the bug has two shapes:
//!
//!   1. **Interleaving** — compiling B between two compiles of A must not change A. That is the
//!      field shape, and it is stronger than compiling A twice in a row: state seeded from A alone
//!      may look stable while state that B perturbs does not.
//!   2. **A catalogue of the emitter's global state**, so a NEW thread-local cannot be added
//!      without deciding how it is scoped. The counter was not a mistake anybody made — it was a
//!      decision nobody was asked to take.

use logicaffeine_compile::compile::compile_to_rust;

/// Programs chosen to exercise the state the emitter actually keeps: bignum temps, promoted record
/// fields, the function-return map, and the range oracle.
const A: &str = "\
## A Cell has:
    a v, which is Int.

## To grow (c: Cell) -> Int:
    Return c's v * 2.

## Main
Let a be a new Cell with v (2 ** 70).
Let b be a new Cell with v grow(a).
Show b's v.
";

const B: &str = "\
## To fact (n: Int) -> Int:
    Let mutable p be 1.
    Let mutable i be 1.
    While i is at most n:
        Set p to p * i.
        Set i to i + 1.
    Return p.

## Main
Show fact(30).
";

const C: &str = "\
## Main
Let mutable total be 0.
Let mutable i be 1.
While i is at most 10:
    Set total to total + i.
    Set i to i + 1.
Show total.
";

/// ★ The SIMD-reduce shape: a transcendental accumulated over a counted loop, which the AVX2
/// peephole lifts into a GENERATED Rust function whose NAME carries a counter. A generated name is
/// emitter state that reaches the output directly, so it is exactly the place a shared counter is
/// most visible and least excusable.
const D: &str = "\
## Main
Let mutable sum be 0.
Let mutable i be 1.
While i is at most 1000:
    Set sum to sum + trunc(sin(toReal(i) / 1000.0) * 1000000.0).
    Set i to i + 1.
Show sum.
";

/// ★ The fused map test: `If m contains k:` over a read of `item k of m` binds the value once, in a GENERATED
/// `__probeN` whose number comes from a counter. Two tests in one program get distinct numbers.
const E: &str = "\
## Main
Let m be a new Map of Int to Int.
Set m at 5 to 42.
Set m at 6 to 7.
If m contains 5:
    Show item 5 of m.
If m contains 6:
    Show item 6 of m.
";

#[test]
fn compiling_another_program_in_between_does_not_change_the_first() {
    // ★ THE HEADLINE, and the shape `largo build` of a multi-page site actually has.
    let first = compile_to_rust(A).expect("A compiles");
    let _ = compile_to_rust(B).expect("B compiles");
    let again = compile_to_rust(A).expect("A compiles again");
    assert_eq!(
        first, again,
        "compiling B between two compiles of A changed A's output — the emitter is carrying state \
         across programs"
    );
}

#[test]
fn a_whole_sequence_of_programs_is_order_independent() {
    // Every program in the set, compiled alone, must equal itself compiled after all the others.
    // A single interleaving can miss state that only accumulates past a threshold.
    let alone: Vec<String> = [A, B, C, D, E]
        .iter()
        .map(|s| compile_to_rust(s).expect("compiles"))
        .collect();
    for (i, src) in [A, B, C, D, E].iter().enumerate() {
        for other in [A, B, C, D, E, B, A] {
            let _ = compile_to_rust(other).expect("compiles");
        }
        let after = compile_to_rust(src).expect("compiles");
        assert_eq!(
            alone[i], after,
            "program {i} compiles differently after other programs have been compiled"
        );
    }
}

#[test]
fn the_bignum_temp_counter_restarts_for_every_program() {
    // The specific instance, named, so a regression points straight at the cause rather than at
    // "codegen is non-deterministic".
    let first = compile_to_rust(A).expect("compiles");
    assert!(
        first.contains("__lbop"),
        "this fixture must exercise the bignum operand path:\n{first}"
    );
    for _ in 0..5 {
        let _ = compile_to_rust(B).expect("compiles");
    }
    let again = compile_to_rust(A).expect("compiles");
    assert_eq!(
        first, again,
        "the bignum temp counter carried over from earlier compiles"
    );
}

#[test]
fn the_simd_reduce_helper_is_named_the_same_way_every_time() {
    // The second instance of the same class, named for the same reason: a regression points at the
    // counter rather than at "codegen is non-deterministic". The helper's name is EMITTED, so this
    // counter does not merely affect a temporary — it renames a function in the shipped source.
    let first = compile_to_rust(D).expect("compiles");
    assert!(
        first.contains("__logos_simd_chunks_"),
        "this fixture must exercise the SIMD-reduce lifter, or it proves nothing:\n{first}"
    );
    for _ in 0..3 {
        let _ = compile_to_rust(D).expect("compiles");
    }
    let again = compile_to_rust(D).expect("compiles");
    assert_eq!(
        first, again,
        "the SIMD-reduce helper counter carried over from earlier compiles — the same program \
         emits a differently-named generated function depending on what was built before it"
    );
}

#[test]
fn the_probe_bindings_are_numbered_the_same_way_every_time() {
    // The third instance of the class: the binding's name is emitted, so a counter carried over from an earlier
    // compile renames it in the shipped source.
    let first = compile_to_rust(E).expect("compiles");
    assert!(
        first.contains("__probe0") && first.contains("__probe1"),
        "this fixture must fuse both map tests, or it proves nothing:\n{first}"
    );
    for _ in 0..3 {
        let _ = compile_to_rust(E).expect("compiles");
    }
    let again = compile_to_rust(E).expect("compiles");
    assert_eq!(
        first, again,
        "the probe binding counter carried over from earlier compiles"
    );
}

// ---------------------------------------------------------------------------------------------
// ★ THE CATALOGUE — every piece of global state the emitter keeps, and how it is scoped.
// ---------------------------------------------------------------------------------------------

/// Each `thread_local!` static in `codegen/`, with the reason it is safe to keep across compiles.
///
/// A static that is none of these is a `BIGNUM_TMP` waiting to happen, so adding one turns this
/// list RED until somebody decides which it is. That is the whole point: the counter was never a
/// mistake anyone made, it was a decision nobody was asked to take.
const CATALOGUE: &[(&str, Scope)] = &[
    // Immutable constants — nothing to reset.
    ("EMPTY_SYMS", Scope::Constant),
    ("EMPTY_TYPES", Scope::Constant),
    ("EMPTY_BOXED_FIELDS", Scope::Constant),
    ("EMPTY_REGISTRY", Scope::Constant),
    // Whole-program facts, fully overwritten by a `set_*` at codegen entry.
    ("BIGINT_RETURNING_FNS", Scope::InstalledPerProgram),
    ("BIGINT_STRUCT_FIELDS", Scope::InstalledPerProgram),
    ("ROUTE_REFUSALS", Scope::InstalledPerProgram),
    ("EXACT_HOFS", Scope::InstalledPerProgram),
    ("RANGE_ORACLE", Scope::InstalledPerProgram),
    ("FN_PARAM_TYPES", Scope::InstalledPerProgram),
    ("FN_RETURN_TYPES", Scope::InstalledPerProgram),
    // The `Opt::TextSliceReturn` / `Opt::ByteSliceReturn` families, published so an argument at a
    // borrow position knows the value is ALREADY a borrow and must not be wrapped in `.borrow()`.
    // Installed unconditionally by `set_borrowed_return_fns` at codegen entry — the two collectors
    // always run and return an empty set when their opt is off, so a program that borrows nothing
    // still OVERWRITES the previous program's set rather than inheriting it. Kept apart from
    // `FN_RETURN_TYPES` on purpose: it answers one question at one emit site and must not perturb
    // whole-program result-type inference.
    ("BORROWED_RETURN_FNS", Scope::InstalledPerProgram),
    // Per-body classification facts fully overwritten (`= vars`, never `+=`) by a `set_*` at the entry
    // of EVERY function body and Main (program.rs `set_nonneg_vars`/`set_wide_acc`), so they never
    // accumulate and never carry a stale set into a body that did not compute one.
    ("NONNEG_VARS", Scope::InstalledPerProgram),
    ("WIDE_ACC", Scope::InstalledPerProgram),
    ("TURBOFISH", Scope::InstalledPerProgram), // instantiation::install, every compile
    ("PROGRAM", Scope::InstalledPerProgram), // types::set_program_types, first thing codegen_program_inner does
    // Whether the program declares a `## Session` and lends its program record to a served handler;
    // `stmt::set_serve_facts` overwrites both at codegen entry (program.rs), every compile.
    ("SERVE_FACTS", Scope::InstalledPerProgram),
    // Caller-scoped MODE flags, each set and restored by a `with_*` guard around one compile.
    ("SEEDED_SELECT", Scope::CallerScoped),
    ("OBSERVED_CODEGEN", Scope::CallerScoped),
    ("EMBED_ASSETS", Scope::CallerScoped),
    ("FORCE_DISABLE", Scope::CallerScoped),
    ("EMIT_WIRE_IMPLS", Scope::CallerScoped),
    // Caller-scoped: `GenericParamsInScope` swaps it in for one generic function and restores it on drop.
    ("GENERIC_PARAMS", Scope::CallerScoped),
    // Caller-scoped: `LentParamsInScope` swaps in one function's lent `mutable` collection
    // parameters and restores the previous set on drop.
    ("LENT_PARAMS", Scope::CallerScoped),
    // Caller-scoped: `flow::install` swaps in one program's per-statement liveness for its emission
    // and restores the previous table on drop.
    ("LIVE_AFTER", Scope::CallerScoped),
    // Caller-scoped: `tower::begin_program` swaps in one program's reach of the exact extension for
    // its emission and restores the previous answer on drop; `tower::enter_scope` does the same for
    // the function being emitted.
    ("TOWER", Scope::CallerScoped),
    ("SCOPE", Scope::CallerScoped),
    // Caller-scoped: `absent::begin_program` swaps in one program's absence (where a stored `nothing` reaches) for its
    // emission and restores the previous one on drop; `absent::enter_function` does the same for the result type of
    // the function being emitted, and `absent::reading` for an operand an enclosing operation has read.
    ("ABSENCE", Scope::CallerScoped),
    ("RETURNING", Scope::CallerScoped),
    ("READ", Scope::CallerScoped),
    // Caller-scoped: `control::function` gives a function's or a closure's body a boundary stack of its own and
    // `control::enter` pushes a loop's or a zone's for its statement, each restoring the enclosing one on drop.
    ("BOUNDARIES", Scope::CallerScoped),
    // Set and cleared around ONE statement's emission; never crosses a statement, let alone a
    // program.
    ("MOVABLE_LAST_USE", Scope::StatementLocal),
    // Set at the outermost variant construction and cleared when it is emitted; never crosses a statement.
    ("CONSTRUCTION_USES", Scope::StatementLocal),
    // `flow::enter` publishes one statement's flow for its emission and restores the enclosing
    // statement's on drop; `flow::suspend` withdraws it for a closure body the same way.
    ("FLOW", Scope::StatementLocal),
    // `probe::within` pushes one fused map test's binding for its branch's emission and pops it after.
    ("PROBES", Scope::StatementLocal),
    // ★ THE ONE THAT ACCUMULATES, and therefore the one that must be reset.
    ("BIGNUM_TMP", Scope::ResetAtEntry),
    // The second of its kind, and the more visible: this one names an EMITTED function
    // (`__logos_simd_chunks_{id}`), so leaving it to accumulate does not merely renumber a
    // temporary — it changes the shipped source of an unchanged program. Unique WITHIN a program
    // (two SIMD loops need distinct names), reset ACROSS them.
    ("SIMD_REDUCE_ID", Scope::ResetAtEntry),
    // The third: numbers the `__probeN` binding of each fused map test. Unique within a program, reset across them.
    ("PROBE_ID", Scope::ResetAtEntry),
];

#[derive(PartialEq, Debug, Clone, Copy)]
enum Scope {
    Constant,
    InstalledPerProgram,
    CallerScoped,
    StatementLocal,
    ResetAtEntry,
}

fn codegen_sources() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../logicaffeine_compile/src/codegen");
    let mut out = Vec::new();
    for e in std::fs::read_dir(&dir)
        .expect("src/codegen exists")
        .flatten()
    {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) == Some("rs") {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            out.push((name, std::fs::read_to_string(&p).unwrap()));
        }
    }
    out.sort();
    out
}

/// Every `static NAME:` declared inside a `thread_local!` (or a `LazyLock`) in `codegen/`.
fn declared_statics() -> Vec<String> {
    let mut out = Vec::new();
    for (_, src) in codegen_sources() {
        for line in src.lines() {
            let t = line.trim();
            if let Some(rest) = t.strip_prefix("static ") {
                if let Some(name) = rest.split(':').next() {
                    let name = name.trim();
                    if !name.is_empty()
                        && name
                            .chars()
                            .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
                    {
                        out.push(name.to_string());
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn every_piece_of_emitter_state_is_catalogued() {
    let declared = declared_statics();
    let known: Vec<&str> = CATALOGUE.iter().map(|(n, _)| *n).collect();

    let uncatalogued: Vec<&String> = declared
        .iter()
        .filter(|n| !known.contains(&n.as_str()))
        .collect();
    assert!(
        uncatalogued.is_empty(),
        "new global state in the emitter, with no decision recorded about how it is scoped: {uncatalogued:?}\n\
         Add it to CATALOGUE with one of: Constant (immutable), InstalledPerProgram (a `set_*` at \
         codegen entry overwrites it), CallerScoped (a `with_*` guard restores it), StatementLocal \
         (set and cleared within one statement), or ResetAtEntry (it ACCUMULATES — reset it in \
         `begin_program_emission`). `BIGNUM_TMP` was the last one nobody chose for, and it made \
         codegen depend on what had been compiled before it."
    );

    let stale: Vec<&&str> = known
        .iter()
        .filter(|n| !declared.contains(&n.to_string()))
        .collect();
    assert!(
        stale.is_empty(),
        "CATALOGUE names state that no longer exists: {stale:?}"
    );
}

#[test]
fn everything_that_accumulates_is_reset_at_codegen_entry() {
    // The catalogue records the DECISION; this checks the decision was carried out. A
    // `ResetAtEntry` entry whose name never appears in `begin_program_emission` is a promise that
    // was written down and not kept — which is exactly how the comment claiming two field sets were
    // "identical" survived for as long as it did.
    let expr = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_compile/src/codegen/expr.rs"),
    )
    .expect("codegen/expr.rs");
    let body = expr
        .split("pub(crate) fn begin_program_emission()")
        .nth(1)
        .and_then(|s| s.split("\n}").next())
        .expect("begin_program_emission exists");
    for (name, scope) in CATALOGUE {
        if *scope == Scope::ResetAtEntry {
            assert!(
                body.contains(name),
                "`{name}` accumulates across compiles but `begin_program_emission` does not reset it"
            );
        }
    }
}

#[test]
fn begin_program_emission_runs_before_anything_is_emitted() {
    // A reset that happens after the first line is written is not a reset. It must be the first
    // thing `codegen_program_inner` does, ahead of the promotion analysis and the prelude.
    let program = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_compile/src/codegen/program.rs"),
    )
    .expect("codegen/program.rs");
    let inner = program
        .split("fn codegen_program_inner")
        .nth(1)
        .expect("the entry point exists");
    let reset = inner
        .find("begin_program_emission")
        .expect("codegen entry resets emitter state");
    let first_write = inner.find("writeln!(output").unwrap_or(usize::MAX);
    assert!(
        reset < first_write,
        "`begin_program_emission` must run before the first byte is emitted"
    );
}
