//! `Opt::SortNest`: a nest of passes of adjacent compare-exchanges that sorts a window of a sequence of Ints runs as
//! one in-place heap sort.
//!
//! The bubble_sort benchmark sorts 30,000 Ints with `n(n-1)/2` comparisons. Its nest leaves items `1` to `n` in
//! ascending order, as a heap sort does in `O(n log n)`, and a sequence of Ints has one ascending arrangement. The
//! nest becomes `If <the passes suffice and the window lies in the sequence>: <heap sort> Otherwise: <the nest>`, on
//! every spelling of the exchange, ascending or descending, `While` or `Repeat`, a shrinking or a fixed inner range,
//! a window anywhere in the sequence. Each program prints what it printed, on every tier. A nest that sorts only part
//! of its window, touches anything else, sorts Reals, or runs its inner range the other way keeps its loops; one whose
//! window runs off the sequence fails where it failed.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_c_output, assert_compiled_equals_interpreted, compile_to_rust, run_all_tiers};
use logicaffeine_compile::optimization::decorate_source;

fn benchmark(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../benchmarks/programs/{name}/main.lg"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// bubble_sort with `n` in the source instead of on the command line.
fn bubble_sort_at(n: i64) -> String {
    benchmark("bubble_sort").replace(
        "Let n be parseInt(item 2 of arguments).",
        &format!("Let n be {n}."),
    )
}

/// A `Main` that fills `arr` with `n` pseudo-random Ints in `[-50, 49]` (duplicates included), runs `nest`, and shows
/// every item, then `after`.
fn program(n: i64, nest: &str, after: &str) -> String {
    format!(
        "## Main\nLet n be {n}.\nLet arr be a new Seq of Int.\nLet mutable seed be 7.\nLet mutable k be 0.\nWhile k is less than n:\n    Set seed to (seed * 1103515245 + 12345) % 2147483648.\n    Push (seed / 65536) % 100 - 50 to arr.\n    Set k to k + 1.\n{nest}Let mutable shown be \"\".\nRepeat for x in arr:\n    Set shown to shown + x + \" \".\nShow shown.\n{after}"
    )
}

/// The program's generated Rust, and whether its nest became a heap sort.
fn sorts_by_heap(source: &str) -> (bool, String) {
    let rust = compile_to_rust(source).unwrap_or_else(|e| panic!("{e:?}\n{source}"));
    (rust.contains("__sortnest"), rust)
}

/// Every tier prints the same, compiled Rust prints the interpreter's output, and C prints it too.
fn answers_alike(source: &str) {
    let agreed = run_all_tiers(source);
    assert!(agreed.error.is_none(), "{:?}\n{source}", agreed.error);
    assert_c_output(source, agreed.output.trim());
    assert_compiled_equals_interpreted(source);
    let off = decorate_source(source, &["sortnest"]);
    assert_eq!(
        run_all_tiers(&off).output,
        agreed.output,
        "sorted and unsorted print alike:\n{source}"
    );
}

/// The compiled program fails as the interpreter does, after printing what it printed.
fn compiled_fails_alike(source: &str) {
    let compiled = common::run_logos(source);
    let interpreted = logicaffeine_compile::compile::tw_outcome(source);
    let want = interpreted.error.expect("the interpreter fails");
    assert!(
        !compiled.success,
        "the compiled program must fail as the interpreter does ({want}); it printed:\n{}\n\n{}",
        compiled.stdout, compiled.rust_code
    );
    assert_eq!(compiled.stdout.trim(), interpreted.output.trim());
    assert!(
        compiled.stderr.contains(&want),
        "the compiled failure {:?} is not the interpreter's {:?}\n\n{}",
        compiled.stderr,
        want,
        compiled.rust_code
    );
}

const SIZES: [i64; 9] = [0, 1, 2, 3, 4, 7, 16, 65, 200];

#[test]
fn the_bubble_sort_benchmark_sorts_by_heap() {
    let (sorted, rust) = sorts_by_heap(&benchmark("bubble_sort"));
    assert!(
        sorted,
        "the benchmark's nest is one heap sort. Got:\n{rust}"
    );
}

#[test]
fn turning_the_optimization_off_keeps_the_nest() {
    let (sorted, rust) = sorts_by_heap(&decorate_source(&benchmark("bubble_sort"), &["sortnest"]));
    assert!(!sorted, "Got:\n{rust}");
}

/// At every size the benchmark prints alike; with no items it reads `item 1` of an empty sequence and fails alike.
#[test]
fn the_bubble_sort_benchmark_prints_alike_at_every_size() {
    for n in SIZES.into_iter().filter(|&n| n > 0) {
        answers_alike(&bubble_sort_at(n));
    }
    let empty = bubble_sort_at(0);
    assert!(run_all_tiers(&empty).error.is_some(), "item 1 of no items");
    compiled_fails_alike(&empty);
}

/// Nests that sort their whole window: each spelling of the exchange, both orders, both loop forms, a fixed inner
/// range, a window inside the sequence, more passes than needed, bounds read from the length.
fn sorting_nests() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "a swap through one local, inside the test",
            "Let mutable i be 0.\nWhile i is less than n - 1:\n    Let mutable j be 1.\n    While j is at most n - 1 - i:\n        If item j of arr is greater than item (j + 1) of arr:\n            Let t be item j of arr.\n            Set item j of arr to item (j + 1) of arr.\n            Set item (j + 1) of arr to t.\n        Set j to j + 1.\n    Set i to i + 1.\n",
        ),
        (
            "descending",
            "Let mutable i be 0.\nWhile i is less than n - 1:\n    Let mutable j be 1.\n    While j is at most n - 1 - i:\n        Let a be item j of arr.\n        Let b be item (j + 1) of arr.\n        If a is less than b:\n            Set item j of arr to b.\n            Set item (j + 1) of arr to a.\n        Set j to j + 1.\n    Set i to i + 1.\n",
        ),
        (
            "the comparison written the other way round",
            "Let mutable i be 0.\nWhile i is less than n - 1:\n    Let mutable j be 1.\n    While j is at most n - 1 - i:\n        Let a be item j of arr.\n        Let b be item (j + 1) of arr.\n        If b is less than a:\n            Set item (j + 1) of arr to a.\n            Set item j of arr to b.\n        Set j to j + 1.\n    Set i to i + 1.\n",
        ),
        (
            "equal items exchanged too",
            "Let mutable i be 0.\nWhile i is less than n - 1:\n    Let mutable j be 1.\n    While j is at most n - 1 - i:\n        Let a be item j of arr.\n        Let b be item (j + 1) of arr.\n        If a is at least b:\n            Set item j of arr to b.\n            Set item (j + 1) of arr to a.\n        Set j to j + 1.\n    Set i to i + 1.\n",
        ),
        (
            "every pass over the whole window",
            "Let mutable i be 1.\nWhile i is less than n:\n    Let mutable j be 1.\n    While j is less than n:\n        Let a be item j of arr.\n        Let b be item (j + 1) of arr.\n        If a is greater than b:\n            Set item j of arr to b.\n            Set item (j + 1) of arr to a.\n        Set j to j + 1.\n    Set i to i + 1.\n",
        ),
        (
            "Repeat loops",
            "Repeat for i from 1 to n - 1:\n    Repeat for j from 1 to n - i:\n        Let a be item j of arr.\n        Let b be item (j + 1) of arr.\n        If a is greater than b:\n            Set item j of arr to b.\n            Set item (j + 1) of arr to a.\n",
        ),
        (
            "the pair below the counter",
            "Let mutable i be 0.\nWhile i is less than n - 1:\n    Let mutable j be 2.\n    While j is at most n - i:\n        Let a be item (j - 1) of arr.\n        Let b be item j of arr.\n        If a is greater than b:\n            Set item (j - 1) of arr to b.\n            Set item j of arr to a.\n        Set j to j + 1.\n    Set i to i + 1.\n",
        ),
        (
            "a window inside the sequence",
            "Let mutable i be 0.\nWhile i is less than n - 5:\n    Let mutable j be 3.\n    While j is at most n - 3 - i:\n        Let a be item j of arr.\n        Let b be item (j + 1) of arr.\n        If a is greater than b:\n            Set item j of arr to b.\n            Set item (j + 1) of arr to a.\n        Set j to j + 1.\n    Set i to i + 1.\n",
        ),
        (
            "more passes than it needs, bounded by the length",
            "Let size be length of arr.\nLet mutable i be 0.\nWhile i is at most size:\n    Let mutable j be 1.\n    While j is less than size - i:\n        Let a be item j of arr.\n        Let b be item (j + 1) of arr.\n        If a is greater than b:\n            Set item j of arr to b.\n            Set item (j + 1) of arr to a.\n        Set j to j + 1.\n    Set i to i + 1.\n",
        ),
    ]
}

#[test]
fn every_sorting_nest_sorts_by_heap_and_prints_alike() {
    for (why, nest) in sorting_nests() {
        let (sorted, rust) = sorts_by_heap(&program(10, nest, ""));
        assert!(sorted, "{why}: one heap sort. Got:\n{rust}");
        for n in SIZES {
            answers_alike(&program(n, nest, ""));
        }
    }
}

/// The counter of a `While` nest is where the loop leaves it, whether the loop ran or not.
#[test]
fn the_outer_counter_ends_where_the_loop_leaves_it() {
    let exclusive = "Let mutable i be 0.\nWhile i is less than n - 1:\n    Let mutable j be 1.\n    While j is at most n - 1 - i:\n        Let a be item j of arr.\n        Let b be item (j + 1) of arr.\n        If a is greater than b:\n            Set item j of arr to b.\n            Set item (j + 1) of arr to a.\n        Set j to j + 1.\n    Set i to i + 1.\n";
    let inclusive = "Let mutable i be 1.\nWhile i is at most n - 1:\n    Let mutable j be 1.\n    While j is at most n - i:\n        Let a be item j of arr.\n        Let b be item (j + 1) of arr.\n        If a is greater than b:\n            Set item j of arr to b.\n            Set item (j + 1) of arr to a.\n        Set j to j + 1.\n    Set i to i + 1.\n";
    for nest in [exclusive, inclusive] {
        assert!(sorts_by_heap(&program(10, nest, "Show i.\n")).0);
        for n in SIZES {
            answers_alike(&program(n, nest, "Show i.\n"));
        }
    }
}

/// A nest inside a function, sorting a sequence it was handed and returning it.
#[test]
fn a_function_sorts_its_argument() {
    let source = "## To sorted (xs: Seq of Int) -> Seq of Int:\n    Let mutable arr be copy of xs.\n    Let n be length of arr.\n    Let mutable i be 0.\n    While i is less than n - 1:\n        Let mutable j be 1.\n        While j is at most n - 1 - i:\n            Let a be item j of arr.\n            Let b be item (j + 1) of arr.\n            If a is greater than b:\n                Set item j of arr to b.\n                Set item (j + 1) of arr to a.\n            Set j to j + 1.\n        Set i to i + 1.\n    Return arr.\n\n## Main\nLet mutable xs be a new Seq of Int.\nLet mutable seed be 11.\nLet mutable k be 0.\nWhile k is less than 40:\n    Set seed to (seed * 1103515245 + 12345) % 2147483648.\n    Push (seed / 65536) % 30 - 15 to xs.\n    Set k to k + 1.\nLet ys be sorted(xs).\nShow ys.\nShow xs.\nLet mutable none be a new Seq of Int.\nShow sorted(none).\nPush 4 to none.\nShow sorted(none).\n";
    assert!(sorts_by_heap(source).0);
    answers_alike(source);
    let off = format!("## No sortnest\n{source}");
    assert!(!sorts_by_heap(&off).0, "the function opted out");
}

/// Passes counted from the sequence's length: enough of them sort, too few leave the nest's partial order, both
/// alike. The nest is rewritten behind its guard whatever later passes then prove about the guard.
#[test]
fn passes_chosen_at_run_time_take_the_branch_they_allow() {
    let nest = "Let mutable i be 0.\nWhile i is less than passes:\n    Let mutable j be 1.\n    While j is at most n - 1 - i:\n        Let a be item j of arr.\n        Let b be item (j + 1) of arr.\n        If a is greater than b:\n            Set item j of arr to b.\n            Set item (j + 1) of arr to a.\n        Set j to j + 1.\n    Set i to i + 1.\n";
    for passes in [0, 1, 3, 8, 9, 10, 40] {
        let source = program(
            10,
            &format!("Let passes be length of arr - 10 + {passes}.\n{nest}"),
            "Show i.\n",
        );
        assert!(
            logicaffeine_compile::compile::optimizations_fired(&source).contains(&"sortnest"),
            "{passes} passes: the nest is rewritten behind its guard"
        );
        answers_alike(&source);
    }
}

/// Nests outside the shape: each keeps its loops and prints alike.
fn other_nests() -> Vec<(&'static str, String)> {
    let swap = "        Let a be item j of arr.\n        Let b be item (j + 1) of arr.\n        If a is greater than b:\n            Set item j of arr to b.\n            Set item (j + 1) of arr to a.\n";
    vec![
        (
            "one pass short",
            format!("Let mutable i be 0.\nWhile i is less than n - 2:\n    Let mutable j be 1.\n    While j is at most n - 1 - i:\n{swap}        Set j to j + 1.\n    Set i to i + 1.\n"),
        ),
        (
            "an inner range that grows",
            format!("Let mutable i be 0.\nWhile i is less than n - 1:\n    Let mutable j be 1.\n    While j is at most i:\n{swap}        Set j to j + 1.\n    Set i to i + 1.\n"),
        ),
        (
            "an inner range that falls by two",
            format!("Let mutable i be 0.\nWhile i is less than n - 1:\n    Let mutable j be 1.\n    While j is at most n - 1 - 2 * i:\n{swap}        Set j to j + 1.\n    Set i to i + 1.\n"),
        ),
        (
            "an inner start that moves",
            format!("Let mutable i be 0.\nWhile i is less than n - 1:\n    Let mutable j be i + 1.\n    While j is at most n - 1:\n{swap}        Set j to j + 1.\n    Set i to i + 1.\n"),
        ),
        (
            "a count of the exchanges",
            "Let mutable swaps be 0.\nLet mutable i be 0.\nWhile i is less than n - 1:\n    Let mutable j be 1.\n    While j is at most n - 1 - i:\n        Let a be item j of arr.\n        Let b be item (j + 1) of arr.\n        If a is greater than b:\n            Set item j of arr to b.\n            Set item (j + 1) of arr to a.\n            Set swaps to swaps + 1.\n        Set j to j + 1.\n    Set i to i + 1.\nShow swaps.\n".to_string(),
        ),
        (
            "a half exchange",
            "Let mutable i be 0.\nWhile i is less than n - 1:\n    Let mutable j be 1.\n    While j is at most n - 1 - i:\n        Let a be item j of arr.\n        Let b be item (j + 1) of arr.\n        If a is greater than b:\n            Set item j of arr to b.\n            Set item (j + 1) of arr to b.\n        Set j to j + 1.\n    Set i to i + 1.\n".to_string(),
        ),
        (
            "an exchange with an otherwise",
            "Let mutable i be 0.\nWhile i is less than n - 1:\n    Let mutable j be 1.\n    While j is at most n - 1 - i:\n        Let a be item j of arr.\n        Let b be item (j + 1) of arr.\n        If a is greater than b:\n            Set item j of arr to b.\n            Set item (j + 1) of arr to a.\n        Otherwise:\n            Set item j of arr to a.\n        Set j to j + 1.\n    Set i to i + 1.\n".to_string(),
        ),
        (
            "neighbours two apart",
            "Let mutable i be 0.\nWhile i is less than n - 1:\n    Let mutable j be 1.\n    While j is at most n - 2 - i:\n        Let a be item j of arr.\n        Let b be item (j + 2) of arr.\n        If a is greater than b:\n            Set item j of arr to b.\n            Set item (j + 2) of arr to a.\n        Set j to j + 1.\n    Set i to i + 1.\n".to_string(),
        ),
        (
            "an inner counter declared outside the nest",
            format!("Let mutable j be 0.\nLet mutable i be 0.\nWhile i is less than n - 1:\n    Set j to 1.\n    While j is at most n - 1 - i:\n{swap}        Set j to j + 1.\n    Set i to i + 1.\nShow j.\n"),
        ),
        (
            "a bound the nest writes",
            "Let mutable m be n.\nLet mutable i be 0.\nWhile i is less than m - 1:\n    Let mutable j be 1.\n    While j is at most m - 1 - i:\n        Let a be item j of arr.\n        Let b be item (j + 1) of arr.\n        If a is greater than b:\n            Set item j of arr to b.\n            Set item (j + 1) of arr to a.\n        Set j to j + 1.\n    Set i to i + 1.\n    If i equals 3:\n        Set m to m - 1.\n".to_string(),
        ),
    ]
}

#[test]
fn a_nest_outside_the_shape_keeps_its_loops_and_prints_alike() {
    for (why, nest) in other_nests() {
        let (sorted, rust) = sorts_by_heap(&program(10, &nest, ""));
        assert!(!sorted, "{why}: must keep its loops. Got:\n{rust}");
        for n in [0, 1, 2, 5, 16] {
            answers_alike(&program(n, &nest, ""));
        }
    }
}

/// Reals have two zeros a comparison cannot tell apart and an unordered NaN: their nest keeps its loops.
#[test]
fn reals_keep_their_nest() {
    let source = "## Main\nLet arr be [0.0, -0.0, 2.5, -1.5, 0.0, -0.0, 3.25].\nLet n be length of arr.\nLet mutable i be 0.\nWhile i is less than n - 1:\n    Let mutable j be 1.\n    While j is at most n - 1 - i:\n        Let a be item j of arr.\n        Let b be item (j + 1) of arr.\n        If a is greater than b:\n            Set item j of arr to b.\n            Set item (j + 1) of arr to a.\n        Set j to j + 1.\n    Set i to i + 1.\nShow arr.\n";
    let (sorted, rust) = sorts_by_heap(source);
    assert!(!sorted, "Got:\n{rust}");
    answers_alike(source);
}

/// A window past the end of the sequence fails where the nest fails, on every tier: one that starts where the
/// sequence's length puts it, so the guard holds more than the length check, and a window fixed in the source.
#[test]
fn a_window_past_the_end_fails_alike() {
    let swap = "        Let a be item j of arr.\n        Let b be item (j + 1) of arr.\n        If a is greater than b:\n            Set item j of arr to b.\n            Set item (j + 1) of arr to a.\n        Set j to j + 1.\n    Set i to i + 1.\n";
    let fixed = format!("Let mutable i be 0.\nWhile i is less than n:\n    Let mutable j be 1.\n    While j is at most n - i:\n{swap}");
    let moving = format!("Let start be length of arr - 5.\nLet mutable i be 0.\nWhile i is less than n:\n    Let mutable j be start.\n    While j is at most n - i:\n{swap}");
    for nest in [moving, fixed] {
        let source = program(
            6,
            &format!("Show \"before\".\n{nest}"),
            "Show \"unreached\".\n",
        );
        assert!(
            sorts_by_heap(&source).0,
            "the nest is rewritten; its guard is false"
        );
        let agreed = run_all_tiers(&source);
        assert_eq!(agreed.output.trim(), "before");
        assert!(agreed.error.is_some(), "item 7 of a six-item sequence");
        compiled_fails_alike(&source);
    }
}
