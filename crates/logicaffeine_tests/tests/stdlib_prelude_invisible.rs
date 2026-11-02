//! Stdlib "invisible exposure" (FINISH_INTERPRETER Phase B).
//!
//! Every stdlib module — concurrency/net/io/crdt value types and the env/file/random/time
//! native vocabulary — auto-imports without an explicit import, identically on native and
//! wasm, and **collision-safely**: a module is prepended only when the program references
//! its vocabulary AND does not itself define a name the module owns ("declarer wins"). This
//! keeps the benchmark corpus and every non-stdlib program byte-identical (the AOT hot-path
//! contract) and lets a user redefine `Message`/`Severity`/`Delta`/`args` without collision.
//!
//! The auto-import mechanism is tested through `apply_prelude` (the precise seam): the
//! tree-walking interpreter creates ad-hoc records even for undefined types, so a run-level
//! test alone would pass spuriously; compiled mode (`run_logos`) is the end-to-end proof,
//! because generating the Rust `struct` genuinely requires the imported definition.

mod common;

use common::{run_all_tiers, run_interpreter, run_logos};
use logicaffeine_compile::loader::apply_prelude;

// ─── Value-type modules auto-import (mechanism + end-to-end compiled) ────────

#[test]
fn net_message_autoimports() {
    let src = "## Main\n\
        \x20   Let m be a new Message with sender 7 and payload \"hi\".\n\
        \x20   Show m's sender.\n";
    let applied = apply_prelude(src);
    assert!(
        applied.contains("a sender, which is Int"),
        "net module must be prepended when `Message` is referenced"
    );
}

#[test]
fn net_message_compiles_end_to_end() {
    // Compiled mode needs `struct Message` — proof the type is genuinely imported.
    let src = "## Main\n\
        \x20   Let m be a new Message with sender 7 and payload \"hi\".\n\
        \x20   Show m's sender.\n";
    let result = run_logos(src);
    assert!(
        result.success,
        "net `Message` must compile invisibly.\nstderr: {}",
        result.stderr
    );
    assert!(result.stdout.contains('7'), "expected 7: {}", result.stdout);
}

#[test]
fn crdt_delta_autoimports() {
    let src = "## Main\n\
        \x20   Let d be a new Delta with replica 2 and amount 5.\n\
        \x20   Show d's amount.\n";
    let applied = apply_prelude(src);
    assert!(
        applied.contains("a replica, which is Int"),
        "crdt module must be prepended when `Delta` is referenced"
    );
}

#[test]
fn io_severity_autoimports_on_type_reference() {
    // `Severity` named in a parameter type must pull the io module's enum definition.
    let src = "## To rank (s: Severity) -> Int:\n\
        \x20   Return 1.\n\
        ## Main\n\
        \x20   Show 1.\n";
    let applied = apply_prelude(src);
    assert!(
        applied.contains("A Severity is either"),
        "io module must be prepended when `Severity` is referenced as a type"
    );
}

// ─── Native modules auto-import their declarations ───────────────────────────

#[test]
fn env_args_autoimports() {
    let src = "## Main\n    Let xs be args().\n    Show 1.\n";
    assert!(
        apply_prelude(src).contains("## To native args"),
        "env module must be prepended when `args` is called"
    );
}

#[test]
fn time_now_autoimports() {
    let src = "## Main\n    Let t be now().\n    Show 1.\n";
    assert!(
        apply_prelude(src).contains("## To native now"),
        "time module must be prepended when `now` is called"
    );
}

#[test]
fn random_randomint_autoimports() {
    let src = "## Main\n    Let r be randomInt(1, 6).\n    Show 1.\n";
    assert!(
        apply_prelude(src).contains("## To native randomInt"),
        "random module must be prepended when `randomInt` is called"
    );
}

#[test]
fn file_read_autoimports() {
    let src = "## Main\n    Let r be read(\"x.txt\").\n    Show 1.\n";
    assert!(
        apply_prelude(src).contains("## To native read"),
        "file module must be prepended when `read` is called"
    );
}

// ─── Collision safety: the program's own definition wins ─────────────────────

#[test]
fn user_message_type_shadows_net_module() {
    let src = "## Definition\n\
        A Message has:\n\
        \x20   a kind, which is Int.\n\
        ## Main\n\
        \x20   Let m be a new Message with kind 9.\n\
        \x20   Show m's kind.\n";
    let applied = apply_prelude(src);
    assert!(
        !applied.contains("a sender, which is Int"),
        "net module must NOT be prepended when the program defines its own `Message`"
    );
    let result = run_interpreter(src);
    assert!(
        result.success,
        "user's Message must compile: {}",
        result.error
    );
    assert_eq!(result.output.trim(), "9");
}

#[test]
fn user_native_args_decl_is_not_duplicated() {
    // The benchmark corpus declares `## To native args` itself. Referencing `args` must
    // not pull env (which also declares it) — the source stays byte-identical.
    let src = "## To native args -> Seq of Text\n\
        ## Main\n\
        \x20   Let xs be args().\n\
        \x20   Show 1.\n";
    assert!(
        matches!(apply_prelude(src), std::borrow::Cow::Borrowed(_)),
        "a program that declares `args` itself must be returned untouched"
    );
}

// ─── Byte-identity: nothing prepended when no stdlib vocabulary is used ───────

#[test]
fn plain_program_untouched() {
    let src = "## Main\n    Let x be 1 + 2.\n    Show x.\n";
    assert!(
        matches!(apply_prelude(src), std::borrow::Cow::Borrowed(_)),
        "a non-stdlib program must pass through untouched (AOT hot-path contract)"
    );
}

#[test]
fn no_prelude_opts_out_for_net() {
    let src = "## NoPrelude\n\
        ## Main\n\
        \x20   Let m be a new Message with sender 1 and payload \"x\".\n\
        \x20   Show m's sender.\n";
    let applied = apply_prelude(src);
    assert!(
        !applied.contains("a sender, which is Int"),
        "## NoPrelude must suppress the net auto-import"
    );
}

// ─── Leading title/comment headers survive the auto-import splice ────────────

/// A Studio-style program header (`# Title` + `--` notes BEFORE the first
/// `## Main`) must not break when a stdlib module auto-imports. The splice has
/// to land the module code AFTER the leading prose — prepending it pushes the
/// header into the middle of the assembled program, where the parser rejects
/// the `--` lines ("I expected a statement here").
#[test]
fn leading_comment_header_survives_autoimport() {
    let src = "# Stable IDs\n-- A perfectly ordinary comment before the first section.\n\n## Main\n\nLet dns be uuidParse(\"6ba7b810-9dad-11d1-80b4-00c04fd430c8\").\nShow \"parsed: {dns}\".\n";
    let spliced = apply_prelude(src);
    assert!(
        spliced.contains("uuidParse"),
        "uuid module must auto-import for the uuidParse call"
    );
    let agreed = run_all_tiers(src);
    assert!(
        agreed.error.is_none(),
        "leading-comment program must run after auto-import on every tier.\nerror: {}",
        agreed.error.as_deref().unwrap_or("")
    );
    assert!(
        agreed
            .output
            .contains("parsed: 6ba7b810-9dad-11d1-80b4-00c04fd430c8"),
        "wrong output: {}",
        agreed.output
    );
}
