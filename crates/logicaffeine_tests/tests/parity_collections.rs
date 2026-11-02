//! Stdlib-parity north-star corpus - domain: collections
//!
//! Higher-level collection operations (collections.lg). A Counter tallies element frequencies into
//! the built-in Map, byte-identical on every tier. Mirrors Python's collections.Counter.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;

/// Tally element frequencies into a Counter. py: collections.Counter
#[test]
fn parity_collections_counter() {
    assert_all_tiers(
        "## Main\nLet c be countInts([1, 2, 2, 3, 2, 1]).\nShow counterGet(c, 2).\nShow counterGet(c, 3).\n",
        "3\n1",
    );
}

/// Look up a count, defaulting to 0 for absent keys. py: collections.Counter (__missing__)
#[test]
fn parity_collections_counter_get() {
    assert_all_tiers(
        "## Main\nLet c be countInts([5, 5, 7]).\nShow counterGet(c, 5).\nShow counterGet(c, 99).\n",
        "2\n0",
    );
}

/// Total of all counts. py: collections.Counter.total
#[test]
fn parity_collections_counter_total() {
    assert_all_tiers(
        "## Main\nShow counterTotal(countInts([1, 2, 2, 3, 2, 1])).\n",
        "6",
    );
}

/// Most frequent element. py: collections.Counter.most_common
#[test]
fn parity_collections_counter_most_common() {
    assert_all_tiers(
        "## Main\nShow counterMostCommon(countInts([1, 2, 2, 3, 2, 1])).\n",
        "2",
    );
}

/// A fresh empty deque. py: collections.deque
#[test]
fn parity_collections_deque_new() {
    assert_all_tiers("## Main\nShow dequeLen(newDeque()).\n", "0");
}

/// Append to the back. py: collections.deque.append
#[test]
fn parity_collections_deque_push_back() {
    assert_all_tiers("## Main\nLet d be dequePushBack(dequePushBack(newDeque(), 1), 2).\nShow dequeBack(d).\nShow dequeLen(d).\n", "2\n2");
}

/// Prepend to the front. py: collections.deque.appendleft
#[test]
fn parity_collections_deque_push_front() {
    assert_all_tiers(
        "## Main\nLet d be dequePushFront(dequePushBack(newDeque(), 1), 9).\nShow dequeFront(d).\n",
        "9",
    );
}

/// Pop from the front. py: collections.deque.popleft
#[test]
fn parity_collections_deque_pop_front() {
    assert_all_tiers("## Main\nLet d be dequePushBack(dequePushBack(newDeque(), 1), 2).\nShow dequeFront(dequePopFront(d)).\n", "2");
}

/// Peek front and back. py: collections.deque
#[test]
fn parity_collections_deque_peek() {
    assert_all_tiers("## Main\nLet d be dequePushBack(dequePushBack(dequePushBack(newDeque(), 5), 6), 7).\nShow dequeFront(d).\nShow dequeBack(d).\n", "5\n7");
}

/// The keys of a map. py: builtins.dict.keys
#[test]
fn parity_collections_dict_keys() {
    assert_all_tiers(
        "## Main\nLet m be a new Map of Int to Int.\nSet item 1 of m to 10.\nSet item 2 of m to 20.\nShow length of dictKeys(m).\nShow sumSeq(dictKeys(m)).\n",
        "2\n3",
    );
}

/// The values of a map. py: builtins.dict.values
#[test]
fn parity_collections_dict_values() {
    assert_all_tiers(
        "## Main\nLet m be a new Map of Int to Int.\nSet item 1 of m to 10.\nSet item 2 of m to 20.\nShow length of dictValues(m).\nShow sumSeq(dictValues(m)).\n",
        "2\n30",
    );
}
