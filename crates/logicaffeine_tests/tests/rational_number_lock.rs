//! ★ LOCK — a rational is the VM's exact number on every backend that compiles a program. ★
//!
//! The VM never holds a whole rational: `r * 6` for `r = 1/3` is the `Int` 2, so it divides by `/` as an integer,
//! truncating, takes `%` and `//` and the bitwise operators as one, indexes a list and bounds a loop, and refuses a
//! negative power in an integer's words. A fraction stays exact and refuses `%` and the bitwise operators in its own
//! words. A compiled program holds a rational in its machine form, so that form keeps these rules: a whole value
//! acts as its integer, wherever the rational is held. That is a binding the checker types `Rational`, an item of
//! a list, and a variable that held an integer before it gained a fraction (whose `/` truncates until it does).
//! Every program here runs as the VM runs it: a list of rationals shown, indexed from either end, sliced, walked,
//! searched, grown, popped (in a function, and empty), filtered, stored into, sorted by swaps, copied, compared,
//! nested and passed through a function; a record's rational field; a whole item bounding a run or naming a place;
//! and each rule of a whole value and of a fraction, failures included. A record built without its rational field
//! holds nothing on the VM, which a compiled rational cannot hold: each compiled backend refuses that by name.

#![cfg(not(target_arch = "wasm32"))]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

use tier_parity::{Backend, Program};

const PROGRAMS: &[(&str, &str)] = &[
    (
        "shown_and_read",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet ys be [r, r + 1].\nShow ys.\nShow item 1 of ys.\nLet z be item 2 of ys.\nShow z * 3.\n",
    ),
    (
        "searched_grown_and_walked",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet mutable ys be [r, r + 1].\nShow ys.\nShow item 2 of ys.\nShow length of ys.\nShow ys contains r.\nShow ys contains r * 5.\nPush r * 2 to ys.\nShow ys.\nRepeat for y in ys:\n    Show y * 3.\n",
    ),
    (
        "declared_and_stored_into",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet mutable xs: Seq of Rational be a new Seq of Rational.\nPush r to xs.\nPush r * 3 to xs.\nShow xs.\nLet mutable total: Rational be 0.\nRepeat for x in xs:\n    Set total to total + x.\nShow total.\nSet item 1 of xs to r / 2.\nShow xs.\n",
    ),
    (
        "made_with_room",
        "## Main\nLet mutable xs be a new Seq of Rational with capacity 4.\nRepeat for i from 1 to parseInt(\"3\"):\n    Let q: Rational be i / 3.\n    Push q to xs.\nShow xs.\nShow length of xs.\n",
    ),
    (
        "a_whole_item_divides_as_its_integer",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet ys be [r * 6, r].\nShow ys.\nShow item 1 of ys / 4.\nShow item 2 of ys / 4.\nShow ys contains 2.\n",
    ),
    (
        "an_integer_beside_a_rational",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet xs be [1, r].\nShow xs.\nShow item 1 of xs / 2.\nShow xs contains 1.\n",
    ),
    (
        "sliced_popped_and_filtered",
        "## Main\nLet r: Rational be 1 / parseInt(\"4\").\nLet mutable ys be [r, r * 2, r * 3, r * 4, r * 5].\nShow items 2 through 4 of ys.\nPop from ys into last.\nShow last.\nShow ys.\nLet small be every q in ys where q is less than r * 2.\nShow small.\nRemove every q from ys where q is greater than r * 2.\nShow ys.\nShow item -1 of ys.\n",
    ),
    (
        "sorted_by_swaps",
        "## Main\nLet r: Rational be 1 / parseInt(\"7\").\nLet mutable ys be [r * 5, r, r * 3, r * 2].\nRepeat for i from 1 to 3:\n    Repeat for j from 1 to 4 - i:\n        If item j of ys is greater than item (j + 1) of ys:\n            Let t be item j of ys.\n            Set item j of ys to item (j + 1) of ys.\n            Set item (j + 1) of ys to t.\nShow ys.\n",
    ),
    (
        "compared_with_another",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet a be [r, r * 2].\nLet mutable b be a new Seq of Rational.\nPush r to b.\nPush r + r to b.\nShow a = b.\nPush r to b.\nShow a = b.\n",
    ),
    (
        "nested",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet m be [[r, r * 2], [r * 3]].\nShow m.\nShow item 2 of item 1 of m.\nShow item 1 of item 2 of m / 2.\n",
    ),
    (
        "through_a_function",
        "## To total (xs: Seq of Rational) -> Rational:\n    Let mutable s: Rational be 0.\n    Repeat for x in xs:\n        Set s to s + x.\n    Return s.\n\n## To halves (n: Int) -> Seq of Rational:\n    Let mutable out: Seq of Rational be a new Seq of Rational.\n    Repeat for i from 1 to n:\n        Let q: Rational be i / 2.\n        Push q to out.\n    Return out.\n\n## Main\nLet h be halves(parseInt(\"4\")).\nShow h.\nShow total(h).\nLet zs be copy of h.\nShow zs = h.\nShow item -1 of h.\n",
    ),
    (
        "a_rational_parameter_given_an_integer",
        "## To half (q: Rational) -> Rational:\n    Return q / 2.\n\n## To scaled (xs: Seq of Rational) and (k: Int) -> Seq of Rational:\n    Let mutable out: Seq of Rational be a new Seq of Rational.\n    Repeat for x in xs:\n        Push x * k to out.\n    Return out.\n\n## Main\nShow half(parseInt(\"3\")).\nShow half(1 / parseInt(\"3\")).\nShow scaled([parseInt(\"1\"), parseInt(\"2\")], parseInt(\"3\")).\nLet ys be scaled([parseInt(\"4\")], parseInt(\"1\")).\nShow item 1 of ys / 8.\n",
    ),
    (
        "a_rational_result_made_from_an_integer",
        "## To three -> Rational:\n    Return parseInt(\"3\").\n\n## Main\nLet t be three().\nShow t / 2.\nShow t.\nShow t % 2.\n",
    ),
    (
        "a_counter_gains_a_fraction",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet mutable t be parseInt(\"0\").\nSet t to t + r.\nShow t.\nSet t to t + r * 2.\nShow t.\nShow t / 2.\n",
    ),
    (
        "a_counter_divides_as_an_integer_until_its_fraction",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet mutable t be parseInt(\"5\").\nShow t / 2.\nSet t to t + r.\nShow t.\nShow t / 2.\n",
    ),
    (
        "whole_items_take_integer_operations",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet ys be [r * 21, r * 6].\nLet a be 0 - item 1 of ys.\nLet b be item 2 of ys.\nShow a / b.\nShow a % b.\nShow a // b.\nShow b / (0 - 4).\nShow a ^ 3.\nShow b ** 3.\nLet xs be [10, 20, 30].\nShow item b of xs.\nRepeat for i from 1 to b:\n    Show i.\n",
    ),
    (
        "a_fractional_item_stays_exact",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet ys be [r, r * 6].\nLet a be item 1 of ys.\nShow a // 1.\nShow 7 // a.\nShow a / 2.\nShow a ** 2.\nShow a ** (0 - 2).\nShow a + 1.5.\nShow a < 0.5.\n",
    ),
    (
        "a_declared_rational_that_is_whole",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet q: Rational be r * 3.\nShow q.\nShow q / 4.\nShow q % 2.\nShow q // 2.\nShow q ** 2.\nShow q ** (0 - 1).\n",
    ),
    (
        "a_fractional_item_refuses_a_remainder",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet ys be [r].\nShow \"before\".\nShow item 1 of ys % 2.\n",
    ),
    (
        "a_declared_fraction_refuses_a_remainder",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nShow \"before\".\nShow r % 2.\n",
    ),
    (
        "a_whole_item_refuses_a_negative_power",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet ys be [r * 6].\nShow \"before\".\nShow (item 1 of ys) ** (0 - 1).\n",
    ),
    (
        "a_whole_item_divided_by_zero",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet ys be [r * 6, r * 0].\nShow \"before\".\nShow item 1 of ys / item 2 of ys.\n",
    ),
    (
        "a_whole_item_takes_a_remainder_by_zero",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet ys be [r * 6].\nShow \"before\".\nShow 5 % (item 1 of ys - 2).\n",
    ),
    (
        "a_fractional_item_refuses_a_bitwise_operation",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet ys be [r].\nShow \"before\".\nShow item 1 of ys ^ 3.\n",
    ),
    (
        "a_counter_halved_before_it_gains_a_fraction",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet mutable t be parseInt(\"7\").\nShow t / 2.\nIf parseInt(\"1\") is 1:\n    Set t to t + r.\nShow t.\nShow t / 2.\n",
    ),
    (
        "whole_items_bound_a_run_and_name_a_place",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet ys be [r * 3, r * 6].\nLet xs be [10, 20, 30, 40].\nShow items (item 1 of ys) through (item 2 of ys) of xs.\nLet mutable zs be [5, 6, 7].\nSet item (item 2 of ys) of zs to 99.\nShow zs.\n",
    ),
    (
        "a_fraction_refuses_to_bound_a_run",
        "## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet xs be [10, 20, 30, 40].\nShow \"before\".\nShow items r through 2 of xs.\n",
    ),
    (
        "a_fraction_refuses_to_name_a_place",
        "## Main\nLet r: Rational be 1 / parseInt(\"2\").\nLet mutable zs be [5, 6, 7].\nShow \"before\".\nSet item r of zs to 1.\nShow zs.\n",
    ),
    (
        "popped_in_a_function",
        "## To lastHalf (xs: Seq of Rational) -> Rational:\n    Let mutable ys be copy of xs.\n    Pop from ys into last.\n    Return last / 2.\n\n## Main\nLet r: Rational be 1 / parseInt(\"3\").\nShow lastHalf([r, r * 15]).\nShow lastHalf([r, r * 2]).\n",
    ),
    (
        "a_popped_nothing_refuses",
        "## Main\nLet mutable zs be a new Seq of Rational.\nPop from zs into top.\nShow top.\nShow \"before\".\nShow top / parseInt(\"2\").\n",
    ),
    (
        "a_field_holds_a_rational",
        "## A Point has:\n    an x, which is Rational.\n\n## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet p be a new Point with x (r * 6).\nShow p's x.\nShow p's x / 4.\nLet q be a new Point with x parseInt(\"5\").\nShow q's x / 2.\nShow q's x + r.\n",
    ),
];

/// A record built without its rational field: the VM holds nothing there, which a compiled rational cannot hold.
const UNSET_FIELD: &str = "## A Point has:\n    an x, which is Rational.\n\n## Main\nLet r: Rational be 1 / parseInt(\"3\").\nShow r.\nLet e be a new Point.\nShow e's x.\nShow e's x + r.\n";

/// The words every compiled backend refuses [`UNSET_FIELD`] in.
const HOLDS_NO_NOTHING: &str = "holds no nothing";

/// The backends that compile a program to run it: the engines, the generated Rust, both C lowerings, linked
/// WebAssembly and RISC-V.
const BACKENDS: &[Backend] = &[
    Backend::TreeWalker,
    Backend::VmOptimized,
    Backend::VmForge,
    Backend::Interpret,
    Backend::RustDebug,
    Backend::RustRelease,
    Backend::C,
    Backend::CBytecode,
    Backend::WasmLinked,
    Backend::RiscV,
];

/// The child side of every isolated backend run. Does nothing unless `LOGOS_PARITY_JOB` is set.
#[test]
#[ignore = "the worker the parity sweep re-executes this binary into"]
fn tier_parity_worker() {
    tier_parity::worker_main();
}

/// Declares what reads a program's numbers at run time, so that no tier folds them into constants.
const NATIVE: &str = "## To native parseInt (s: Text) -> Int\n\n";

/// `source` with each number it reads at run time written as the literal it reads, which the optimizing tiers
/// fold.
fn folded(source: &str) -> String {
    let mut out = String::new();
    let mut rest = source;
    while let Some(at) = rest.find("parseInt(\"") {
        out.push_str(&rest[..at]);
        let digits = &rest[at + "parseInt(\"".len()..];
        let end = digits.find("\")").expect("every parseInt here closes");
        out.push_str(&digits[..end]);
        rest = &digits[end + 2..];
    }
    out.push_str(rest);
    out
}

/// Each program twice: reading its numbers at run time, and folded.
fn programs() -> Vec<Program> {
    PROGRAMS
        .iter()
        .flat_map(|(id, source)| {
            [
                Program::new(format!("rational/{id}"), format!("{NATIVE}{source}")),
                Program::new(format!("rational/{id}/folded"), folded(source)),
            ]
        })
        .collect()
}

#[test]
fn every_program_here_reaches_the_backends() {
    let refused: Vec<String> = programs()
        .into_iter()
        .filter(|p| tier_parity::rejection(&p.source).is_some())
        .map(|p| p.id)
        .collect();
    assert!(refused.is_empty(), "the front end refuses {refused:?}, which pin nothing");
}

#[test]
fn every_program_here_reads_its_numbers_at_run_time_and_its_twin_folds_them() {
    for (id, source) in PROGRAMS {
        assert!(source.contains("parseInt(\""), "{id} reads no number at run time, so a tier may fold it whole");
        assert!(!folded(source).contains("parseInt"), "{id}'s folded twin still reads a number at run time");
    }
}

fn differ(backends: &[Backend]) -> Vec<String> {
    let rows = tier_parity::sweep(&programs(), backends, 4).unwrap_or_else(|why| panic!("the sweep could not start: {why}"));
    rows.iter()
        .filter(|row| !row.matches())
        .map(|row| format!("{} on {}: {} {}", row.program, row.backend.name(), row.status(), row.detail()))
        .collect()
}

#[test]
fn every_backend_holds_a_rational_as_the_vm_does() {
    let bad = differ(BACKENDS);
    assert!(bad.is_empty(), "{} runs differ from the VM:\n{}", bad.len(), bad.join("\n"));
}

#[test]
fn a_record_built_without_its_rational_field_holds_nothing_or_is_refused_by_name() {
    let programs = [
        Program::new("rational/unset_field".to_string(), format!("{NATIVE}{UNSET_FIELD}")),
        Program::new("rational/unset_field/folded".to_string(), folded(UNSET_FIELD)),
    ];
    let rows = tier_parity::sweep(&programs, BACKENDS, 4).unwrap_or_else(|why| panic!("the sweep could not start: {why}"));
    let wrong: Vec<String> = rows
        .iter()
        .filter(|row| !row.matches() && !(row.status() == "REFUSED" && row.detail().contains(HOLDS_NO_NOTHING)))
        .map(|row| format!("{} on {}: {} {}", row.program, row.backend.name(), row.status(), row.detail()))
        .collect();
    assert!(wrong.is_empty(), "a backend gave a record's missing rational a value:\n{}", wrong.join("\n"));
    assert!(
        rows.iter().any(|row| row.backend == Backend::TreeWalker && row.matches()),
        "the reference itself must run the program"
    );
}
