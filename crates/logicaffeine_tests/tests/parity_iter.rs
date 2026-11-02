//! Stdlib-parity north-star corpus — domain: iter
//!
//! Auto-scaffolded from benchmarks/parity/catalog/superset.json by
//! scripts/bench/gen-parity-corpus.py. Each test mirrors one deduplicated capability of the
//! C/Python/Rust/Go (+Java) superset. `#[ignore]` = parity-pending (not yet implemented);
//! the tree stays green. To LAND a capability: implement it, replace the body with a real
//! `assert_all_tiers` behavioral spec, DELETE the `#[ignore]` line, flip the catalog entry
//! to `status: green`, and bump GREEN_FLOOR in parity_catalog_lock.rs.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;

/// Whether every element satisfies a predicate.
/// provenance — python: builtins.all · rust: Iterator.all · java: java.util.stream.Stream.allMatch
/// LOGOS surface: `all of xs satisfy p`
#[test]
fn parity_iter_all() {
    assert_all_tiers(
        r#"## Main
Show allSeq((n: Int) -> n is greater than 0, [1, 2, 3]).
Show allSeq((n: Int) -> n is greater than 1, [1, 2, 3]).
"#,
        "true
false",
    );
}

/// Whether any element satisfies a predicate.
/// provenance — python: builtins.any · rust: Iterator.any · java: java.util.stream.Stream.anyMatch
/// LOGOS surface: `any of xs satisfies p`
#[test]
fn parity_iter_any() {
    assert_all_tiers(
        r#"## Main
Show anySeq((n: Int) -> n is greater than 3, [1, 2, 3, 4]).
Show anySeq((n: Int) -> n is greater than 9, [1, 2, 3]).
"#,
        "true
false",
    );
}

/// Pair each element with its index.
/// provenance — python: builtins.enumerate · rust: Iterator.enumerate
/// LOGOS surface: `xs with indices`
#[test]
fn parity_iter_enumerate() {
    assert_all_tiers(
        "## Main
Let e be enumerateSeq([10, 20, 30]).
Let p be item 2 of e.
Show p's first.
Show p's second.
",
        "2
20",
    );
}

/// Keep only elements satisfying a predicate.
/// provenance — python: builtins.filter · rust: Iterator.filter · java: java.util.stream.Stream.filter
/// LOGOS surface: `the elements of xs where p holds`
#[test]
fn parity_iter_filter() {
    assert_all_tiers(
        r#"## Main
Let r be filterSeq((n: Int) -> n is greater than 2, [1, 2, 3, 4]).
Show length of r.
Show item 1 of r.
"#,
        "2
3",
    );
}

/// Apply a function to each element, producing a new sequence.
/// provenance — python: builtins.map · rust: Iterator.map · java: java.util.stream.Stream.map
/// LOGOS surface: `map f over xs`
#[test]
fn parity_iter_map() {
    assert_all_tiers(
        r#"## Main
Let r be mapSeq((n: Int) -> n * 2, [1, 2, 3]).
Show item 1 of r.
Show item 3 of r.
"#,
        "2
6",
    );
}

/// Largest element of a sequence.
/// provenance — python: builtins.max · rust: Iterator.max · go: slices.Max · java: java.util.Collections.max
/// LOGOS surface: `the maximum of xs`
#[test]
fn parity_iter_max() {
    assert_all_tiers(
        r#"## Main
Show maxSeq([3, 1, 2]).
"#,
        "3",
    );
}

/// Smallest element of a sequence.
/// provenance — python: builtins.min · rust: Iterator.min · go: slices.Min · java: java.util.Collections.min
/// LOGOS surface: `the minimum of xs`
#[test]
fn parity_iter_min() {
    assert_all_tiers(
        r#"## Main
Show minSeq([3, 1, 2]).
"#,
        "1",
    );
}

/// Fold a sequence to a single value with an accumulator.
/// provenance — python: functools.reduce · rust: Iterator.fold · java: java.util.stream.Stream.reduce
/// LOGOS surface: `fold xs from init with f`
#[test]
fn parity_iter_reduce() {
    assert_all_tiers(
        r#"## Main
Show reduceSeq((acc: Int, x: Int) -> acc + x, 0, [1, 2, 3, 4]).
"#,
        "10",
    );
}

/// Reverse the order of a sequence.
/// provenance — python: builtins.reversed · rust: Iterator.rev · go: slices.Reverse · java: java.util.Collections.reverse
/// LOGOS surface: `xs reversed`
#[test]
fn parity_iter_reverse() {
    assert_all_tiers(
        r#"## Main
Let r be reverseSeq([1, 2, 3]).
Show item 1 of r.
Show item 3 of r.
"#,
        "3
1",
    );
}

/// Stably sort a sequence into ascending order.
/// provenance — c: stdlib.h.qsort · python: builtins.sorted · rust: Vec.sort · go: sort.Slice · java: java.util.Collections.sort
/// LOGOS surface: `xs sorted`
#[test]
fn parity_iter_sort() {
    assert_all_tiers(
        r#"## Main
Let s be sortSeq([3, 1, 2]).
Show item 1 of s.
Show item 2 of s.
Show item 3 of s.
"#,
        "1
2
3",
    );
}

/// Sum the elements of a numeric sequence.
/// provenance — python: builtins.sum · rust: Iterator.sum
/// LOGOS surface: `the sum of xs`
#[test]
fn parity_iter_sum() {
    assert_all_tiers(
        r#"## Main
Show sumSeq([1, 2, 3, 4]).
"#,
        "10",
    );
}

/// Pair up elements of two sequences.
/// provenance — python: builtins.zip · rust: Iterator.zip
/// LOGOS surface: `xs zipped with ys`
#[test]
fn parity_iter_zip() {
    assert_all_tiers(
        "## Main
Let z be zipSeq([1, 2, 3], [4, 5, 6]).
Let p be item 1 of z.
Show p's first.
Show p's second.
",
        "1
4",
    );
}

/// Deduplicate keeping first-seen order. py: (dict.fromkeys) · rust: Iterator.dedup
#[test]
fn parity_iter_unique() {
    assert_all_tiers("## Main\nLet u be uniqueSeq([1, 2, 2, 3, 1]).\nShow length of u.\nShow item 1 of u.\nShow item 3 of u.\n", "3\n1\n3");
}
/// First n elements. rust: Iterator.take · py: itertools.islice
#[test]
fn parity_iter_take() {
    assert_all_tiers(
        "## Main\nLet t be takeSeq(2, [10, 20, 30]).\nShow length of t.\nShow item 2 of t.\n",
        "2\n20",
    );
}
/// Skip the first n elements. rust: Iterator.skip
#[test]
fn parity_iter_drop() {
    assert_all_tiers(
        "## Main\nLet d be dropSeq(1, [10, 20, 30]).\nShow length of d.\nShow item 1 of d.\n",
        "2\n20",
    );
}
/// Membership. rust: slice.contains · go: slices.Contains
#[test]
fn parity_iter_contains() {
    assert_all_tiers(
        "## Main\nShow containsSeq(20, [10, 20, 30]).\nShow containsSeq(99, [10, 20, 30]).\n",
        "true\nfalse",
    );
}
/// Product of a sequence. rust: Iterator.product · py: math.prod
#[test]
fn parity_iter_product() {
    assert_all_tiers("## Main\nShow productSeq([1, 2, 3, 4]).\n", "24");
}

/// Inclusive integer range. py: range · rust: RangeInclusive
#[test]
fn parity_iter_range() {
    assert_all_tiers(
        "## Main\nLet r be rangeSeq(1, 5).\nShow length of r.\nShow item 3 of r.\n",
        "5\n3",
    );
}
/// Repeat a value n times. rust: vec![x; n] · go: slices.Repeat
#[test]
fn parity_iter_repeat_val() {
    assert_all_tiers(
        "## Main\nLet r be repeatVal(7, 3).\nShow length of r.\nShow item 2 of r.\n",
        "3\n7",
    );
}
/// First element. rust: Iterator.next · py: seq[0]
#[test]
fn parity_iter_first() {
    assert_all_tiers("## Main\nShow firstOf([10, 20, 30]).\n", "10");
}
/// Last element. rust: Iterator.last · py: seq[-1]
#[test]
fn parity_iter_last() {
    assert_all_tiers("## Main\nShow lastOf([10, 20, 30]).\n", "30");
}

/// Concatenate two sequences. rust: Iterator.chain · py: itertools.chain
#[test]
fn parity_iter_concat() {
    assert_all_tiers(
        "## Main\nLet r be concatSeq([1, 2], [3, 4]).\nShow length of r.\nShow item 3 of r.\n",
        "4\n3",
    );
}
/// Subrange lo..hi (inclusive). py: seq[a:b] · rust: slice range
#[test]
fn parity_iter_slice() {
    assert_all_tiers("## Main\nLet r be sliceSeq([10, 20, 30, 40], 2, 3).\nShow length of r.\nShow item 1 of r.\n", "2\n20");
}
/// Position of first occurrence (0 if none). py: list.index · go: slices.Index
#[test]
fn parity_iter_index_of() {
    assert_all_tiers(
        "## Main\nShow indexOfInt(30, [10, 20, 30]).\nShow indexOfInt(99, [10, 20, 30]).\n",
        "3\n0",
    );
}
/// Count occurrences of a value. py: list.count · go: slices.Contains
#[test]
fn parity_iter_count_of() {
    assert_all_tiers("## Main\nShow countOf(2, [1, 2, 2, 3, 2]).\n", "3");
}

/// The lower of the two middle values (no interpolation). py: statistics.median_low
#[test]
fn parity_iter_median_low() {
    assert_all_tiers(
        "## Main\nShow medianLow([3, 1, 2]).\nShow medianLow([4, 1, 3, 2]).\n",
        "2\n2",
    );
}

/// The higher of the two middle values. py: statistics.median_high
#[test]
fn parity_iter_median_high() {
    assert_all_tiers(
        "## Main\nShow medianHigh([3, 1, 2]).\nShow medianHigh([4, 1, 3, 2]).\n",
        "2\n3",
    );
}

/// The most frequent value (first-seen wins ties). py: statistics.mode
#[test]
fn parity_iter_mode() {
    assert_all_tiers(
        "## Main\nShow modeSeq([1, 2, 2, 3, 2, 1]).\nShow modeSeq([5, 5, 7]).\n",
        "2\n5",
    );
}

/// The span between the largest and smallest elements (max - min).
#[test]
fn parity_iter_range_stat() {
    assert_all_tiers("## Main\nShow rangeStat([5, 1, 9, 3]).\n", "8");
}

/// Leftmost insertion index in a sorted sequence. py: bisect.bisect_left · go: sort.SearchInts
#[test]
fn parity_iter_bisect_left() {
    assert_all_tiers(
        "## Main\nShow bisectLeft([1, 3, 3, 5, 7], 3).\nShow bisectLeft([1, 3, 5], 4).\n",
        "2\n3",
    );
}

/// Rightmost insertion index in a sorted sequence. py: bisect.bisect_right
#[test]
fn parity_iter_bisect_right() {
    assert_all_tiers("## Main\nShow bisectRight([1, 3, 3, 5, 7], 3).\n", "4");
}

/// Insert into a sorted sequence keeping it sorted. py: bisect.insort
#[test]
fn parity_iter_insort() {
    assert_all_tiers(
        "## Main\nShow insort([1, 3, 5], 4).\nShow insort([2, 4, 6], 1).\n",
        "[1, 3, 4, 5]\n[1, 2, 4, 6]",
    );
}

/// Take elements while a predicate holds, then stop. rust: Iterator.take_while · py: itertools.takewhile
#[test]
fn parity_iter_take_while() {
    assert_all_tiers("## Main\nLet t be takeWhileSeq((n: Int) -> n is less than 3, [1, 2, 3, 4, 1]).\nShow length of t.\nShow item 2 of t.\n", "2\n2");
}
/// Drop elements while a predicate holds, then take the rest. rust: Iterator.skip_while · py: itertools.dropwhile
#[test]
fn parity_iter_drop_while() {
    assert_all_tiers("## Main\nLet d be dropWhileSeq((n: Int) -> n is less than 3, [1, 2, 3, 4, 1]).\nShow length of d.\nShow item 1 of d.\n", "3\n3");
}
/// Position (1-based) of the first element satisfying a predicate, else 0. rust: Iterator.find / position
#[test]
fn parity_iter_find() {
    assert_all_tiers("## Main\nShow findSeq((n: Int) -> n is greater than 2, [1, 2, 3, 4]).\nShow findSeq((n: Int) -> n is greater than 9, [1, 2]).\n", "3\n0");
}
/// Insert a value before a position. rust: Vec.insert · go: slices.Insert
#[test]
fn parity_iter_insert_at() {
    assert_all_tiers("## Main\nLet r be insertAt([1, 2, 4], 3, 3).\nShow length of r.\nShow item 3 of r.\nShow item 4 of r.\n", "4\n3\n4");
}
/// Remove the element at a position. rust: Vec.remove · go: slices.Delete
#[test]
fn parity_iter_remove_at() {
    assert_all_tiers(
        "## Main\nLet r be removeAt([1, 2, 3, 4], 2).\nShow length of r.\nShow item 2 of r.\n",
        "3\n3",
    );
}
/// Rotate left by n (first n move to the end). rust: Vec.rotate_left
#[test]
fn parity_iter_rotate_left() {
    assert_all_tiers("## Main\nLet r be rotateLeftSeq([1, 2, 3, 4, 5], 2).\nShow item 1 of r.\nShow item 5 of r.\n", "3\n2");
}
/// Rotate right by n (last n move to the front). rust: Vec.rotate_right
#[test]
fn parity_iter_rotate_right() {
    assert_all_tiers(
        "## Main\nLet r be rotateRightSeq([1, 2, 3, 4, 5], 2).\nShow item 1 of r.\n",
        "4",
    );
}
/// Take every nth element starting from the first. rust: Iterator.step_by · py: itertools.islice
#[test]
fn parity_iter_step_by() {
    assert_all_tiers("## Main\nLet r be stepBySeq([1, 2, 3, 4, 5, 6], 2).\nShow length of r.\nShow item 2 of r.\n", "3\n3");
}

/// Swap two elements by position. rust: Vec.swap
#[test]
fn parity_iter_swap() {
    assert_all_tiers(
        "## Main\nLet r be swapSeq([1, 2, 3, 4], 2, 4).\nShow item 2 of r.\nShow item 4 of r.\n",
        "4\n2",
    );
}
/// Keep only the first n elements. rust: Vec.truncate
#[test]
fn parity_iter_truncate() {
    assert_all_tiers("## Main\nLet r be truncateSeq([1, 2, 3, 4, 5], 3).\nShow length of r.\nShow item 3 of r.\n", "3\n3");
}
/// Whether a sequence is empty. rust: Vec.is_empty
#[test]
fn parity_iter_is_empty() {
    assert_all_tiers(
        "## Main\nShow isEmptySeq([1]).\nLet e be a new Seq of Int.\nShow isEmptySeq(e).\n",
        "false\ntrue",
    );
}
/// Bounds-checked read with a default. rust: Vec.get
#[test]
fn parity_iter_get_or() {
    assert_all_tiers(
        "## Main\nShow getOr([10, 20, 30], 2, -1).\nShow getOr([10, 20, 30], 9, -1).\n",
        "20\n-1",
    );
}
/// Binary search a sorted sequence (1-based position, else 0). rust: Vec.binary_search · go: slices.BinarySearch
#[test]
fn parity_iter_binary_search() {
    assert_all_tiers(
        "## Main\nShow binarySearchSeq([1, 3, 5, 7, 9], 7).\nShow binarySearchSeq([1, 3, 5], 4).\n",
        "4\n0",
    );
}
/// Remove by swapping with the last element (order not preserved). rust: Vec.swap_remove
#[test]
fn parity_iter_swap_remove() {
    assert_all_tiers(
        "## Main\nLet r be swapRemove([1, 2, 3, 4], 2).\nShow length of r.\nShow item 2 of r.\n",
        "3\n4",
    );
}
/// Remove consecutive duplicates. rust: Vec.dedup · go: slices.Compact
#[test]
fn parity_iter_dedup() {
    assert_all_tiers("## Main\nLet r be dedupSeq([1, 1, 2, 3, 3, 3, 1]).\nShow length of r.\nShow item 4 of r.\n", "4\n1");
}
/// Count elements matching a predicate. rust: Iterator.filter+count
#[test]
fn parity_iter_count_where() {
    assert_all_tiers(
        "## Main\nShow countWhere((n: Int) -> n is greater than 2, [1, 2, 3, 4, 5]).\n",
        "3",
    );
}
