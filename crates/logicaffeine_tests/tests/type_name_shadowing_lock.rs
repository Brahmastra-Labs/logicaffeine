//! ★ LOCK — a program's own type is ITS type, whatever it is called. ★
//!
//! Two ways a record's name stopped meaning the record, both found through a web page and both
//! older than the web framework:
//!
//! - **A name the language also uses for a builtin.** `Span`, `Money`, `Time` and the rest name
//!   first-class value types, and the bytecode compiler resolved a type name against that list
//!   BEFORE the program's own declarations. So `## A Span has: …` declared a record, the tree-walker
//!   used the record, and every compiled tier was told `(s: Span)` was the builtin scalar: no
//!   fields to read, no variants to match. A page refused with "field access with no static slot",
//!   and an `{inspect}` over a sum type called `Span` was refused as "an `Option` of a scalar", a
//!   type that appears nowhere in the program. The program's declaration must win, the way it does
//!   on the reference tier: a name in scope is the one the author wrote.
//! - **A name that is also a keyword.** `check` begins `Check that …`, and the type-discovery pass
//!   accepted a keyword in no name position, so `## A Check has: …` registered nothing at all. The
//!   parser has long let a keyword name a variable or a field ("identifier freedom"); a TYPE is the
//!   same kind of name and gets the same freedom (`type_name_freedom_lock` holds the discovery pass
//!   to it directly).
//!
//! Every program case runs on the tree-walker and on WebAssembly and requires the same output, so
//! the lock is on the answer rather than on how either tier chose to get there. The page cases are
//! the pages the findings were reported from: each must build for the browser, load in a real
//! engine and mount.

#![cfg(feature = "wasm-jit")]

#[path = "compile_common/mod.rs"]
mod compile_common;

use compile_common::wasm_host;

fn tw(src: &str) -> String {
    let out = logicaffeine_compile::compile::tw_outcome(src);
    assert_eq!(out.error, None, "the reference tier runs it:\n{src}");
    out.output
}

fn wasm(src: &str) -> String {
    wasm_host::run_show(src)
        .unwrap_or_else(|why| panic!("the browser tier runs it: {why}\n{src}"))
}

fn agree(src: &str, expected: &str) {
    assert_eq!(tw(src), expected, "the reference tier's answer:\n{src}");
    assert_eq!(wasm(src), expected, "WebAssembly gives the same answer:\n{src}");
}

/// A page builds for the browser, its module loads in a real engine, and its mount does not trap.
fn the_page_reaches_the_browser(src: &str) {
    if let Err(why) = logicaffeine_compile::compile::compile_to_wasm(src) {
        panic!("the page must build for the browser: {why:?}\n{src}");
    }
    wasm_host::mount_interactive(src);
}

/// Every builtin value type the bytecode compiler knows by name.
const BUILTIN_NAMES: &[&str] = &[
    "Span", "Duration", "Time", "Rational", "Complex", "Decimal", "Modular", "Money", "Quantity",
    "Uuid", "Date", "Moment",
];

#[test]
fn a_record_named_like_a_builtin_is_the_programs_record() {
    for name in BUILTIN_NAMES {
        let src = format!(
            "## A {name} has:\n    A kind: Text.\n    A size: Int.\n\n\
             ## To describe (x: {name}) -> Text:\n    Return x's kind + \" \" + \"{{x's size}}\".\n\n\
             ## Main\n\nShow describe(a new {name} with kind \"k\" and size 3).\n"
        );
        agree(&src, "k 3");
    }
}

#[test]
fn a_sum_type_named_like_a_builtin_matches_its_variants() {
    for name in BUILTIN_NAMES {
        let src = format!(
            "## A {name} is one of:\n    A Plain with text Text.\n    A Code with text Text.\n\n\
             ## To render (x: {name}) -> Text:\n    Inspect x:\n        When Plain (t):\n            Return t.\n        When Code (t):\n            Return \"<\" + t + \">\".\n\n\
             ## Main\n\nShow render(a new Plain with text \"a\").\nShow render(a new Code with text \"b\").\n"
        );
        agree(&src, "a\n<b>");
    }
}

#[test]
fn a_list_of_records_named_like_a_builtin_reads_its_fields() {
    let src = "## A Span has:\n    A text: Text.\n\n\
        ## To all -> Seq of Span:\n    Return [(a new Span with text \"x\"), (a new Span with text \"y\")].\n\n\
        ## Main\n\nRepeat for s in all():\n    Show s's text.\n";
    agree(src, "x\ny");
}

/// A record named like a builtin, returned from a function and read field by field.
#[test]
fn a_record_named_like_a_builtin_comes_back_from_a_function() {
    for name in BUILTIN_NAMES {
        let src = format!(
            "## A {name} has:\n    A kind: Text.\n\n\
             ## To make -> {name}:\n    Return a new {name} with kind \"made\".\n\n\
             ## Main\n\nShow make()'s kind.\n"
        );
        agree(&src, "made");
    }
}

/// Keywords a program could reasonably name a type after. Each leads a statement somewhere in the
/// language, and each is an ordinary noun.
const KEYWORD_NAMES: &[&str] = &[
    "Check", "Note", "Record", "Task", "Pipe", "Given", "Expect", "Trust", "Inspect", "Stop",
];

#[test]
fn a_record_named_by_a_keyword_is_registered_and_read() {
    for name in KEYWORD_NAMES {
        let src = format!(
            "## A {name} has:\n    A prompt: Text.\n\n\
             ## To ask (c: {name}) -> Text:\n    Return c's prompt.\n\n\
             ## Main\n\nShow ask(a new {name} with prompt \"why\").\n"
        );
        agree(&src, "why");
    }
}

#[test]
fn a_keyword_named_record_inside_an_option_field_reads_through_the_option() {
    let src = "## A Check has:\n    A prompt: Text.\n\n\
        ## A Lesson has:\n    A title: Text.\n    A predict: Option of Check.\n\n\
        ## To ask (l: Lesson) -> Text:\n    Inspect l's predict:\n        When Some (c):\n            Return c's prompt.\n        When None:\n            Return \"-\".\n\n\
        ## Main\n\nShow ask(a new Lesson with title \"t\" and predict (some (a new Check with prompt \"why\"))).\n\
        Show ask(a new Lesson with title \"u\" and predict none).\n";
    agree(src, "why\n-");
}

/// F3, the page it was reported from: a glossary term drawn from a record called `Span`, its
/// fields read into an attribute, a prop and a text hole.
const GLOSSARY_PAGE: &str = "## A Span has:\n    A kind: Text.\n    A text: Text.\n    A means: Text.\n\n\
## A Para has:\n    A spans: Seq of Span.\n\n\
## Component SpanView (span: Span)\n\nView:\n    <span class={span's kind} title={span's means}>{span's text}</span>\n\n\
## Component ParaView (para: Para)\n\nView:\n    <p>{for each s in para's spans:}<SpanView span={s}/>{end}</p>\n\n\
## To idea -> Para:\n    Return a new Para with spans [(a new Span with kind \"plain\" and text \"call \" and means \"\"), (a new Span with kind \"term\" and text \"operation\" and means \"A named, typed function.\")].\n\n\
## Page /\n\nState:\n    Let p be idea().\n\nView:\n    <main><ParaView para={p}/></main>\n";

#[test]
fn a_page_drawing_a_record_called_span_reaches_the_browser() {
    the_page_reaches_the_browser(&GLOSSARY_PAGE.replace("Span", "Piece"));
    the_page_reaches_the_browser(GLOSSARY_PAGE);
}

/// The same page with the record called `Check`, the keyword case of the same finding.
#[test]
fn a_page_drawing_a_record_called_check_reaches_the_browser() {
    the_page_reaches_the_browser(
        &GLOSSARY_PAGE
            .replace("SpanView", "PieceView")
            .replace("Span", "Check")
            .replace("(span: Check)", "(piece: Check)")
            .replace("span's", "piece's")
            .replace("span={s}", "piece={s}"),
    );
}

/// F4, the page it was reported from: an `{inspect}` in a view over a sum type called `Span`.
const WIDGET_PAGE: &str = "## A Span is one of:\n    A Plain with text Text.\n    A Code with text Text.\n\n\
## Component SpanView (span: Span)\n\nView:\n    {inspect span:}{when Plain (t):}<span>{t}</span>{when Code (t):}<code>{t}</code>{end}\n\n\
## To spans -> Seq of Span:\n    Return [(a new Plain with text \"call \"), (a new Code with text \"math.add\")].\n\n\
## Page /\n\nState:\n    Let ss be spans().\n\nView:\n    <main>{for each s in ss:}<SpanView span={s}/>{end}</main>\n";

#[test]
fn an_inspect_in_a_view_over_a_sum_type_called_span_reaches_the_browser() {
    the_page_reaches_the_browser(&WIDGET_PAGE.replace("Span", "Piece"));
    the_page_reaches_the_browser(WIDGET_PAGE);
}

/// A refusal of an `{inspect}` must never describe an `Option` the program does not have: when the
/// scrutinee is a sum type of the program's own, any refusal names that type.
#[test]
fn an_inspect_refusal_never_names_an_option_the_program_does_not_have() {
    for name in BUILTIN_NAMES {
        let src = WIDGET_PAGE.replace("Span", name);
        if let Err(why) = logicaffeine_compile::compile::compile_to_wasm(&src) {
            let why = format!("{why:?}");
            assert!(
                !why.contains("Option"),
                "a sum type called `{name}` was refused as an Option: {why}"
            );
        }
    }
}
