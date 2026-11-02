//! ★ LOCK — `the count of` means the same thing on the tier a server ships as. ★
//!
//! `the count of xs` is the English spelling of "how many", and it works on the tree-walker and the
//! VM. On **AOT-Rust it emits a refusal**:
//!
//! ```text
//! if (i < compile_error!("LGX: a page query has no AOT-Rust representation yet")) { … }
//! ```
//!
//! ★ THAT IS THE WORST SPLIT AVAILABLE, and it is the one `server_surface_deploys_lock` was written
//! about: the suite is green because every test of this form runs on a tier that infers loosely,
//! and the binary does not build. It was found from a long way off — a `## Session` payload helper
//! in `session.lg` used `the count of parts` and the deploy-tier assertion of `session_block_lock`
//! failed on generated code nobody had typed.
//!
//! ★ THE CAUSE IS A FORM CLAIMED TOO EAGERLY. `the count of` is ALSO how a page asks how many
//! elements match a selector — a page QUERY, which genuinely has no AOT-Rust representation because
//! there is no document on a server. So the LGX lowering claims every `the count of`, and a
//! sequence's length is collateral. The two are told apart by what follows: a selector is a
//! STRING (`the count of ".todo"`), a length is anything else.
//!
//! The standard library says `length of xs` and means it, so nothing in the prelude depended on
//! the broken path — which is exactly why it survived: the one form that is ordinary LOGOS was
//! used by ordinary LOGOS everywhere and by the standard library nowhere.

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome};
use logicaffeine_compile::semantics::dom;

/// Compiles on the deploy tier, with the emitted Rust read for refusals — `Ok` proves nothing.
fn builds(subject: &str, src: &str) {
    let rust =
        compile_to_rust(src).unwrap_or_else(|e| panic!("`{subject}` must compile: {e:?}\n{src}"));
    let refusals: Vec<&str> = rust
        .lines()
        .filter(|l| l.contains("compile_error!"))
        .collect();
    assert!(
        refusals.is_empty(),
        "`{subject}` runs on the tree-walker and would NOT BUILD as a binary:\n{}\n{src}",
        refusals.join("\n")
    );
}

fn prints(subject: &str, src: &str) -> String {
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "`{subject}` must run:\n{src}");
    out.output.trim().to_string()
}

#[test]
fn the_count_of_a_sequence_builds_on_the_deploy_tier() {
    let src = "## Main\nLet xs be [1, 2, 3].\nShow the count of xs.\n";
    assert_eq!(prints("the count of a list", src), "3");
    builds("the count of a list", src);
}

#[test]
fn it_builds_inside_a_function_and_a_condition() {
    // The failure was found inside a function, in a comparison — the position a length is actually
    // used in, and one no `Show` at the top level would have covered.
    let src = "## To sizeOf (xs: Seq of Text) -> Int:\n\x20   Return the count of xs.\n\n\
               ## To hasTwo (xs: Seq of Text) -> Bool:\n\
              \x20   If 2 is greater than the count of xs:\n\x20       Return false.\n\
              \x20   Return true.\n\n\
               ## Main\nLet xs be [\"a\", \"b\", \"c\"].\nShow sizeOf(xs).\nShow hasTwo(xs).\n";
    assert_eq!(prints("in a function", src), "3\ntrue");
    builds("in a function", src);
}

#[test]
fn it_builds_for_the_other_collections() {
    let src = "## Main\n\
               Let m be a new Map of Text to Int.\n\
               Set item \"a\" of m to 1.\n\
               Show the count of m.\n\
               Let s be split(\"a.b.c\", \".\").\n\
               Show the count of s.\n";
    assert_eq!(prints("a map and a split", src), "1\n3");
    builds("a map and a split", src);
}

/// A mounted page with two `.row` elements — so a selector has something real to match.
const PAGE: &str = "\
## Component App
State:
    Let name be \"ada\".
View:
    <ul>
      <li class=\"row\">a</li>
      <li class=\"row\">b</li>
    </ul>

## Main
Mount App.
";

#[test]
fn a_page_query_is_still_a_page_query() {
    // ★ THE HALF THAT WOULD CATCH OVER-CORRECTING, and it is sharp: `".row"` is four characters
    // and matches two elements, so a reading that turned the selector into a LENGTH would answer
    // `4` here. `page_query_lock` already stated the rule this file now applies at parse time —
    // *"the VALUE settles the reading: only a Text is a selector"* — it was simply applied when
    // the expression was EVALUATED, which is a place AOT-Rust never reaches.
    dom::reset();
    let out = tw_outcome(&format!("{PAGE}Show the count of \".row\".\n"));
    assert_eq!(out.error, None);
    assert_eq!(
        out.output.trim(),
        "2",
        "a literal selector must still COUNT MATCHES; `4` would mean it had been read as the \
         length of the string `.row`"
    );
}

#[test]
fn the_journal_is_still_a_page_query() {
    // The one NON-literal a page query legitimately takes, and the other way the discriminator
    // could have been written too narrowly.
    dom::reset();
    let out = tw_outcome(&format!("{PAGE}Show the count of the journal.\n"));
    assert_eq!(out.error, None);
    assert_eq!(out.output.trim(), "0");
}

#[test]
fn length_of_is_unchanged() {
    // The floor: the spelling the standard library uses everywhere keeps meaning what it meant.
    let src = "## Main\nLet xs be [1, 2, 3].\nShow length of xs.\n";
    assert_eq!(prints("length of", src), "3");
    builds("length of", src);
}
