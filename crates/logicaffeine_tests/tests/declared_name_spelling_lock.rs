//! LOCK — a name an author declares is the name as they spelled it.
//!
//! `## Store Messages`, `## Component Toasts`, `## Directive highlights`: each header names a thing
//! the rest of the file refers to by that exact spelling — `Call Messages's post.`, `Use Toasts.`,
//! `<Toasts/>`, `<p highlights={v}>`. A header that recorded the word's dictionary form instead
//! (`message`, `toast`, `highlight`, because the lexicon knows the plural noun) left every
//! reference pointing at a name nothing declared: a store "is not a store", a component is not
//! found, and a directive silently renders as a plain attribute.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

/// `src` on the tree-walker and on the lowered VM: each tier's document, which must agree.
fn rendered_on_both_tiers(src: &str) -> String {
    dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker runs it:\n{src}");
    let tw_doc = dom::render();
    dom::reset();
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the lowered VM runs it:\n{src}");
    assert_eq!(dom::render(), tw_doc, "both tiers draw the same page");
    tw_doc
}

#[test]
fn a_store_named_like_a_plural_noun_is_called_by_its_spelling() {
    for store in ["Messages", "Toasts"] {
        let src = format!(
            "## Store {store}\nState:\n    Let n be 0.\nActions:\n    To post:\n        Set n to n + 1.\n\n\
             ## Page /\nUse {store}.\nActions:\n    To go:\n        Call {store}'s post.\n\
             View:\n    <p id=\"n\">{{{store}'s n}}</p><button id=\"b\" on-click={{go}}>b</button>\n\n\
             ## Main\nClick \"#b\".\n"
        );
        let doc = rendered_on_both_tiers(&src);
        assert!(doc.contains("<p id=\"n\">1</p>"), "{store}: {doc}");
    }
}

#[test]
fn a_component_named_like_a_plural_noun_is_placed_by_its_spelling() {
    let src = "## Component Toasts\nView:\n    <p class=\"t\">toasts</p>\n\n\
               ## Page /\nView:\n    <main><Toasts/></main>\n\n## Main\n";
    let doc = rendered_on_both_tiers(src);
    assert!(
        doc.contains("<main><p class=\"t\">toasts</p></main>"),
        "{doc}"
    );
}

#[test]
fn a_directive_named_like_a_plural_noun_runs_on_its_spelling() {
    let src = "## Directive highlights (el: Element) and (v: Text):\n    Call setAttribute with el and \"title\" and v.\n\n\
               ## Page /\nView:\n    <p highlights=\"x\">p</p>\n\n## Main\n";
    let doc = rendered_on_both_tiers(src);
    assert!(
        doc.contains("<p title=\"x\">p</p>"),
        "the directive ran rather than rendering as an attribute: {doc}"
    );
}

/// ★ AND EVERY REFERENCE READS IT THE SAME WAY. With `## Component Toast` declared, the lexer knows
/// `Toasts` as the plural of a declared name; a store named `Toasts` beside it is still referred to
/// by its own spelling — `Use`, `Call …'s`, a `When …'s … changes:` trigger and a possessive read.
#[test]
fn a_store_named_as_another_components_plural_is_referred_to_by_its_spelling() {
    let src = "## Store Toasts\nState:\n    Let n be 0.\nActions:\n    To post:\n        Set n to n + 1.\n\n\
               ## Component Toast\nUse Toasts.\nState:\n    Let heard be 0.\nActions:\n    When Toasts's n changes:\n        Set heard to heard + 1.\n\
               View:\n    <p id=\"t\">{heard}|{Toasts's n}</p>\n\n\
               ## Page /\nUse Toasts.\nActions:\n    To go:\n        Call Toasts's post.\n\
               View:\n    <main><Toast/><button id=\"b\" on-click={go}>b</button></main>\n\n\
               ## Main\nClick \"#b\".\n";
    let doc = rendered_on_both_tiers(src);
    assert!(doc.contains("<p id=\"t\">1|1</p>"), "{doc}");
}
