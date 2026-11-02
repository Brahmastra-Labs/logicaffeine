//! ★ LOCK — every value renders as the same Text on wasm as on the reference tiers. ★
//!
//! An interpolation, a `+` onto Text, `format(x)` and `Fail with x` all turn a value into the Text it
//! shows as, and a page's journal renders every changed field that way on every turn. The wasm module
//! renders the scalars inline and every other value SHAPE through one generated display function per
//! shape (`vm/wasm/module/display.rs`), recursing through the parts — so a list of records whose field
//! is a list of Text renders like the VM's `RuntimeValue::to_display_string`, at any depth.
//!
//! Every row runs on the tree-walker, the VM and wasm and compares whole outcomes. The census holds the
//! rows to the display module's own list of shapes: a shape the module gains without a row here fails.

#![cfg(feature = "wasm-jit")]

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::run_outcome;

const TYPES: &str = "\
## A Tag has:
    a name, which is Text.

## A Note has:
    a title, which is Text.
    a tags, which is Seq of Text.
    a score, which is Float.
    a pinned, which is Bool.

## A Pair has:
    a left, which is Int.
    a right, which is Note.

## A Shape is one of:
    A Dot.
    A Circle with radius Int.
    A Label with text Text and size Int.

";

/// `(shapes covered, program body)` — each body interpolates, concatenates and formats its values.
fn rows() -> Vec<(&'static [&'static str], &'static str)> {
    vec![
        (&["Seq", "Text"], "Let xs be a new Seq of Text.\nShow \"[{xs}]\".\nPush \"a\" to xs.\nPush \"b c\" to xs.\nShow \"{xs}\".\nShow \"xs=\" + xs.\nShow format(xs)."),
        (&["Seq", "Float"], "Let fs be a new Seq of Float.\nPush 1.5 to fs.\nPush 2.0 to fs.\nShow \"{fs}\"."),
        (&["Seq", "Int", "Bool"], "Let ns be [1, 2, 3].\nLet bs be [true, false].\nShow \"{ns} {bs}\"."),
        (&["Set", "Text"], "Let s be a new Set of Text.\nAdd \"x\" to s.\nAdd \"y\" to s.\nAdd \"x\" to s.\nShow \"{s}\"."),
        (&["Set", "Int"], "Let s be a new Set of Int.\nAdd 3 to s.\nAdd 1 to s.\nShow \"{s}\"."),
        (&["Map", "Text", "Int"], "Let m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\nSet item \"b\" of m to 2.\nShow \"{m}\"."),
        (&["Map", "Int", "Text"], "Let m be a new Map of Int to Text.\nSet item 7 of m to \"seven\".\nShow \"m: \" + m."),
        (&["Map", "Seq"], "Let m be a new Map of Text to (Seq of Int).\nSet item \"k\" of m to [4, 5].\nShow \"{m}\"."),
        (&["Record", "Seq", "Text", "Float", "Bool"], "Let n be a new Note with title \"hi\" and tags [\"a\", \"b\"] and score 2.5 and pinned true.\nShow \"{n}\".\nShow \"n=\" + n."),
        (&["Record"], "Let t be a new Tag with name \"x\".\nShow \"{t}\"."),
        (&["Record", "Int"], "Let p be a new Pair with left 1 and right (a new Note with title \"t\" and tags [] and score 0.0 and pinned false).\nShow \"{p}\"."),
        (&["Seq", "Record"], "Let ns be a new Seq of Note.\nPush a new Note with title \"one\" and tags [\"x\"] and score 1.0 and pinned false to ns.\nShow \"{ns}\"."),
        (&["Variant"], "Let a be a new Dot.\nLet b be a new Circle with radius 3.\nLet c be a new Label with text \"hi\" and size 2.\nShow \"{a} {b} {c}\"."),
        (&["Seq", "Variant"], "Let shapes be a new Seq of Shape.\nPush a new Dot to shapes.\nPush a new Circle with radius 4 to shapes.\nShow \"{shapes}\"."),
        (&["Seq"], "Let grid be a new Seq of (Seq of Text).\nPush [\"a\", \"b\"] to grid.\nPush [] of Text to grid.\nShow \"{grid}\"."),
        (&["Tuple"], "Let t be (1, \"two\", true).\nShow \"{t}\"."),
        (&["Seq", "Record"], "## To render(ns: Seq of Note) -> Text:\n    Return \"{ns}\".\n\n## Main\nLet ns be a new Seq of Note.\nPush a new Note with title \"p\" and tags [] and score 0.5 and pinned true to ns.\nShow render(ns)."),
        (&["Map", "Record"], "Let m be a new Map of Text to Tag.\nSet item \"k\" of m to a new Tag with name \"v\".\nShow \"{m}\"."),
        // A value displayed through a COPY of a declared binding, built from an iteration — the shape
        // a page's journal renders every changed field in (`Let field be …`, then `"{field}"`).
        (&["Seq", "Record"], "## To kept(ns: Seq of Note) -> Text:\n    Let mutable out be a new Seq of Note.\n    Repeat for n in ns:\n        If n's score is greater than 0.5:\n            Push n to out.\n    Let shown be out.\n    Return \"{shown}\" + \" / \" + shown.\n\n## Main\nLet ns be a new Seq of Note.\nPush a new Note with title \"a\" and tags [] and score 1.0 and pinned true to ns.\nPush a new Note with title \"b\" and tags [\"t\"] and score 0.0 and pinned false to ns.\nShow kept(ns)."),
        (&["Seq", "Variant"], "## To circles(xs: Seq of Shape) -> Text:\n    Let mutable out be a new Seq of Shape.\n    Repeat for s in xs:\n        Push s to out.\n    Let shown be out.\n    Return \"{shown}\".\n\n## Main\nLet xs be a new Seq of Shape.\nPush a new Circle with radius 2 to xs.\nPush a new Dot to xs.\nShow circles(xs)."),
        (&["Map", "Record"], "## To tags(m: Map of Text to Tag) -> Text:\n    Let same be m.\n    Return \"{same}\".\n\n## Main\nLet m be a new Map of Text to Tag.\nSet item \"x\" of m to a new Tag with name \"y\".\nShow tags(m)."),
    ]
}

fn program(body: &str) -> String {
    if body.starts_with("## To ") {
        format!("{TYPES}{body}\n")
    } else {
        format!("{TYPES}## Main\n{body}\n")
    }
}

#[test]
fn every_value_shape_renders_the_same_text_on_every_tier() {
    let mut failures = Vec::new();
    for (shapes, body) in rows() {
        let src = program(body);
        let tw = tw_outcome(&src);
        let vm = vm_outcome(&src);
        let wasm = run_outcome(&src)
            .map(|r| (r.output, r.failure))
            .unwrap_or_else(|why| (String::new(), Some(why)));
        let t = |s: &str| s.trim_end().to_string();
        let (tw, vm, wasm) = (
            (t(&tw.output), tw.error),
            (t(&vm.output), vm.error),
            (t(&wasm.0), wasm.1),
        );
        if tw.1.is_some() || vm != tw || wasm != tw {
            failures.push(format!(
                "── {shapes:?}\n  tree-walker {tw:?}\n  VM {vm:?}\n  wasm {wasm:?}\n{src}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} row(s) diverge:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Every variant of the display module's `Shown` enum is covered by at least one row — read from the
/// module's SOURCE, so a shape it learns to render cannot go unexercised here.
#[test]
fn every_display_shape_has_a_row() {
    let src = include_str!("../../logicaffeine_compile/src/vm/wasm/module/display.rs");
    let body = src
        .split("pub(super) enum Shown {")
        .nth(1)
        .expect("display.rs declares `enum Shown`");
    let body = &body[..body.find("\n}").expect("the enum closes")];
    let variants: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("///") && !l.starts_with("//"))
        .map(|l| {
            l.split(|c: char| c == '(' || c == ',')
                .next()
                .unwrap_or(l)
                .trim()
        })
        .collect();
    assert!(
        variants.len() >= 10,
        "read only {variants:?} out of `enum Shown` — the reader is broken"
    );
    let covered: Vec<&str> = rows()
        .iter()
        .flat_map(|(shapes, _)| shapes.iter().copied())
        .collect();
    let missing: Vec<&&str> = variants.iter().filter(|v| !covered.contains(v)).collect();
    assert!(
        missing.is_empty(),
        "display shapes with no row here: {missing:?}"
    );
}
