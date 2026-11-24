//! LOCK — friction points hit building the Learn Logos course site, a real multi-page LOGOS web app,
//! each a finding under its own heading below. Every fixture is the smallest page or program that
//! reproduces its finding, reduced from the site.
//!
//! Each finding is held twice where it can be: as the PAGE it was reported from, which must build
//! for the browser, load in a real engine and mount, and as a plain PROGRAM that runs on the
//! tree-walker and on WebAssembly and must print the same answer on both, so the lock is on the
//! answer and not only on the build succeeding.
//!
//! The record and prop names that fail (`Span`, `Check`) are held by `type_name_shadowing_lock`.

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

// ─── F24 — a field named `tally` ────────────────────────────────────────────────────────────────
//
// `tally` is also the Tally CRDT type's keyword. A record declared a field called `tally` and a
// constructor filled it without complaint, then `k's tally` could not be read: the browser build
// refused "field access with no static slot", and `count`, `size` or `total` in the same place
// built. A word after a possessive is a field name, whatever else the word can mean.

const TALLY_PAGE: &str = "## A Kind has:\n    A label: Text.\n    A tally: Int.\n\n\
## To all -> Seq of Kind:\n    Return [(a new Kind with label \"Core\" and tally 2)].\n\n\
## Component KindView (k: Kind)\n\nView:\n    <h4>{k's label} <span>{k's tally} crates</span></h4>\n\n\
## Page /\n\nView:\n    <main>{for each k in all():}<KindView k={k}/>{end}</main>\n";

#[test]
fn f24_a_page_reading_a_field_called_tally_reaches_the_browser() {
    the_page_reaches_the_browser(&TALLY_PAGE.replace("tally", "total"));
    the_page_reaches_the_browser(TALLY_PAGE);
}

#[test]
fn f24_a_field_called_tally_is_read_set_and_passed_on_every_tier() {
    agree(
        "## A Kind has:\n    A label: Text.\n    A tally: Int.\n\n\
         ## To count (k: Kind) -> Int:\n    Return k's tally.\n\n\
         ## Main\n\nLet mutable k be a new Kind with label \"Core\" and tally 2.\n\
         Show k's tally.\nSet k's tally to k's tally + 1.\nShow count(k).\n",
        "2\n3",
    );
}

/// The other CRDT type keywords are type names in a type position and field names after a
/// possessive, exactly as `tally` is.
#[test]
fn f24_every_crdt_keyword_is_a_field_name_after_a_possessive() {
    for field in ["tally", "divergent", "shared"] {
        agree(
            &format!(
                "## A Kind has:\n    A {field}: Int.\n\n\
                 ## Main\n\nLet k be a new Kind with {field} 7.\nShow k's {field}.\n"
            ),
            "7",
        );
    }
}

// ─── F26 — `some x and <next field>` ────────────────────────────────────────────────────────────
//
// `some` took a whole condition as its operand, so in a constructor the `and` that separates the
// next field was read as a logical `and` inside the Option: `predict some (…) and title "One"`
// swallowed `title` and the build failed with "I expected a statement here". `none` in the same
// place, and `some` as the last field, both worked. `some` takes the one value after it.

const SOME_THEN_A_FIELD_PAGE: &str = "## A Question has:\n    A prompt: Text.\n\n\
## A Lesson has:\n    A predict: Option of Question.\n    A title: Text.\n\n\
## To probe -> Lesson:\n    Return a new Lesson with predict some (a new Question with prompt \"why?\") and title \"One\".\n\n\
## Page /\n\nView:\n    <main><p>{probe()'s title}</p></main>\n";

#[test]
fn f26_a_page_with_some_before_another_field_reaches_the_browser() {
    the_page_reaches_the_browser(SOME_THEN_A_FIELD_PAGE);
}

#[test]
fn f26_some_before_another_field_fills_both_fields() {
    agree(
        "## A Question has:\n    A prompt: Text.\n\n\
         ## A Lesson has:\n    A predict: Option of Question.\n    A title: Text.\n\n\
         ## To ask (l: Lesson) -> Text:\n    Inspect l's predict:\n        When Some (q):\n            Return q's prompt.\n        When None:\n            Return \"-\".\n\n\
         ## Main\n\n\
         Let a be a new Lesson with predict some (a new Question with prompt \"why?\") and title \"One\".\n\
         Show a's title.\nShow ask(a).\n\
         Let b be a new Lesson with predict none and title \"Two\".\n\
         Show b's title.\nShow ask(b).\n",
        "One\nwhy?\nTwo\n-",
    );
}

#[test]
fn f26_some_of_a_scalar_before_another_field_fills_both_fields() {
    agree(
        "## A Box has:\n    A size: Option of Int.\n    A name: Text.\n\n\
         ## Main\n\n\
         Let b be a new Box with size some 3 and name \"crate\".\n\
         Show b's name.\n\
         Inspect b's size:\n    When Some (n):\n        Show n.\n    When None:\n        Show \"-\".\n",
        "crate\n3",
    );
}

/// `some` still takes a whole arithmetic value: only `and` and `or`, the words that join clauses
/// and fields, end it.
#[test]
fn f26_some_takes_a_whole_arithmetic_value() {
    agree(
        "## Main\n\nLet x be some 1 + 2 * 3.\n\
         Inspect x:\n    When Some (n):\n        Show n.\n    When None:\n        Show \"-\".\n",
        "7",
    );
}

// ─── F23 — a variant payload typed by the plural of another record ──────────────────────────────
//
// A record `Kinds` beside a record `Kind`, and a sum type whose variant carries a `Kinds`: the
// browser build refused "call argument type does not match the parameter" from the component that
// inspects the sum type. Renamed `KindList`, the same page built. A program's declared name is
// its type, never the plural of another one.

const PLURAL_PAYLOAD_PAGE: &str = "## A Kind has:\n    A label: Text.\n\n\
## A Kinds has:\n    A kinds: Seq of Kind.\n\n\
## A Link has:\n    A text: Text.\n\n\
## A Widget is one of:\n    A KindsWidget with kinds Kinds.\n    A LinkWidget with link Link.\n\n\
## To all -> Seq of Widget:\n    Return [(a new KindsWidget with kinds (a new Kinds with kinds [(a new Kind with label \"Core\"), (a new Kind with label \"Apps\")])), (a new LinkWidget with link (a new Link with text \"docs\"))].\n\n\
## Component KindsView (k: Kinds)\n\nView:\n    <ul>{for each kd in k's kinds:}<li>{kd's label}</li>{end}</ul>\n\n\
## Component WidgetView (w: Widget)\n\nView:\n    {inspect w:}{when KindsWidget (k):}<KindsView k={k}/>{when LinkWidget (l):}<p>{l's text}</p>{end}\n\n\
## Page /\n\nView:\n    <main>{for each w in all():}<WidgetView w={w}/>{end}</main>\n";

#[test]
fn f23_a_page_whose_variant_carries_a_plural_named_record_reaches_the_browser() {
    the_page_reaches_the_browser(&PLURAL_PAYLOAD_PAGE.replace("Kinds", "KindList"));
    the_page_reaches_the_browser(PLURAL_PAYLOAD_PAGE);
}

/// The narrowest form of the same finding: a component prop typed by the plural-named record.
/// The lexer reads a capitalized plural followed by a verb as a lowercased common noun, and the
/// word after `(k: Kinds)` is the `View` that opens the section, so the prop was typed `kinds`.
#[test]
fn f23_a_component_prop_typed_by_a_plural_named_record_reaches_the_browser() {
    let page = "## A Kind has:\n    A label: Text.\n\n\
## A Kinds has:\n    A kinds: Seq of Kind.\n\n\
## Component KindsView (k: Kinds)\n\nView:\n    <ul>{for each kd in k's kinds:}<li>{kd's label}</li>{end}</ul>\n\n\
## Page /\n\nState:\n    Let ks be a new Kinds with kinds [(a new Kind with label \"Core\")].\n\nView:\n    <main><KindsView k={ks}/></main>\n";
    the_page_reaches_the_browser(&page.replace("Kinds", "KindList"));
    the_page_reaches_the_browser(page);
}

#[test]
fn f23_a_variant_carrying_a_plural_named_record_is_built_and_read_on_every_tier() {
    agree(
        "## A Kind has:\n    A label: Text.\n\n\
         ## A Kinds has:\n    A kinds: Seq of Kind.\n\n\
         ## A Widget is one of:\n    A KindsWidget with kinds Kinds.\n    A Blank.\n\n\
         ## To labels (k: Kinds) -> Text:\n    Let mutable out be \"\".\n    Repeat for kd in k's kinds:\n        Set out to out + kd's label + \";\".\n    Return out.\n\n\
         ## To draw (w: Widget) -> Text:\n    Inspect w:\n        When KindsWidget (k):\n            Return labels(k).\n        When Blank:\n            Return \"-\".\n\n\
         ## Main\n\n\
         Let w be a new KindsWidget with kinds (a new Kinds with kinds [(a new Kind with label \"Core\"), (a new Kind with label \"Apps\")]).\n\
         Show draw(w).\nShow draw(a new Blank).\n",
        "Core;Apps;\n-",
    );
}

// ─── F27 — an interpolated attribute is text ───────────────────────────────────────────────────
//
// `<RailLink n="{s's n}"/>` hands a Text prop an attribute that interpolates an Int. The browser
// build refused "call argument type does not match the parameter"; interpolating a Text field, or
// a literal, built. A quoted attribute is text whatever it interpolates, the way a text hole is.

const INTERPOLATED_PROP_PAGE: &str = "## A Stop has:\n    An id: Text.\n    A n: Int.\n\n\
## To outline -> Seq of Stop:\n    Return [(a new Stop with id \"why\" and n 0)].\n\n\
## Component RailLink (n: Text)\n\nView:\n    <li><span class=\"n\">{n}</span></li>\n\n\
## Page /\n\nView:\n    <main><ol>{for each s in outline():}<RailLink n=\"{s's n}\"/>{end}</ol></main>\n";

#[test]
fn f27_a_text_prop_given_an_interpolated_int_reaches_the_browser() {
    the_page_reaches_the_browser(&INTERPOLATED_PROP_PAGE.replace("{s's n}", "{s's id}"));
    the_page_reaches_the_browser(INTERPOLATED_PROP_PAGE);
}

#[test]
fn f27_a_text_prop_given_interpolated_text_around_an_int_reaches_the_browser() {
    the_page_reaches_the_browser(&INTERPOLATED_PROP_PAGE.replace("{s's n}", "A.{s's n}"));
}

#[test]
fn f27_every_scalar_interpolated_into_a_text_prop_reaches_the_browser() {
    for (ty, value) in [("Int", "0"), ("Real", "1.5"), ("Bool", "true")] {
        let src = INTERPOLATED_PROP_PAGE
            .replace("A n: Int.", &format!("A n: {ty}."))
            .replace("and n 0", &format!("and n {value}"));
        the_page_reaches_the_browser(&src);
    }
}
