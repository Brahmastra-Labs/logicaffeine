//! ★ LOCK — Tristen's rule: a `keyed by` loop whose siblings share a key fails LOUDLY, on every
//! compiled tier, naming the loop, the key and both (1-based) positions; no aliasing, no
//! positional fallback. ★
//!
//! This is the LOWERING half — `lgx_lower.rs`'s `duplicate_key_check` emits a pre-pass `Repeat`
//! over the WHOLE collection, before any row renders, tracking every key it has seen, in order, in
//! a fresh `Seq of Text`, and `Fail`ing the moment a second item's key is already in it. The
//! message is built from the SAME literal parts (`duplicate_keyed_loop_message_parts`) the
//! tree-walker's own `duplicate_keyed_loop_message` formats with, so the wording is identical on
//! every tier by construction, not by two authors agreeing to copy it twice.
//!
//! ★ A `Map`, NOT A `Seq`, WAS THE FIRST DRAFT — REVERTED. ★ A freshly-declared LOCAL `Map of Text
//! and Int` hit a real wasm backend value-kind inference gap (`vm/wasm/module.rs`'s own
//! `map_value_load`/`map_value_store`, unrelated to this feature and outside this lane's
//! ownership) and refused EVERY keyed page's wasm build, duplicate or not — caught live by
//! safety2's `browser_tier_lock` run against `examples/todo`. Switching to `Seq` (`Contains`) hit
//! a SECOND, separate wasm gap ("unsupported contains over a sequence of Text — needs byte-
//! equality"). The fix that actually lowers: no `Contains` either — the pre-pass's own duplicate-
//! position scan (a `Repeat` over the `Seq` with a plain `Text` `Eq` compare, the SAME comparison
//! an event-modifier match already lowers everywhere) does double duty as the membership test
//! too, so nothing here needs anything beyond `Seq`/`Push`/`Eq`/`Repeat` — every one of them
//! already proven on wasm by the keyed reconciler's own `__KEYS_<C>` tables. Verified directly
//! against `examples/todo`, `wasm_module_validity_lock`, and the wasm-runtime test in
//! `wasm_dom.rs` (`an_early_return_still_commits_its_write_back_on_wasm`'s neighbor) before
//! landing, not merely compiled and assumed.

use logicaffeine_compile::compile::{compile_to_wasm, vm_outcome};

const DUPLICATE_SRC: &str = "## Component App\n\
    State:\n    Let items be [1, 2, 1].\n\
    View:\n    <main><ul>{for each x in items, keyed by x:}<li>{x}</li>{end}</ul></main>\n\n\
    ## Main\nMount App.\n";

const DISTINCT_SRC: &str = "## Component App\n\
    State:\n    Let items be [1, 2, 3].\n\
    View:\n    <main><ul>{for each x in items, keyed by x:}<li>{x}</li>{end}</ul></main>\n\n\
    ## Main\nMount App.\n";

#[test]
fn a_duplicate_key_fails_loudly_on_the_vm_naming_the_loop_key_and_both_positions() {
    let out = vm_outcome(DUPLICATE_SRC);
    let err = out
        .error
        .expect("a duplicate key must fail the page, not silently render it");
    assert_eq!(
        err,
        "the loop over items keys item 1 and item 3 both \"1\" — a `keyed by` loop's keys must \
         be unique among its own siblings; give the duplicate its own key, or drop `keyed by` if \
         position alone already identifies each row here.",
    );
}

#[test]
fn distinct_keys_are_not_flagged_on_the_vm() {
    let out = vm_outcome(DISTINCT_SRC);
    assert!(
        out.error.is_none(),
        "three distinct keys must not be flagged: {:?}",
        out.error
    );
}

/// A duplicate in the MIDDLE of a longer list still names the exact two positions that collide,
/// not merely "a" duplicate exists — proving the pre-pass does not stop at the first item, and
/// does not confuse "seen before" with "seen at position 1" specifically.
#[test]
fn a_duplicate_key_later_in_the_list_still_names_its_own_two_positions() {
    const SRC: &str = "## Component App\n\
        State:\n    Let items be [10, 20, 30, 20, 40].\n\
        View:\n    <main><ul>{for each x in items, keyed by x:}<li>{x}</li>{end}</ul></main>\n\n\
        ## Main\nMount App.\n";
    let out = vm_outcome(SRC);
    let err = out.error.expect("a duplicate key must fail the page");
    assert!(
        err.contains("item 2 and item 4"),
        "must name positions 2 and 4 (both \"20\"): {err}"
    );
    assert!(
        err.contains("\"20\""),
        "must name the colliding key's own value: {err}"
    );
}

/// A page with NO keyed loop at all must be entirely unaffected — the pre-pass is only ever
/// emitted for a loop that actually says `keyed by`.
#[test]
fn a_page_with_no_keyed_loop_is_unaffected() {
    const SRC: &str = "## Component App\n\
        State:\n    Let items be [1, 1, 1].\n\
        View:\n    <main><ul>{for each x in items:}<li>{x}</li>{end}</ul></main>\n\n\
        ## Main\nMount App.\n";
    let out = vm_outcome(SRC);
    assert!(
        out.error.is_none(),
        "no `keyed by` means no duplicate-key check at all: {:?}",
        out.error
    );
}

/// A keyed page with DISTINCT keys must compile to wasm cleanly — the pre-pass itself must never
/// be the reason a normal, well-formed keyed page stops shipping.
#[test]
fn a_keyed_page_with_distinct_keys_compiles_to_wasm() {
    compile_to_wasm(DISTINCT_SRC).expect("a keyed page with distinct keys must compile to wasm");
}

/// A keyed page with a DUPLICATE must ALSO compile — the check is a runtime `Fail`, not a
/// compile-time refusal — see `wasm_dom.rs`'s own runtime test for the trap actually firing with
/// the identical message the VM tier gives.
#[test]
fn a_keyed_page_with_a_duplicate_still_compiles_to_wasm() {
    compile_to_wasm(DUPLICATE_SRC)
        .expect("the duplicate-key check is a RUNTIME Fail, not a compile-time refusal — the page must still compile");
}
