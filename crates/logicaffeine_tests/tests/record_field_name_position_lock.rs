//! A field's name — a record's or a variant's — is known by its POSITION, never by the word's part
//! of speech.
//!
//! After an article (`a first, which is Int`) any word is the name, and so is the word directly
//! before the `:` of `name: Type` or the `,` of `name, which is Type`. In a variant's `with` list
//! the name is the first word of each field (`with first Int and second Text`). An article is
//! itself the name where the position says so: `a: Int.`, `with a Int and b Int`, `with a Seq of
//! Int` — and stays an article before a name (`with a radius Int`). The lexer calls `first` an
//! ordinal, `who` an interrogative and `a` an article; any of them dropped from the registry left
//! the tree-walker unaffected (it builds values dynamically) while every compiled tier refused the
//! field. Each program must print the same on the tree-walker, the VM, AOT-Rust and C.

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

fn on_every_tier(src: &str, expected: &str) {
    assert_all_tiers(src, expected);
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, expected);
}

#[test]
fn ordinal_field_names_in_the_concise_form() {
    on_every_tier(
        "## A Pair has:\n    first: Int.\n    second: Text.\n\n## Main\nLet p be a new Pair with first 1 and second \"one\".\nShow p's first.\nShow p's second.\n",
        "1\none",
    );
}

#[test]
fn ordinal_field_names_in_the_natural_form() {
    on_every_tier(
        "## A Pair has:\n    first, which is Int.\n    second, which is Text.\n\n## Main\nLet p be a new Pair with first 2 and second \"two\".\nShow p's first.\nShow p's second.\n",
        "2\ntwo",
    );
}

#[test]
fn interrogative_field_names_in_the_concise_form() {
    on_every_tier(
        "## An Entry has:\n    who: Text.\n    what: Text.\n\n## Main\nLet e be a new Entry with who \"ann\" and what \"hello\".\nShow e's who.\nShow e's what.\n",
        "ann\nhello",
    );
}

/// The Studio's own generics example: a type parameter named `A` (an article to the lexer) and
/// ordinal field names, together.
#[test]
fn a_generic_pair_whose_parameter_is_named_a() {
    on_every_tier(
        "## A Pair of [A] and [B] has:\n    first: A.\n    second: B.\n\n## Main\nLet entry be a new Pair of Text and Int with first \"score\" and second 97.\nShow \"Pair: {entry's first} = {entry's second}.\".\n",
        "Pair: score = 97.",
    );
}

#[test]
fn a_record_field_named_a_in_the_concise_form() {
    on_every_tier(
        "## A P has:\n    a: Int.\n    b: Int.\n\n## Main\nLet p be a new P with a 1 and b 2.\nShow p's a + p's b.\n",
        "3",
    );
}

#[test]
fn a_record_field_named_a_in_the_natural_form() {
    on_every_tier(
        "## A P has:\n    a, which is Int.\n    b, which is Int.\n\n## Main\nLet p be a new P with a 4 and b 5.\nShow p's a * p's b.\n",
        "20",
    );
}

#[test]
fn a_variant_field_named_a() {
    on_every_tier(
        "## A Tri is either:\n    A Z.\n    A T3 with a Int and b Int and c Int.\n\n## Main\nLet x be a new T3 with a 1 and b 2 and c 3.\nInspect x:\n    When Z: Show 0.\n    When T3 (p, q, r):\n        Show p + q + r.\n",
        "6",
    );
}

#[test]
fn a_variant_field_named_a_of_a_generic_type() {
    on_every_tier(
        "## A Bag is either:\n    A Z.\n    A Box with a Seq of Int and b Int.\n\n## Main\nLet x be a new Box with a [1, 2] and b 3.\nInspect x:\n    When Z: Show 0.\n    When Box (xs, n): Show length of xs + n.\n",
        "5",
    );
}

#[test]
fn a_variant_field_named_a_in_the_concise_form() {
    on_every_tier(
        "## A Tri is either:\n    A Z.\n    A T3 (a: Int, b: Int, c: Int).\n\n## Main\nLet x be a new T3 with a 1 and b 2 and c 3.\nInspect x:\n    When Z: Show 0.\n    When T3 (p, q, r):\n        Show p * q * r.\n",
        "6",
    );
}

#[test]
fn ordinal_variant_field_names_in_the_natural_form() {
    on_every_tier(
        "## A Duo is either:\n    A Z.\n    A Pair with first Int and second Text.\n\n## Main\nLet x be a new Pair with first 1 and second \"one\".\nInspect x:\n    When Z: Show 0.\n    When Pair (f, s):\n        Show f.\n        Show s.\n",
        "1\none",
    );
}

#[test]
fn ordinal_variant_field_names_in_the_concise_form() {
    on_every_tier(
        "## A Duo is either:\n    A Z.\n    A Pair (first: Int, second: Text).\n\n## Main\nLet x be a new Pair with first 2 and second \"two\".\nInspect x:\n    When Z: Show 0.\n    When Pair (f, s):\n        Show f.\n        Show s.\n",
        "2\ntwo",
    );
}

#[test]
fn interrogative_variant_field_names() {
    on_every_tier(
        "## A Msg is either:\n    A Z.\n    A Note with who Text and what Text.\n\n## Main\nLet x be a new Note with who \"ann\" and what \"hi\".\nInspect x:\n    When Z: Show 0.\n    When Note (w, m):\n        Show w.\n        Show m.\n",
        "ann\nhi",
    );
}

/// The article before a name stays an article: the position rule must not swallow it.
#[test]
fn an_article_before_a_variant_field_name_is_an_article() {
    on_every_tier(
        "## A Shape is either:\n    A Dot.\n    A Circle with a radius Int.\n    A Ring with a radius, which is Int.\n    A Rect with an x Int and a y Int.\n\n## Main\nLet c be a new Circle with radius 4.\nLet r be a new Ring with radius 5.\nLet q be a new Rect with x 1 and y 2.\nInspect c:\n    When Circle (n): Show n.\n    Otherwise: Show 0.\nInspect r:\n    When Ring (n): Show n.\n    Otherwise: Show 0.\nInspect q:\n    When Rect (p, s): Show p + s.\n    Otherwise: Show 0.\n",
        "4\n5\n3",
    );
}
