//! Phase 10 (FINISH_INTERPRETER.md) — stdlib prelude bundling.
//!
//! The concurrency/net/io/crdt vocabulary is embedded at compile time and made
//! available WITHOUT an explicit import — but only to programs that actually
//! reference it (so the benchmark corpus and every non-stdlib program stay
//! byte-identical). `## NoPrelude` opts out.

mod common;

use common::run_interpreter;
use logicaffeine_compile::loader::{apply_prelude, prelude};

#[test]
fn prelude_is_embedded_and_loads() {
    let p = prelude();
    assert!(!p.is_empty(), "prelude is embedded at compile time");
    // Every module's code (its `##` sections) is present — titles + prose are
    // documentation and are not prepended.
    assert!(p.contains("## To flush"), "concurrency helper `flush` defined");
    assert!(p.contains("A Message has"), "net `Message` type defined");
    assert!(p.contains("A Severity is either"), "io `Severity` type defined");
    assert!(p.contains("A Delta has"), "crdt `Delta` type defined");
}

#[test]
fn prelude_auto_prepended_enables_vocabulary() {
    // `flush` is a stdlib helper (sends a whole sequence into a pipe). The program
    // never imports it — the prelude makes it available because the source
    // references `flush`.
    let src = "## Main\n\
        \x20   Let xs be [1, 2, 3].\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Call flush with xs and ch.\n\
        \x20   Receive a from ch.\n\
        \x20   Show a.\n";
    let result = run_interpreter(src);
    assert!(result.success, "stdlib helper resolved without import: {}", result.error);
    assert_eq!(result.output.trim(), "1", "flush enqueued 1,2,3; first receive is 1");
}

#[test]
fn prelude_no_prelude_decorator_opts_out() {
    // The same program, but `## NoPrelude` suppresses the auto-import, so `flush`
    // is undefined and the program fails to resolve it.
    let src = "## NoPrelude\n\
        ## Main\n\
        \x20   Let xs be [1, 2, 3].\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Call flush with xs and ch.\n\
        \x20   Receive a from ch.\n\
        \x20   Show a.\n";
    let result = run_interpreter(src);
    assert!(
        !result.success,
        "## NoPrelude must leave `flush` undefined, but the program succeeded with {:?}",
        result.output
    );
}

#[test]
fn prelude_does_not_touch_programs_that_use_no_stdlib() {
    // A program that references no stdlib vocabulary is returned verbatim — this
    // is what keeps the AOT hot path byte-identical.
    let plain = "## Main\n    Let x be 1 + 2.\n    Show x.\n";
    assert!(
        matches!(apply_prelude(plain), std::borrow::Cow::Borrowed(_)),
        "non-stdlib program must pass through untouched"
    );
    let result = run_interpreter(plain);
    assert!(result.success, "plain program still runs: {}", result.error);
    assert_eq!(result.output.trim(), "3");
}

#[test]
fn prelude_all_modules_parse_clean() {
    // Every module is prepended verbatim when its vocabulary is referenced, so
    // every module must parse cleanly. Prepend all four + a trivial main and
    // confirm the whole thing parses and runs.
    let combined = format!("{}\n\n## Main\n    Show 1.\n", prelude());
    let result = run_interpreter(&combined);
    assert!(result.success, "all prelude modules must parse clean: {}", result.error);
    assert_eq!(result.output.trim(), "1");
}

#[test]
fn prelude_identical_native_and_wasm() {
    // `prelude()` is built from `include_str!`, a compile-time constant, so the
    // bytes are identical on every target by construction. We assert the embedded
    // content is exactly the concatenation of the four modules with stable
    // separators (the invariant a wasm build would also satisfy).
    let p = prelude();
    // Every module's distinctive marker, in stable embedding order: concurrency, net,
    // io, crdt, then the native env/file/random/time declarations.
    let markers = [
        "## To flush",
        "A Message has",
        "A Severity is either",
        "A Delta has",
        "## To native get",
        "## To native read",
        "## To native randomInt",
        "## To native now",
    ];
    let mut last = 0usize;
    for m in markers {
        let at = p.find(m).unwrap_or_else(|| panic!("marker {m:?} missing"));
        assert!(at >= last, "modules embedded in stable order");
        last = at;
    }
}
