//! ★ LOCK — the copy-on-write barrier still protects everything an author can observe.
//!
//! Writing one element of a GLOBAL collection reads it into a register first, so the allocation is
//! held twice — by the globals store and by that register. `ensure_reg_owned` sees
//! `strong_count > 1` and DEEP-CLONES the whole collection before the write. For LGX that is
//! quadratic: a component's state lives in `__S_<C>_<field>` keyed by instance, the map grows with
//! every instance ever created, and every turn copies it once per field.
//!
//! Measured on a five-row keyed list churned 160 times, min of five reps: per-round cost rose with
//! history AND with field count — slopes 0.81 / 2.08 / 4.50 for 1 / 2 / 4 state fields — and
//! disabling the barrier entirely flattened it (slopes −0.43 / −0.64 / −0.43) and cut absolute cost
//! 16×. The fix RELEASES the globals store's reference before the write, so the register is sole
//! owner and the write lands in place: 2.1–2.5× faster, and growth no longer scales with field
//! count.
//!
//! ★ IT IS RESTRICTED TO COMPILER-WRITTEN NAMES, AND THIS FILE IS WHY. A `__`-prefixed global is
//! storage the compiler minted, which nothing else can alias; an author's global stays on the
//! original path because value semantics are something they can SEE. If that restriction is ever
//! widened, these tests are what notices — they are the observable half of the optimization.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// Both tiers must agree, because value semantics is a language rule rather than a tier's habit.
fn both(label: &str, src: &str) -> String {
    let tw = tw_outcome(src);
    assert_eq!(
        tw.error, None,
        "`{label}` must run on the tree-walker:\n{src}"
    );
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "`{label}` must run on the VM:\n{src}");
    // Trimmed: one tier ends its output with a newline and the other does not, which is a
    // formatting difference rather than a semantic one and is not what this file is about.
    assert_eq!(
        tw.output.trim(),
        vm.output.trim(),
        "`{label}` differs across tiers:\n{src}"
    );
    tw.output.trim().to_string()
}

/// A copy of a map is a COPY. Writing through one name must not be visible through the other.
#[test]
fn a_copied_map_does_not_share_writes() {
    let out = both(
        "map copy",
        "## Main\nLet mutable a be {\"k\": 1}.\nLet mutable b be a.\n\
         Set item \"k\" of a to 99.\nShow item \"k\" of b.\n",
    );
    assert_eq!(
        out, "1",
        "`b` was copied BEFORE the write, so it still reads 1"
    );
}

/// The same for a list.
#[test]
fn a_copied_list_does_not_share_writes() {
    let out = both(
        "list copy",
        "## Main\nLet mutable a be [1, 2, 3].\nLet mutable b be a.\n\
         Set item 1 of a to 99.\nShow item 1 of b.\n",
    );
    assert_eq!(out, "1", "`b` keeps the values it was copied with");
}

/// And the write really did land on the original — otherwise the tests above would pass with
/// element writes broken entirely.
#[test]
fn the_write_still_lands_on_the_collection_it_named() {
    let out = both(
        "write lands",
        "## Main\nLet mutable a be {\"k\": 1}.\nLet mutable b be a.\n\
         Set item \"k\" of a to 99.\nShow item \"k\" of a.\n",
    );
    assert_eq!(out, "99", "`a` sees its own write");
}

/// ★ AND AN LGX PAGE'S STATE SURVIVES THE SAME TREATMENT. The optimization fires on `__S_…` maps
/// every turn; this is the end-to-end statement that a page still remembers what it was told.
#[test]
fn an_lgx_page_keeps_per_instance_state_across_many_turns() {
    let src = "## Component Row (k: Int)\n\
               State:\n\x20   Let n be 0.\n\
               Actions:\n\x20   To bump:\n\x20       Set n to n + 1.\n\
               View:\n\x20   <li id=\"r{k}\">{k}:{n}<button id=\"b{k}\" on-click={bump}>+</button></li>\n\n\
               ## Component App\n\
               View:\n\x20   <ul>{for each i in [1, 2, 3], keyed by i:}<Row k={i}/>{end}</ul>\n\n\
               ## Main\nMount App.\n\
               Click \"#b2\".\nClick \"#b2\".\nClick \"#b3\".\n";
    use logicaffeine_compile::semantics::dom;
    dom::reset();
    assert_eq!(tw_outcome(src).error, None);
    let tw = dom::render();
    dom::reset();
    assert_eq!(vm_outcome(src).error, None);
    let vm = dom::render();
    assert_eq!(tw, vm, "both tiers agree about what each row remembers");
    assert!(tw.contains("1:0"), "row 1 was never clicked: {tw}");
    assert!(
        tw.contains("2:2"),
        "row 2 counted twice — its map write landed each time: {tw}"
    );
    assert!(tw.contains("3:1"), "row 3 counted once: {tw}");
}
