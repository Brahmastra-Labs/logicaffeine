//! Two-way (Crochemore-Perrin) substring search ≡ the naive O(n·m) reference — fuzz-locked.
//!
//! `strings.lg`'s `indexOf` now routes through `twoWayIndexBytes` (linear-time, O(1) extra space —
//! the algorithm CPython/glibc use), keeping `indexOfNaiveBytes` (the trusted matchAt-scan) as the
//! reference. This lock proves them byte-identical over a deterministic corpus of adversarial inputs:
//! a SMALL alphabet ({A,B,C}) so periodic patterns, repeated partial matches, and the critical-
//! factorization / memory paths are all exercised, across many text/pattern length combinations
//! (including pattern-longer-than-text, pattern==text, and absent patterns). Every tier runs the
//! same `.lg`, so a two-way bug on any backend surfaces as a non-zero mismatch count here.
//!
//! (This is the fuzz that discovered a pre-existing copy-and-patch JIT soundness bug: a hot list-
//! parameter function with an early `Return` from a loop was miscompiled. The fix — the JIT declines
//! that pattern to the correct VM bytecode — lives in `logicaffeine_jit`; this lock runs on the
//! test harness's interpreter/VM/AOT tiers, where two-way is proven correct.)

mod common;
use common::assert_all_tiers;

/// A deterministic LCG fuzz: 1000 random (text, pattern) pairs over {A,B,C}, comparing the two-way
/// search to the naive reference. `Show`s the mismatch count — which must be 0 on every tier.
const TWO_WAY_FUZZ: &str = "## Main\n\
Let mutable seed be 123456789.\n\
Let mutable mism be 0.\n\
Let mutable t be 0.\n\
While t is less than 1000:\n\
\x20   Set seed to (seed * 1103515245 + 12345) mod 2147483648.\n\
\x20   Let tlen be (seed mod 18) + 1.\n\
\x20   Set seed to (seed * 1103515245 + 12345) mod 2147483648.\n\
\x20   Let plen be (seed mod 5) + 1.\n\
\x20   Let mutable text be a new Seq of Int.\n\
\x20   Let mutable a be 1.\n\
\x20   While a is at most tlen:\n\
\x20       Set seed to (seed * 1103515245 + 12345) mod 2147483648.\n\
\x20       Push ((seed mod 3) + 65) to text.\n\
\x20       Set a to a + 1.\n\
\x20   Let mutable pat be a new Seq of Int.\n\
\x20   Let mutable b be 1.\n\
\x20   While b is at most plen:\n\
\x20       Set seed to (seed * 1103515245 + 12345) mod 2147483648.\n\
\x20       Push ((seed mod 3) + 65) to pat.\n\
\x20       Set b to b + 1.\n\
\x20   Let r1 be twoWayIndexBytes(text, pat).\n\
\x20   Let r2 be indexOfNaiveBytes(text, pat).\n\
\x20   Let mutable eq be 0.\n\
\x20   If r1 is equal to r2:\n\
\x20       Set eq to 1.\n\
\x20   If eq is equal to 0:\n\
\x20       Set mism to mism + 1.\n\
\x20   Set t to t + 1.\n\
Show mism.\n";

#[test]
fn two_way_search_matches_the_naive_reference_on_every_tier() {
    assert_all_tiers(TWO_WAY_FUZZ, "0");
}

/// Concrete anchors: the two-way `indexOf` (1-based, 0 when absent) on hand-checked cases, including
/// the periodic pattern that stresses the critical factorization + the absent/empty edges.
#[test]
fn two_way_indexof_hits_known_cases_on_every_tier() {
    assert_all_tiers(
        "## Main\n    Show indexOf(\"hello world\", \"world\").\n",
        "7",
    );
    assert_all_tiers(
        "## Main\n    Show indexOf(\"abcabcabd\", \"abcabd\").\n",
        "4",
    );
    assert_all_tiers("## Main\n    Show indexOf(\"aaaa\", \"aab\").\n", "0");
    assert_all_tiers("## Main\n    Show indexOf(\"abc\", \"\").\n", "0");
    assert_all_tiers("## Main\n    Show indexOf(\"aaaaaa\", \"aaa\").\n", "1");
}

/// The `twoWayIndexBytesFrom(sb, pb, from)` primitive — the workhorse the refactored `replace`,
/// `split`, `countSub`, `lastIndexOf`, `splitN`, `replaceN`, `splitAfter` walk occurrences with —
/// must equal a naive `matchAt` scan started at `from`, for 1000 random (text, pattern, from) triples
/// including `from` past the end. A bug here would corrupt every occurrence-walking function at once.
const FROM_FUZZ: &str = "## Main\n\
Let mutable seed be 987654321.\n\
Let mutable mism be 0.\n\
Let mutable t be 0.\n\
While t is less than 1000:\n\
\x20   Set seed to (seed * 1103515245 + 12345) mod 2147483648.\n\
\x20   Let tlen be (seed mod 18) + 1.\n\
\x20   Set seed to (seed * 1103515245 + 12345) mod 2147483648.\n\
\x20   Let plen be (seed mod 5) + 1.\n\
\x20   Let mutable text be a new Seq of Int.\n\
\x20   Let mutable a be 1.\n\
\x20   While a is at most tlen:\n\
\x20       Set seed to (seed * 1103515245 + 12345) mod 2147483648.\n\
\x20       Push ((seed mod 3) + 65) to text.\n\
\x20       Set a to a + 1.\n\
\x20   Let mutable pat be a new Seq of Int.\n\
\x20   Let mutable b be 1.\n\
\x20   While b is at most plen:\n\
\x20       Set seed to (seed * 1103515245 + 12345) mod 2147483648.\n\
\x20       Push ((seed mod 3) + 65) to pat.\n\
\x20       Set b to b + 1.\n\
\x20   Set seed to (seed * 1103515245 + 12345) mod 2147483648.\n\
\x20   Let from be (seed mod (tlen + 1)) + 1.\n\
\x20   Let tw be twoWayIndexBytesFrom(text, pat, from).\n\
\x20   Let mutable naive be 0.\n\
\x20   Let mutable ii be from.\n\
\x20   Let mutable searching be true.\n\
\x20   While searching is equal to true:\n\
\x20       If ii is greater than (length of text):\n\
\x20           Set searching to false.\n\
\x20       Otherwise:\n\
\x20           If matchAt(text, pat, ii):\n\
\x20               Set naive to ii.\n\
\x20               Set searching to false.\n\
\x20           Otherwise:\n\
\x20               Set ii to ii + 1.\n\
\x20   Let mutable eq be 0.\n\
\x20   If tw is equal to naive:\n\
\x20       Set eq to 1.\n\
\x20   If eq is equal to 0:\n\
\x20       Set mism to mism + 1.\n\
\x20   Set t to t + 1.\n\
Show mism.\n";

#[test]
fn two_way_search_from_matches_naive_on_every_tier() {
    assert_all_tiers(FROM_FUZZ, "0");
}
