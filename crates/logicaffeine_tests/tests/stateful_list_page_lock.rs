//! ★ LOCK — a page that HAS STATE and shows a list of records reaches the browser. ★
//!
//! This is the shape of essentially every real page: some state, a `{for each}` over a list of
//! records, a field of each row on the screen. It stopped compiling to wasm:
//!
//! ```text
//! wasm AOT backend: unsupported field access with no static slot
//! ```
//!
//! ★ AND ONLY WHEN THE PAGE IS STATEFUL, which is what made it hard to see. A page with the same
//! `State:` and the same loop but NO `Actions:` compiles fine. The difference is that a component
//! with actions keeps its state PER INSTANCE — each field is a `Map of Text to <field type>` keyed
//! by the instance path, seeded `mapOf("", …)` — so a field declared `Seq of Memo` reaches the
//! backend as a global typed `Map of Text to (Seq of Memo)`.
//!
//! The wasm backend's static-slot plan resolves a row's field offsets from a global's DECLARED
//! type. It knows two shapes: a global that IS a record, and a global that IS a `Seq of <record>`.
//! A global that is a MAP whose value is a `Seq of <record>` matched neither, so the loop variable
//! taken out of it carried no layout and `row's title` had no slot.
//!
//! Two correct changes met and produced a broken product: per-instance state is right, and
//! resolving layouts from declared types is right. Neither is wrong on its own — the map layer was
//! simply new, and nothing asked whether the layout plan could see through it.
//!
//! ★ WHY NOTHING CAUGHT IT: the wasm locks that cover records-in-lists use PLAIN LOGOS programs
//! (`wasm_record_list_lock`), which have no components and therefore no per-instance map; the LGX
//! locks that cover stateful pages use SCALAR state. The intersection — stateful AND a list of
//! records — was the one square nothing stood on, and it is the square every real page occupies.

use logicaffeine_compile::compile::compile_to_wasm;

const MEMO: &str = "## A Memo has:\n    an id, which is Int.\n    a title, which is Text.\n\
    \x20   a pinned, which is Bool.\n\n";

/// A page with state, an action, and a list of records on screen.
fn stateful_list_page(loop_head: &str) -> String {
    format!(
        "{MEMO}## Page /\n\
         State:\n\
        \x20   Let notes be a new Seq of Memo.\n\
         Actions:\n\
        \x20   To go:\n\
        \x20       Set notes to notes.\n\
         View:\n\
        \x20   <ul>{{{loop_head}}}<li>{{n's title}}</li>{{end}}</ul>\n\
        \x20   <button id=\"b\" on-click={{go}}>x</button>\n"
    )
}

#[test]
fn a_stateful_page_showing_a_list_of_records_compiles_for_the_browser() {
    let src = stateful_list_page("for each n in notes:");
    let bytes = compile_to_wasm(&src).unwrap_or_else(|e| {
        panic!("a stateful page with a list of records must reach the browser: {e:?}\n{src}")
    });
    assert_eq!(&bytes[..4], b"\0asm", "and it must be a real module");
}

#[test]
fn the_same_page_keyed_compiles_too() {
    // `keyed by` is what a real list uses, and it changes how the row is addressed — so it gets its
    // own row here rather than being assumed to follow.
    let src = stateful_list_page("for each n in notes, keyed by n's id:");
    let bytes = compile_to_wasm(&src).unwrap_or_else(|e| panic!("keyed: {e:?}\n{src}"));
    assert_eq!(&bytes[..4], b"\0asm");
}

#[test]
fn a_child_component_reading_a_row_of_stateful_state_compiles() {
    // The full shape: the row is a component, handed the record as a prop.
    let src = format!(
        "{MEMO}## Component Row (entry: Memo)\n\
         View:\n\
        \x20   <li class:pinned={{entry's pinned}}>{{entry's title}}</li>\n\n\
         ## Page /\n\
         State:\n\
        \x20   Let notes be a new Seq of Memo.\n\
         Actions:\n\
        \x20   To go:\n\
        \x20       Set notes to notes.\n\
         View:\n\
        \x20   <ul>{{for each n in notes, keyed by n's id:}}<Row entry={{n}}/>{{end}}</ul>\n\
        \x20   <button id=\"b\" on-click={{go}}>x</button>\n"
    );
    let bytes = compile_to_wasm(&src).unwrap_or_else(|e| panic!("child row: {e:?}\n{src}"));
    assert_eq!(&bytes[..4], b"\0asm");
}

#[test]
fn the_stateless_form_still_compiles() {
    // ★ THE CONTROL THAT LOCATES IT. This one never broke, and keeping it here is what says the
    // three above are about STATE rather than about lists — the distinction that made the bug
    // invisible for as long as it was.
    let src = format!(
        "{MEMO}## Page /\n\
         State:\n\
        \x20   Let notes be a new Seq of Memo.\n\
         View:\n\
        \x20   <ul>{{for each n in notes:}}<li>{{n's title}}</li>{{end}}</ul>\n"
    );
    assert!(
        compile_to_wasm(&src).is_ok(),
        "the stateless page was always fine:\n{src}"
    );
}

#[test]
fn a_stateful_page_with_a_scalar_still_compiles() {
    // The other half of the control: state alone is not the problem either.
    let src = "## Page /\nState:\n\x20   Let n be 0.\n\
        Actions:\n\x20   To go:\n\x20       Set n to n + 1.\n\
        View:\n\x20   <p>{n}</p>\n\x20   <button id=\"b\" on-click={go}>x</button>\n";
    assert!(compile_to_wasm(src).is_ok(), "scalar state was always fine");
}
