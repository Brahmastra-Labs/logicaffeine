//! LOCK — `ui_bridge::with_parsed_program_checked` undoes the prelude splice on a parse-time
//! refusal's span EXACTLY ONCE, for every backend that goes through it.
//!
//! `parse_seam_e` splices the stdlib modules a program demands ahead of the author's own text
//! (`loader::apply_prelude_mapped`), so every span the parser computes is a span of the EXPANDED
//! text, not the file the author wrote. `with_parsed_program_checked` used to undo that splice
//! BOTH before handing the parsed `Result` to its caller's closure AND again, unconditionally, on
//! whatever the closure returned — so a caller whose closure simply forwards the parse error (the
//! shape every real caller uses: `let (stmts, ..) = parsed?;`) had its span translated twice. The
//! first translation is correct; the second reinterprets the now-small, already-correct offset as
//! if it still needed un-splicing, and the spliced-in stdlib text is typically the largest run in
//! the expanded text, so almost any real file offset falls inside it on the second pass — mapping
//! to "added text" and silently becoming `Span::default()` (line 1).
//!
//! One fixture, one shared parse-time mistake (an `## Session` field a cookie cannot hold, on line
//! 4), driven through every public entry point built on `with_parsed_program_checked` — the parse
//! fails identically for all of them, before any backend-specific codegen ever runs, so the same
//! span-survival property must hold for every one of them.

fn fixture() -> String {
    "# App\n\n## Session\n    a cart, which is Seq of Int.\n\n## Main\nShow upper(\"ok\").\n"
        .to_string()
}

/// The 1-based `(line, column)` of byte `at` in `src`.
fn line_col(src: &str, at: usize) -> (usize, usize) {
    let before = &src[..at.min(src.len())];
    (
        before.matches('\n').count() + 1,
        before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1,
    )
}

/// Every entry point must locate the mistake at exactly this line, column and covered text.
const WANT_LINE_COL: (usize, usize) = (4, 7);
const WANT_TEXT: &str = "cart";

fn check(label: &str, src: &str, span: logicaffeine_language::token::Span) {
    let got_text = src.get(span.start..span.end);
    assert_eq!(
        got_text,
        Some(WANT_TEXT),
        "{label}: span does not cover {WANT_TEXT:?} — {span:?}"
    );
    let lc = line_col(src, span.start);
    assert_eq!(
        lc, WANT_LINE_COL,
        "{label}: reported at {lc:?}, not {WANT_LINE_COL:?}"
    );
}

#[test]
fn compile_to_isa_locates_the_mistake() {
    let src = fixture();
    let e = logicaffeine_compile::compile::compile_to_isa(&src).expect_err("must refuse");
    check("compile_to_isa", &src, e.span);
}

#[test]
fn compile_to_program_locates_the_mistake() {
    let src = fixture();
    let e = logicaffeine_compile::compile::compile_to_program(&src).expect_err("must refuse");
    check("compile_to_program", &src, e.span);
}

#[test]
fn compile_to_riscv_locates_the_mistake() {
    let src = fixture();
    let e = logicaffeine_compile::compile::compile_to_riscv(&src).expect_err("must refuse");
    check("compile_to_riscv", &src, e.span);
}

#[test]
fn compile_to_wasm_locates_the_mistake() {
    let src = fixture();
    let e = logicaffeine_compile::compile::compile_to_wasm(&src).expect_err("must refuse");
    check("compile_to_wasm", &src, e.span);
}

// `compile_to_wasm_observed` and `compile_to_wasm_linked` are NOT part of this census: both build
// on `with_parsed_program` (the message-only seam, via `parse_seam`'s own `socratic_explanation`
// conversion), not `with_parsed_program_checked` — their `custom()` helper stuffs
// `Span::default()` unconditionally into every error they raise, by design, independent of
// today's fix. A real location for those two is a separate gap (switching them onto
// `with_parsed_program_checked`), not addressed here.

// The remaining four entry points live behind `logicaffeine-compile`'s OWN `codegen` feature — on
// by default for that crate, so no `#[cfg(..)]` is needed here (a `feature = "codegen"` guard on
// this crate's OWN, unrelated Cargo.toml would just never activate and silently skip these).
#[test]
fn compile_to_tenstorrent_locates_the_mistake() {
    let src = fixture();
    let e = logicaffeine_compile::compile::compile_to_tenstorrent(&src).expect_err("must refuse");
    check("compile_to_tenstorrent", &src, e.span);
}

#[test]
fn compile_to_analog_locates_the_mistake() {
    let src = fixture();
    let e = logicaffeine_compile::compile::compile_to_analog(&src).expect_err("must refuse");
    check("compile_to_analog", &src, e.span);
}

#[test]
fn compile_to_plasticine_locates_the_mistake() {
    let src = fixture();
    let e = logicaffeine_compile::compile::compile_to_plasticine(&src).expect_err("must refuse");
    check("compile_to_plasticine", &src, e.span);
}

// `compile_to_cuda` is NOT in this census: it calls `compile_to_c(source)?` BEFORE its own
// `with_parsed_program_checked`, and `compile_to_c` has an entirely separate, hand-rolled parse
// pipeline (its own `Lexer`/`DiscoveryPass`/`Parser`, with its own prelude-splice handling) that
// never goes through `ui_bridge` at all — so a parse failure on this fixture is caught there
// first, with whatever span THAT pipeline computes, never reaching the code this lock covers.
// (Confirmed directly: `compile_to_cuda`'s error on the shared fixture is a real but WRONG span
// unrelated to today's fix — a separate, pre-existing gap in `compile_to_c`'s own pipeline, out
// of scope here. `compile_to_tenstorrent`/`compile_to_analog` call `compile_to_c` only AFTER their
// own `with_parsed_program_checked` succeeds, so a parse failure never reaches it — both verified
// GREEN above.)
