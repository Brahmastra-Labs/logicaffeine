//! Stdlib-parity north-star corpus - domain: heap
//!
//! A binary min-heap over a 1-indexed `Seq of Int` (heap.lg), written in pure LOGOS and
//! byte-identical on every tier. Mirrors Python's heapq, Go's container/heap, and Rust's
//! std::collections::BinaryHeap.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;

/// A fresh empty heap. rust: BinaryHeap::new
#[test]
fn parity_heap_new() {
    assert_all_tiers(
        "## Main\nLet h be heapNew().\nShow heapIsEmpty(h).\n",
        "true",
    );
}

/// Number of elements in the heap. rust: BinaryHeap::len
#[test]
fn parity_heap_len() {
    assert_all_tiers(
        "## Main\nLet mutable h be heapNew().\nSet h to heapPush(h, 5).\nSet h to heapPush(h, 3).\nShow heapLen(h).\n",
        "2",
    );
}

/// Whether the heap has no elements. rust: BinaryHeap::is_empty
#[test]
fn parity_heap_is_empty() {
    assert_all_tiers(
        "## Main\nLet mutable h be heapNew().\nShow heapIsEmpty(h).\nSet h to heapPush(h, 1).\nShow heapIsEmpty(h).\n",
        "true\nfalse",
    );
}

/// Read the smallest element without removing it. rust: BinaryHeap::peek
#[test]
fn parity_heap_peek() {
    assert_all_tiers(
        "## Main\nLet h be heapifySeq([5, 3, 8, 1, 9, 2]).\nShow heapPeek(h).\n",
        "1",
    );
}

/// Push onto a heap keeping the smallest at the root. py: heapq.heappush · rust: BinaryHeap::push · go: heap.Push
#[test]
fn parity_heap_push() {
    assert_all_tiers(
        "## Main\nLet mutable h be heapNew().\nSet h to heapPush(h, 5).\nSet h to heapPush(h, 3).\nSet h to heapPush(h, 8).\nSet h to heapPush(h, 1).\nShow heapPeek(h).\nShow heapLen(h).\n",
        "1\n4",
    );
}

/// Build a heap from an arbitrary sequence in O(n). py: heapq.heapify · go: heap.Init
#[test]
fn parity_heap_heapify() {
    assert_all_tiers(
        "## Main\nLet h be heapifySeq([5, 3, 8, 1, 9, 2]).\nShow heapPeek(h).\nShow heapLen(h).\n",
        "1\n6",
    );
}

/// Pop the minimum: the root leaves and the next-smallest becomes the new root. py: heapq.heappop · rust: BinaryHeap::pop · go: heap.Pop
#[test]
fn parity_heap_pop() {
    assert_all_tiers(
        "## Main\nLet mutable h be heapifySeq([5, 3, 8, 1, 9, 2]).\nShow heapPeek(h).\nSet h to heapPopRest(h).\nShow heapPeek(h).\nSet h to heapPopRest(h).\nShow heapPeek(h).\n",
        "1\n2\n3",
    );
}

/// Replace the smallest with a new value in one sift. py: heapq.heapreplace
#[test]
fn parity_heap_replace() {
    assert_all_tiers(
        "## Main\nLet mutable h be heapifySeq([1, 3, 5, 7]).\nShow heapPeek(h).\nSet h to heapReplace(h, 4).\nShow heapPeek(h).\n",
        "1\n3",
    );
}

/// Push then pop, cheaper than the two apart; if the new value is the smallest it stays put. py: heapq.heappushpop
#[test]
fn parity_heap_pushpop() {
    assert_all_tiers(
        "## Main\nLet mutable h be heapifySeq([3, 5, 7]).\nSet h to heapPushPop(h, 1).\nShow heapPeek(h).\nSet h to heapPushPop(h, 9).\nShow heapPeek(h).\n",
        "3\n5",
    );
}

/// Draining a min-heap yields ascending order. rust: BinaryHeap into_sorted_vec
#[test]
fn parity_heap_sort() {
    assert_all_tiers(
        "## Main\nLet s be heapSortSeq([5, 3, 8, 1, 9, 2, 7]).\nShow item 1 of s.\nShow item 4 of s.\nShow item 7 of s.\n",
        "1\n5\n9",
    );
}

/// The k smallest elements in ascending order. py: heapq.nsmallest
#[test]
fn parity_heap_nsmallest() {
    assert_all_tiers(
        "## Main\nLet s be nSmallest(3, [5, 3, 8, 1, 9, 2, 7]).\nShow length of s.\nShow item 1 of s.\nShow item 3 of s.\n",
        "3\n1\n3",
    );
}

/// The k largest elements in descending order. py: heapq.nlargest
#[test]
fn parity_heap_nlargest() {
    assert_all_tiers(
        "## Main\nLet s be nLargest(3, [5, 3, 8, 1, 9, 2, 7]).\nShow length of s.\nShow item 1 of s.\nShow item 3 of s.\n",
        "3\n9\n7",
    );
}

/// Merge two already-sorted sequences into one sorted run. py: heapq.merge
#[test]
fn parity_heap_merge() {
    assert_all_tiers(
        "## Main\nLet s be heapMerge([1, 4, 7], [2, 3, 8, 9]).\nShow length of s.\nShow item 1 of s.\nShow item 4 of s.\nShow item 7 of s.\n",
        "7\n1\n4\n9",
    );
}

/// Re-establish heap order after the element at a position changed. go: heap.Fix
#[test]
fn parity_heap_fix() {
    assert_all_tiers(
        "## Main\nLet mutable h be heapifySeq([1, 3, 5, 7, 9]).\nSet item 1 of h to 6.\nSet h to heapFix(h, 1).\nShow heapPeek(h).\n",
        "3",
    );
}

/// Remove the element at a position, keeping the heap valid. go: heap.Remove
#[test]
fn parity_heap_remove() {
    assert_all_tiers(
        "## Main\nLet mutable h be heapifySeq([1, 3, 5, 7, 9, 8]).\nSet h to heapRemove(h, 1).\nShow heapLen(h).\nShow heapPeek(h).\n",
        "5\n3",
    );
}
