//! ★ LOCK — resolving a bare variant name (`a new Failure with …`, `North`) to the enum that
//! declares it, when more than one type could. ★
//!
//! `crates/logicaffeine_language/src/analysis/registry.rs::TypeRegistry::find_variant` used to
//! walk a `HashMap` and return the FIRST enum it happened to iterate that declared a variant of
//! the given name — an order `HashMap`'s own per-process random seed changes on every run, so a
//! program with two enums sharing a variant name could compile to a DIFFERENT value each time it
//! was built, with no error and no warning. `find_variant_candidates` now returns every matching
//! enum sorted by its own `Symbol` (a fixed, deterministic intern index), and
//! `parser/mod.rs::find_variant` (the construction-site wrapper) refuses outright when more than
//! one candidate remains after the loader's own declarer-wins bare-variant scanner
//! (`loader.rs`, release-prep) has had its say — a bare construction has no context to prefer one
//! real enum over another the way an `Inspect` arm can (`resolve_arm_variant`, established from
//! an earlier arm).

use logicaffeine_compile::compile::compile_to_program;
use logicaffeine_compile::error::ParseErrorKind;

// ─── `a new <Variant> from <Enum>` — an enum names its own variant directly ────────────────────
//
// An enum is a namespace of its own variants: `from <Enum>` reuses the SAME preposition Phase 36
// uses for `<Type> from <Module>`, but resolves along a completely different axis (a variant NAME
// inside the registry, never an import). `resolve_construction_target`
// (`crates/logicaffeine_language/src/parser/mod.rs`) tells the two apart: a module contributes
// only its OWN qualified `Module::Type` names to the registry, never an entry for the module's
// bare name itself, while an enum IS registered under its own bare name — so `<qualifier>::<base>`
// being a real type and `<qualifier>` being a real enum declaring `<base>` are two independent,
// checkable facts, not a guess.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// Both tiers, same answer, no error — see `option_of_record_lock.rs`'s identical helper.
fn both(src: &str, want: &[&str]) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker must run it:\n{src}");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM must run it:\n{src}");
    assert_eq!(
        tw.output.lines().collect::<Vec<_>>(),
        want,
        "tree-walker:\n{src}"
    );
    assert_eq!(vm.output.lines().collect::<Vec<_>>(), want, "VM:\n{src}");
}

const TWO_FAILURES_WITH_DIFFERENT_FIELDS: &str = "\
## An Outcome is either:
    Success (value: Int).
    Failure (message: Text).

## A Status is either:
    Up.
    Failure (code: Int).

";

/// `with …` fields: each `from` picks its own enum's OWN field shape, not whichever candidate
/// `find_variant` would have guessed at (the two `Failure`s do not even share a field name).
#[test]
fn from_qualified_construction_with_fields_picks_the_named_enum_on_both_tiers() {
    let src = format!(
        "{TWO_FAILURES_WITH_DIFFERENT_FIELDS}\
         ## Main
Let a be a new Failure from Outcome with message \"timeout\".
Let b be a new Failure from Status with code 500.
Inspect a:
    When Success (v): Show \"wrong a\".
    When Failure (m): Show m.
Inspect b:
    When Up: Show \"wrong b\".
    When Failure (c): Show c.
"
    );
    both(&src, &["timeout", "500"]);
}

/// `a new <Variant> from <Enum>` with NO `with …` — a zero-field variant needs no fields, but the
/// qualifier still has to resolve deterministically to ITS enum.
#[test]
fn from_qualified_construction_without_fields_picks_the_named_enum_on_both_tiers() {
    let src = "\
## A Foo is either:
    Ping.
    Other.

## A Bar is either:
    Ping.
    Different (x: Int).

## Main
Let a be a new Ping from Foo.
Let b be a new Ping from Bar.
Inspect a:
    When Other: Show \"wrong a\".
    When Ping: Show \"foo\".
Inspect b:
    When Different (x): Show \"wrong b\".
    When Ping: Show \"bar\".
";
    both(src, &["foo", "bar"]);
}

/// The bare-variant-word construction site (no `a new` at all, Phase 33's `North`) accepts the
/// same qualifier — the third of the three construction sites `resolve_construction_target`
/// serves.
#[test]
fn from_qualified_construction_as_a_bare_variant_word_picks_the_named_enum_on_both_tiers() {
    let src = "\
## A Foo is either:
    Ping.
    Other.

## A Bar is either:
    Ping.
    Different (x: Int).

## Main
Let a be Ping from Foo.
Let b be Ping from Bar.
Inspect a:
    When Other: Show \"wrong a\".
    When Ping: Show \"foo\".
Inspect b:
    When Different (x): Show \"wrong b\".
    When Ping: Show \"bar\".
";
    both(src, &["foo", "bar"]);
}

/// The bare `new` construction site (no article — Phase 31's `new Point`) accepts the qualifier
/// too: all THREE sites share the one `resolve_construction_target`, so a fourth site can never
/// again drift out of sync with the other two the way the tree_depth bug once did.
#[test]
fn from_qualified_construction_via_bare_new_picks_the_named_enum() {
    let src = format!(
        "{TWO_FAILURES_WITH_DIFFERENT_FIELDS}\
         ## Main
Let a be new Failure from Status with code 404.
Inspect a:
    When Up: Show \"wrong\".
    When Failure (c): Show c.
"
    );
    both(&src, &["404"]);
}

/// `from <Enum>` where `<Enum>` is real but does not declare the variant asked for: refused,
/// naming the enum actually asked for and its nearest-spelled variant — `crate::suggest::
/// find_similar`'s own Levenshtein-2 threshold ('Failuer' is a transposition of 'Failure', a
/// distance-2 typo, and 'Status' declares no 'Failure' variant at all here).
#[test]
fn from_an_enum_missing_the_variant_is_refused_with_did_you_mean() {
    let src = "\
## An Outcome is either:
    Success (value: Int).
    Failure (message: Text).

## A Status is either:
    Up.
    Failuer (code: Int).

## Main
Let a be a new Failure from Status with message \"boom\".
Show a.
";
    let err = compile_to_program(src)
        .expect_err("'Status' declares no 'Failure' variant and must refuse, not fall through");
    let message = match &err.kind {
        ParseErrorKind::Custom(msg) => msg.clone(),
        other => format!("{other:?}"),
    };
    assert!(
        message.contains("'Failure' is not a variant of 'Status'"),
        "{message}"
    );
    assert!(message.contains("Did you mean 'Failuer'"), "{message}");
}

/// `from <X>` where `<X>` names no enum at all — refused, but honestly: it does not invent a
/// qualified-construction syntax that would not actually help ('Widget' stays a plain struct),
/// and instead teaches the `from` forms that DO exist for `Failure`.
#[test]
fn from_something_that_is_not_an_enum_is_refused_naming_the_real_candidates() {
    let src = "\
## An Outcome is either:
    Success (value: Int).
    Failure (message: Text).

## A Widget has:
    a name, which is Text.

## Main
Let a be a new Failure from Widget with message \"boom\".
Show a.
";
    let err = compile_to_program(src)
        .expect_err("'Widget' is not an enum and must refuse a variant-qualified construction");
    let message = match &err.kind {
        ParseErrorKind::Custom(msg) => msg.clone(),
        other => format!("{other:?}"),
    };
    assert!(message.contains("'Widget' is not an enum"), "{message}");
    assert!(
        message.contains("`a new Failure from Outcome`"),
        "{message}"
    );
}

/// Phase 36 module qualification still works, UNTOUCHED, when the qualifier resolves to no enum
/// at all: `resolve_construction_target` only ever intercepts a `from` whose base name is
/// SOMEWHERE a variant (`Point` never is), so an ordinary imported struct is unaffected. The
/// full round trip (a real cross-file import, `Uses […](file:…)`) is `phase36_modules.rs`'s own
/// `test_recursive_discovery_imports_types`/`test_compile_project_with_imports` — re-run here as
/// evidence, not duplicated per rule 7.
#[test]
fn module_qualification_is_unaffected_when_the_base_name_is_never_a_variant() {
    let src = "\
## A Point has:
    an x, which is Int.
    a y, which is Int.

## Main
Let p be a new Point with x 1 and y 2.
Show p's x.
";
    let program = compile_to_program(src);
    assert!(
        program.is_ok(),
        "an ordinary struct construction must be untouched: {:?}",
        program.err()
    );
}

/// The bare, ambiguous form's refusal (unchanged: no context, no `from`, two candidates) now
/// TEACHES the qualified form with the author's OWN enum names, instead of only telling them to
/// rename something.
#[test]
fn the_bare_ambiguous_refusal_teaches_the_from_form() {
    let src = "\
## An Outcome is either:
    Success (value: Int).
    Failure (message: Text).

## A Status is either:
    Up.
    Failure (code: Int).

## Main
Let a be a new Failure with message \"boom\".
Show a.
";
    let err = compile_to_program(src)
        .expect_err("two user enums sharing a variant name must still refuse a bare construction");
    let message = match &err.kind {
        ParseErrorKind::Custom(msg) => msg.clone(),
        other => format!("{other:?}"),
    };
    assert!(
        message.contains("`a new Failure from Outcome`"),
        "{message}"
    );
    assert!(message.contains("`a new Failure from Status`"), "{message}");
}

/// A qualifier naming BOTH a real enum's own variant AND an imported module's own qualified type
/// at once is a genuine clash neither reading can silently win — refused, naming the qualifier
/// and the variant, not guessed.
#[test]
fn an_enum_and_a_same_named_import_module_clash_and_refuse() {
    use logicaffeine_compile::compile::{compile_project, CompileError};
    use std::fs;

    let temp_dir = tempfile::tempdir().unwrap();
    fs::write(
        temp_dir.path().join("store.md"),
        "# Store\n\n## Definition\nA Cart has:\n    an id, which is Int.\n",
    )
    .unwrap();
    let main_path = temp_dir.path().join("main.md");
    fs::write(
        &main_path,
        "# Main\n\n\
         Uses [Store](file:./store.md) for carts.\n\n\
         ## A Store is either:\n    Cart (quantity: Int).\n    Empty.\n\n\
         ## Main\n\
         Let x be a new Cart from Store.\n\
         Show \"unreachable\".\n",
    )
    .unwrap();
    let err = compile_project(&main_path).expect_err(
        "'Store' naming both a real enum's own variant and an imported module's qualified type \
         must refuse — nothing here can tell which reading was meant",
    );
    let message = match err {
        CompileError::Parse(pe) => match pe.kind {
            ParseErrorKind::Custom(msg) => msg,
            other => format!("{other:?}"),
        },
        other => panic!("expected a Parse error, got {other:?}"),
    };
    assert!(
        message.contains("Store"),
        "must name the clashing qualifier: {message}"
    );
    assert!(
        message.contains("Cart"),
        "must name the clashing variant: {message}"
    );
}

/// The SAME ambiguous source through the qualified `from` construction AND its two new refusal
/// shapes, compiled 20 times over each — every message must be byte-identical every time (the
/// same `find_variant_candidates`/`variant_in` determinism `variant_ambiguity_resolution_is_
/// deterministic_across_twenty_compiles` already locks for the bare form, extended to the new
/// qualified-construction refusals).
#[test]
fn qualified_construction_refusals_are_deterministic_across_twenty_compiles() {
    const MISSING_VARIANT: &str = "\
## An Outcome is either:
    Success (value: Int).
    Failure (message: Text).

## A Status is either:
    Up.
    Failuer (code: Int).

## Main
Let a be a new Failure from Status with message \"boom\".
Show a.
";
    const NOT_AN_ENUM: &str = "\
## An Outcome is either:
    Success (value: Int).
    Failure (message: Text).

## A Widget has:
    a name, which is Text.

## Main
Let a be a new Failure from Widget with message \"boom\".
Show a.
";
    for src in [MISSING_VARIANT, NOT_AN_ENUM] {
        let mut messages: Vec<String> = Vec::with_capacity(20);
        for run in 0..20 {
            let err = compile_to_program(src).expect_err(&format!("run {run}: must still refuse"));
            messages.push(match err.kind {
                ParseErrorKind::Custom(msg) => msg,
                other => format!("{other:?}"),
            });
        }
        let first = &messages[0];
        for (run, message) in messages.iter().enumerate() {
            assert_eq!(
                message, first,
                "run {run}'s refusal text differs from run 0's:\nrun 0: {first}\nrun {run}: {message}"
            );
        }
    }
}

/// A user's own enum shadows a same-named stdlib variant: `Outcome`'s `Failure` beats
/// `assets/std/io.lg`'s `Severity::Failure` for a BARE, unqualified `a new Failure with …` —
/// release-prep's `loader.rs` declarer-wins fix keeps `Severity` out of the registry at all once
/// the user's own `Failure` is visible to it, so there is only ever ONE candidate here, not an
/// ambiguity this lock's own refusal (below) would otherwise have to catch. Verified by an
/// OBSERVABLE difference, not just "it compiled": `Outcome::Failure`'s own `message` field is
/// read back by name — if a stale registry ever let `Severity::Failure` (no such field) win
/// instead, the program would refuse to compile or the field read would come back empty/wrong,
/// not print `boom`.
#[test]
fn a_user_enum_shadows_a_stdlib_variant_of_the_same_name() {
    const USER_FAILURE_SHADOWS_STDLIB_SEVERITY: &str = "\
## Definition
An Outcome is either:
    Success (value: Int).
    Failure (message: Text).

## Main
Let x be a new Failure with message \"boom\".
Inspect x:
    When Success (v): Show \"wrong: success\".
    When Failure (m): Show m.
";
    let program = compile_to_program(USER_FAILURE_SHADOWS_STDLIB_SEVERITY);
    assert!(
        program.is_ok(),
        "a bare `a new Failure` must resolve to the user's OWN `Outcome`, not refuse as \
         ambiguous with the stdlib `Severity`'s same-named variant — declarer wins: {:?}",
        program.err()
    );
}

/// Two USER enums (never stdlib) declaring the SAME variant name, in a bare, unqualified
/// construction, is a genuine ambiguity nothing can silently resolve — refused, naming both
/// enums, per the language decision that adding a qualifying construction syntax
/// (`a new Outcome's Failure`, `a new Failure of Outcome`) is Tristen's call, not made here
/// because no such form exists in the grammar today (checked directly, not assumed).
#[test]
fn two_user_enums_sharing_a_variant_name_refuse_a_bare_construction() {
    const TWO_ENUMS_SHARE_FAILURE: &str = "\
## An Outcome is either:
    Success (value: Int).
    Failure (message: Text).

## A Report is either:
    Clean.
    Failure (reason: Text).

## Main
Let x be a new Failure with message \"boom\".
Show x.
";
    let err = compile_to_program(TWO_ENUMS_SHARE_FAILURE)
        .expect_err("two user enums sharing a variant name must refuse a bare construction");
    let message = match &err.kind {
        ParseErrorKind::Custom(msg) => msg.clone(),
        other => format!("{other:?}"),
    };
    assert!(
        message.contains("Outcome"),
        "the refusal must name 'Outcome': {message}"
    );
    assert!(
        message.contains("Report"),
        "the refusal must name 'Report': {message}"
    );
    assert!(
        message.contains("Failure"),
        "the refusal must name the ambiguous variant itself: {message}"
    );
}

/// The SAME ambiguous source, compiled 20 times over — the refusal (which of the two enums is
/// named first, in what order) must be BYTE-IDENTICAL every time. Each `compile_to_program` call
/// runs its own `DiscoveryPass` into a fresh `TypeRegistry`, whose backing `HashMap` gets a
/// FRESH, independently-randomized hasher per Rust's own `RandomState` (reseeded per `new()`,
/// not once per process) — so 20 iterations in one test process genuinely exercise 20 different
/// iteration orders under the hood, and would have caught the original bug (before
/// `find_variant_candidates`'s own sort) by producing more than one distinct message across the
/// 20 runs.
#[test]
fn variant_ambiguity_resolution_is_deterministic_across_twenty_compiles() {
    const TWO_ENUMS_SHARE_FAILURE: &str = "\
## An Outcome is either:
    Success (value: Int).
    Failure (message: Text).

## A Report is either:
    Clean.
    Failure (reason: Text).

## Main
Let x be a new Failure with message \"boom\".
Show x.
";
    let mut messages: Vec<String> = Vec::with_capacity(20);
    for run in 0..20 {
        let err = compile_to_program(TWO_ENUMS_SHARE_FAILURE)
            .expect_err(&format!("run {run}: still ambiguous, must still refuse"));
        messages.push(match err.kind {
            ParseErrorKind::Custom(msg) => msg,
            other => format!("{other:?}"),
        });
    }
    let first = &messages[0];
    for (run, message) in messages.iter().enumerate() {
        assert_eq!(
            message, first,
            "run {run}'s refusal text differs from run 0's — variant resolution is not \
             deterministic:\nrun 0: {first}\nrun {run}: {message}"
        );
    }
}
