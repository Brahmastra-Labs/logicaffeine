//! LOCK — a store changes only through its own `Actions:`. `Set Cart's count to 5.` reads as
//! ordinary field mutation and used to compile clean on every tier — and then either wrote a copy
//! nothing read or a global nothing repainted, since a store's `Actions:` are the only place a turn
//! recomputes its derived fields, persists it, or notifies what shares it. The write compiled and
//! was silently lost. `Call <Store>'s <action>.` is the one door in; every other mutating sentence
//! whose root names a declared store is refused, naming the store's own actions.

#[path = "language_common/mod.rs"]
mod common;

use common::refusal;

const CART: &str = "## Store Cart\nState:\n    Let count be 0.\n    Let items be a new Seq of Text.\nActions:\n    To add (n: Int):\n        Set count to count + n.\n\n";

fn app(body: &str) -> String {
    format!(
        "{CART}## Component App\nUse Cart.\nActions:\n    To go:\n        {body}\nView:\n    <p on-click={{go}}>{{Cart's count}}</p>\n\n## Main\nMount App.\n"
    )
}

#[test]
fn a_direct_set_on_a_store_field_is_refused() {
    let (message, covered) = refusal(&app("Set Cart's count to 5."));
    assert_eq!(
        covered, "Cart",
        "the span covers the store's own name: {message}"
    );
    assert!(message.contains("`Set Cart's … to …`"), "{message}");
    assert!(
        message.contains("changes only through its own `Actions:`"),
        "{message}"
    );
    assert!(message.contains("its actions are add"), "{message}");
    assert!(
        message.contains("Call `Cart`'s action instead: `Call Cart's <action>.`"),
        "{message}"
    );
}

#[test]
fn every_mutating_verb_on_a_store_is_refused_the_same_way() {
    const WRITES: &[(&str, &str)] = &[
        ("Set Cart's count to 5.", "`Set Cart's … to …`"),
        ("Toggle Cart.", "`Toggle Cart.`"),
        ("Push \"x\" to Cart.", "`Push … to Cart`"),
        ("Add \"x\" to Cart.", "`Add … to Cart`"),
        ("Remove \"x\" from Cart.", "`Remove … from Cart`"),
        ("Pop from Cart.", "`Pop from Cart`"),
        ("Increase Cart by 1.", "`Increase Cart by …`"),
        ("Decrease Cart by 1.", "`Decrease Cart by …`"),
    ];
    for (sentence, quoted) in WRITES {
        let (message, covered) = refusal(&app(sentence));
        assert_eq!(covered, "Cart", "`{sentence}`: {message}");
        assert!(message.contains(quoted), "`{sentence}`: {message}");
    }
}

#[test]
fn a_write_reaching_a_store_from_outside_any_component_is_still_refused() {
    let src = format!("{CART}## Main\nSet Cart's count to 5.\n");
    let (message, covered) = refusal(&src);
    assert_eq!(covered, "Cart");
    assert!(
        message.contains("changes only through its own `Actions:`"),
        "{message}"
    );
}

#[test]
fn calling_the_stores_own_action_is_unaffected() {
    let src = format!("{CART}## Component App\nUse Cart.\nActions:\n    To go:\n        Call Cart's add with 1.\nView:\n    <p on-click={{go}}>{{Cart's count}}</p>\n\n## Main\nMount App.\n");
    let parsed = common::accepted(&src);
    assert!(!parsed.stmts.is_empty());
}

#[test]
fn writing_a_plain_state_field_of_the_same_shape_is_unaffected() {
    // A local `Let count be 0.` in the COMPONENT's own `State:` is not the store, and `Set count`
    // there must not be mistaken for `Set Cart's count` — no possessive, no store involved.
    let src = "## Component App\nState:\n    Let count be 0.\nActions:\n    To go:\n        Set count to count + 1.\nView:\n    <p on-click={go}>{count}</p>\n\n## Main\nMount App.\n";
    common::accepted(src);
}
