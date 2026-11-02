//! LOCK — a `## Store` field of every shape the lowering types reaches every engine with that type.
//!
//! A store lowers to a record, `__Store_<Name>`, and every engine lays that record out from the
//! type registry built BEFORE parsing (`analysis::discovery`) — so the discovery scan and the
//! lowering must type each field the same way. The lowering learned `Seq of <element>` fields (by
//! annotation and by `a new Seq of …`) while discovery still read only a literal or a one-word
//! annotation, and skipped the rest: the record reached the engines without the field, and a store
//! action walking a list of records (`Repeat for n in notes: … n's number …`) had no slot for
//! `number` on wasm — "field access with no static slot" — while the tree-walker and VM ran it.

use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

/// `src` on the tree-walker and on the lowered VM (their documents must agree) and built for the
/// browser.
fn agrees_and_builds(src: &str) -> String {
    dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker runs it:\n{src}");
    let doc = dom::render();
    dom::reset();
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the lowered VM runs it:\n{src}");
    assert_eq!(dom::render(), doc, "both tiers draw the same page");
    let built = compile_to_wasm(src);
    assert!(
        built.is_ok(),
        "the page builds for the browser: {:?}",
        built.err()
    );
    doc
}

/// A store whose field `notes` is declared by `field`, with actions that push a record and walk
/// the list reading each record's field.
fn records_store(field: &str) -> String {
    format!(
        "## A Note has:\n    a number, which is Int.\n\n\
         ## Store Toasts\nState:\n    {field}\n    Let next be 1.\n\
         Actions:\n    To post:\n        Push (a new Note with number next) to notes.\n        Set next to next + 1.\n\
         \x20   To dismiss (number: Int):\n        Let mutable kept be a new Seq of Note.\n\
         \x20       Repeat for n in notes:\n            If n's number is not equal to number:\n                Push n to kept.\n\
         \x20       Set notes to kept.\n\n\
         ## Page /\nUse Toasts.\nActions:\n    To add:\n        Call Toasts's post.\n    To drop:\n        Call Toasts's dismiss with 1.\n\
         View:\n    <main><button id=\"a\" on-click={{add}}>a</button><button id=\"d\" on-click={{drop}}>d</button>\
         <p id=\"n\">{{length of Toasts's notes}}</p></main>\n\n\
         ## Main\nClick \"#a\".\nClick \"#a\".\nClick \"#d\".\n"
    )
}

#[test]
fn a_store_list_of_records_made_new_reaches_every_engine() {
    let doc = agrees_and_builds(&records_store("Let notes be a new Seq of Note."));
    assert!(doc.contains("<p id=\"n\">1</p>"), "{doc}");
}

#[test]
fn a_store_list_of_records_declared_by_annotation_reaches_every_engine() {
    let doc = agrees_and_builds(&records_store(
        "Let notes: Seq of Note be a new Seq of Note.",
    ));
    assert!(doc.contains("<p id=\"n\">1</p>"), "{doc}");
}

#[test]
fn a_store_list_of_text_reaches_every_engine() {
    let src = "## Store Tags\nState:\n    Let tags be a new Seq of Text.\n\
               Actions:\n    To tag:\n        Push \"x\" to tags.\n\n\
               ## Page /\nUse Tags.\nActions:\n    To go:\n        Call Tags's tag.\n\
               View:\n    <main><button id=\"b\" on-click={go}>b</button><ul>{for each t in Tags's tags:}<li>{t}</li>{end}</ul></main>\n\n\
               ## Main\nClick \"#b\".\nClick \"#b\".\n";
    let doc = agrees_and_builds(src);
    assert!(doc.contains("<ul><li>x</li><li>x</li></ul>"), "{doc}");
}
