//! `contains(haystack, needle)` — a Text substring predicate, proven identical on
//! the tree-walker and the register VM (and, in `wasm_dom`/site tests, the WASM
//! backend). The data-room's FACTS browser searches with it; more broadly it is
//! the substring primitive the Text type was missing next to `length of`.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

fn tw(src: &str) -> String {
    tw_outcome(src).output.trim().to_string()
}
fn vm(src: &str) -> String {
    vm_outcome(src).output.trim().to_string()
}

const HIT: &str = concat!(
    "## Main\n",
    "Let h be \"pigeonhole beats Z3\".\n",
    "If contains(h, \"beats\"):\n",
    "    Show \"yes\".\n",
    "Otherwise:\n",
    "    Show \"no\".\n",
);

const MISS: &str = concat!(
    "## Main\n",
    "Let h be \"pigeonhole beats Z3\".\n",
    "If contains(h, \"zebra\"):\n",
    "    Show \"yes\".\n",
    "Otherwise:\n",
    "    Show \"no\".\n",
);

const EMPTY_NEEDLE: &str = concat!(
    "## Main\n",
    "If contains(\"anything\", \"\"):\n",
    "    Show \"yes\".\n",
    "Otherwise:\n",
    "    Show \"no\".\n",
);

#[test]
fn contains_true_on_tw_and_vm() {
    assert_eq!(tw(HIT), "yes", "tree-walker");
    assert_eq!(vm(HIT), "yes", "VM");
}

#[test]
fn contains_false_on_tw_and_vm() {
    assert_eq!(tw(MISS), "no", "tree-walker");
    assert_eq!(vm(MISS), "no", "VM");
}

#[test]
fn contains_empty_needle_is_true_on_tw_and_vm() {
    // The empty string is a substring of everything (Rust `str::contains` and
    // every mainstream language agree), so the predicate is total.
    assert_eq!(tw(EMPTY_NEEDLE), "yes", "tree-walker");
    assert_eq!(vm(EMPTY_NEEDLE), "yes", "VM");
}
