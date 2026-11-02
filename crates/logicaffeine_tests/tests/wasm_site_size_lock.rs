//! ★ LOCK — a sequence access or a checked Int operation costs a call, not an expansion. ★
//!
//! Every `item i of xs`, `Set item i of xs to v` and `Push v to xs` the WebAssembly backend lowers
//! used to carry its whole mechanism inline: the 1-based bounds check with its trap, the
//! copy-on-write test with a complete clone (two allocations and a copy loop), the amortized growth
//! of a push (an allocation, a copy loop, a free). A program is mostly such sites — the retained-tree
//! reconciler in `vdom.lg` has hundreds — so a page paid for that mechanism hundreds of times over.
//! The mechanism now lives once, in the module's runtime (`__logos_seq_slot`, `__logos_seq_push_slot`,
//! `__logos_seq_clone`, `__logos_seq_own`), and a site is the call that reaches it — outside a loop.
//! Inside one a read, a write or a push keeps its fast path inline and calls only for its slow path,
//! because there the call is paid per element. An Int `+`, `-` or `*` and its signed-overflow trap
//! follow the same rule (`__logos_add_checked`, `__logos_sub_checked`, `__logos_mul_checked`).
//!
//! Each row measures one site's cost as the growth of a module when the program says it `N` more
//! times, over an element kind and a site shape, and holds it to a budget no inline expansion fits in.

#![cfg(feature = "wasm-jit")]

use logicaffeine_compile::compile::compile_to_wasm;

const REPEATS: usize = 8;

fn size(src: &str) -> usize {
    compile_to_wasm(src)
        .unwrap_or_else(|e| panic!("must compile: {e:?}\n{src}"))
        .len()
}

/// The bytes one more `site` costs: a function saying it `2 * REPEATS` times against one saying it
/// `REPEATS` times, over `REPEATS`. `site(k)` is the `k`th statement; `setup` builds `A` in `Main`.
fn per_site(setup: &str, site: impl Fn(usize) -> String) -> f64 {
    let program = |n: usize| {
        let body: String = (0..n).map(|k| format!("    {}\n", site(k))).collect();
        format!("## To f(i: Int) -> Int:\n{body}    Return i.\n\n## Main\n{setup}Show f(1).\nShow length of A.\n")
    };
    let (one, two) = (size(&program(REPEATS)), size(&program(2 * REPEATS)));
    (two as f64 - one as f64) / REPEATS as f64
}

const INTS: &str = "Let mutable A be [0, 0].\n";
const TEXTS: &str = "Let mutable A be [\"a\", \"b\"].\n";

fn assert_within(what: &str, cost: f64, budget: f64) {
    assert!(
        cost <= budget,
        "{what} costs {cost:.1} bytes a site, over its {budget} — a site lowered its mechanism inline \
         again (a bounds check, a copy-on-write clone, a growth, an overflow check) instead of calling \
         the runtime"
    );
}

#[test]
fn reading_an_item_of_a_global_seq_of_int_costs_a_call() {
    assert_within(
        "`Show item i of A` on a Seq of Int",
        per_site(INTS, |_| "Show item i of A.".into()),
        24.0,
    );
}

#[test]
fn reading_an_item_of_a_global_seq_of_text_costs_a_call() {
    assert_within(
        "`Show item i of A` on a Seq of Text",
        per_site(TEXTS, |_| "Show item i of A.".into()),
        24.0,
    );
}

#[test]
fn writing_an_item_of_a_global_seq_of_int_costs_a_call() {
    assert_within(
        "`Set item i of A to k` on a Seq of Int",
        per_site(INTS, |k| format!("Set item i of A to {k}.")),
        64.0,
    );
}

#[test]
fn writing_an_item_of_a_global_seq_of_text_costs_a_call() {
    // One literal throughout: a new literal is data in the module's text pool, not the site's code.
    assert_within(
        "`Set item i of A to \"t\"` on a Seq of Text",
        per_site(TEXTS, |_| "Set item i of A to \"t\".".into()),
        64.0,
    );
}

#[test]
fn pushing_to_a_global_seq_of_int_costs_a_call() {
    assert_within(
        "`Push k to A` on a Seq of Int",
        per_site(INTS, |k| format!("Push {k} to A.")),
        64.0,
    );
}

#[test]
fn pushing_to_a_global_seq_of_text_costs_a_call() {
    assert_within(
        "`Push \"t\" to A` on a Seq of Text",
        per_site(TEXTS, |_| "Push \"t\" to A.".into()),
        64.0,
    );
}

/// The other side of the trade: inside a loop a read keeps its bounds check and address inline,
/// because a call per element costs a tight loop 9–17% on V8. A read there costs what the inline
/// mechanism costs, not what a call does.
#[test]
fn a_read_inside_a_loop_keeps_its_mechanism_inline() {
    let program = |n: usize| {
        let body: String = (0..n)
            .map(|_| "        Set s to s + item j of A.\n".to_string())
            .collect();
        format!(
            "## To f(i: Int) -> Int:\n    Let mutable s be 0.\n    Let mutable j be 1.\n    While j is at most i:\n{body}        Set j to j + 1.\n    Return s.\n\n\
             ## Main\nLet mutable A be [1, 2, 3].\nShow f(3).\n"
        )
    };
    let cost =
        (size(&program(2 * REPEATS)) as f64 - size(&program(REPEATS)) as f64) / REPEATS as f64;
    let add = {
        let program = |n: usize| {
            let body: String = (0..n)
                .map(|_| "        Set s to s + j.\n".to_string())
                .collect();
            format!(
                "## To f(i: Int) -> Int:\n    Let mutable s be 0.\n    Let mutable j be 1.\n    While j is at most i:\n{body}        Set j to j + 1.\n    Return s.\n\n\
                 ## Main\nLet mutable A be [1, 2, 3].\nShow f(3).\n"
            )
        };
        (size(&program(2 * REPEATS)) as f64 - size(&program(REPEATS)) as f64) / REPEATS as f64
    };
    assert!(
        cost - add >= 30.0,
        "a read of `item j of A` inside a loop costs {:.1} bytes over the add it feeds — a call's \
         cost, not the inline bounds check and address a hot loop keeps",
        cost - add
    );
}

#[test]
fn pushing_to_a_local_list_costs_a_call() {
    // A list a function builds for itself, as the reconciler builds its child lists: no global to
    // take it from, so a site is the growth and the store.
    let program = |n: usize| {
        let body: String = (0..n).map(|k| format!("    Push {k} to l.\n")).collect();
        format!(
            "## To f(i: Int) -> Int:\n    Let mutable l be a new Seq of Int.\n{body}    Return i + length of l.\n\n\
             ## Main\nShow f(1).\n"
        )
    };
    let cost =
        (size(&program(2 * REPEATS)) as f64 - size(&program(REPEATS)) as f64) / REPEATS as f64;
    assert_within("`Push k to l` on a local Seq of Int", cost, 32.0);
}

/// The bytes one more statement costs in a function `f(i)` that says `site` `n` times, straight-line
/// (`in_loop` false) or inside a `While` (`in_loop` true).
fn per_statement(site: &str, in_loop: bool) -> f64 {
    let program = |n: usize| {
        let indent = if in_loop { "        " } else { "    " };
        let body: String = (0..n).map(|_| format!("{indent}{site}\n")).collect();
        let body = if in_loop {
            format!("    Let mutable j be 1.\n    While j is at most i:\n{body}        Set j to j + 1.\n")
        } else {
            body
        };
        // `Main` builds a list, so the module carries the runtime a page always does.
        format!(
            "## To f(i: Int) -> Int:\n    Let mutable s be i.\n{body}    Return s.\n\n## Main\n{INTS}Show f(2).\nShow length of A.\n"
        )
    };
    (size(&program(2 * REPEATS)) as f64 - size(&program(REPEATS)) as f64) / REPEATS as f64
}

/// Outside a loop an Int `+`, `-` or `*` is a call to its checked runtime helper, which traps on
/// signed overflow exactly as the inline check does.
#[test]
fn checked_int_arithmetic_outside_a_loop_costs_a_call() {
    for op in ["+", "-", "*"] {
        let site = format!("Set s to s {op} i.");
        assert_within(
            &format!("`{site}` outside a loop"),
            per_statement(&site, false),
            16.0,
        );
    }
}

/// Inside a loop the overflow check stays inline, where a call would be paid per iteration.
#[test]
fn checked_int_arithmetic_inside_a_loop_keeps_its_check_inline() {
    for op in ["+", "-"] {
        let site = format!("Set s to s {op} i.");
        let cost = per_statement(&site, true);
        assert!(cost >= 20.0, "`{site}` inside a loop costs {cost:.1} bytes — a call's cost, not the inline check a hot loop keeps");
    }
}

/// The bytes one more `site` costs in a function whose body says it `n` times, `prelude` above the
/// function (a callee the site uses).
fn per_line(prelude: &str, site: impl Fn(usize) -> String) -> f64 {
    let program = |n: usize| {
        let body: String = (0..n).map(|k| site(k)).collect();
        format!("{prelude}## To f(i: Int) -> Int:\n{body}    Return i.\n\n## Main\n{INTS}Show f(1).\nShow length of A.\n")
    };
    (size(&program(2 * REPEATS)) as f64 - size(&program(REPEATS)) as f64) / REPEATS as f64
}

/// An empty `Seq`, `Set` or `Map` is a call to `__logos_new_header`, not its 16-byte header
/// allocated, zeroed and stamped inline (each row also shows the collection's length).
#[test]
fn an_empty_collection_costs_a_call() {
    for ty in [
        "a new Seq of Int",
        "a new Set of Int",
        "a new Map of Text to Int",
    ] {
        let cost = per_line("", |k| {
            format!("    Let mutable e{k} be {ty}.\n    Show length of e{k}.\n")
        });
        assert_within(
            &format!("`Let e be {ty}.` then `Show length of e.`"),
            cost,
            24.0,
        );
    }
}

/// Handing a list to a function gives it a copy-on-write holder: a call to `__logos_share` outside
/// a loop, not the header word's read-modify-write inline.
#[test]
fn a_shared_argument_costs_a_call() {
    let cost = per_line(
        "## To g(xs: Seq of Int) -> Int:\n    Return length of xs.\n\n",
        |_| "    Show g(A).\n".into(),
    );
    assert_within("`Show g(A).` on a Seq of Int", cost, 18.0);
}
