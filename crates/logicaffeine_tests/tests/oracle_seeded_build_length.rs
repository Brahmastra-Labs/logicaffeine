//! A counted build loop that appends to a collection which already holds items.
//!
//! The oracle learns a length from a counted build loop: `While i is at most n: Push x to a; Set i to
//! i + 1` gives `length(a) >= n - i0 + 1`. A collection that enters the loop holding `k` items ends it
//! with `k` more: the coins and knapsack shapes seed their table with `Push 1 to dp` and then append `n`
//! zeros, so `length(dp) = n + 1`. Every read of the table at `item (j + 1) of dp` with `j <= n` is then
//! in bounds, and nothing in the hot loop needs a check.
//!
//! The bound is a LOWER bound and stays sound when the loop runs zero times: the trip count `n - i0 + 1`
//! is then at most zero and `n - i0 + 1 + k <= k`. The runtime half below holds every tier to the same
//! answers at the edge sizes, and holds a read one past the proven length to its check.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{build_c_release, compile_logos, compile_to_rust, run_all_tiers};
use logicaffeine_compile::compile::{compile_to_c, run_strict_tw_with_args, run_strict_vm_with_args};

/// The coins benchmark's table: one seeded item, then `n` appended, then a DP sweep over
/// `item (j + 1)` and `item (j - coin + 1)`, then a checksum over every item.
const SEEDED_DP: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable coinList be a new Seq of Int.
Push 1 to coinList.
Push 5 to coinList.
Push 10 to coinList.
Push 25 to coinList.
Push 50 to coinList.
Push 100 to coinList.
Let mutable dp be a new Seq of Int.
Push 1 to dp.
Let mutable i be 1.
While i is at most n:
    Push 0 to dp.
    Set i to i + 1.
Let mutable c be 1.
While c is at most 6:
    Let coin be item c of coinList.
    Let mutable j be coin.
    While j is at most n:
        Set item (j + 1) of dp to (item (j + 1) of dp + item (j - coin + 1) of dp) % 1000000007.
        Set j to j + 1.
    Set c to c + 1.
Let mutable checksum be 0.
Set i to 1.
While i is at most n + 1:
    Set checksum to (checksum * 31 + item i of dp) & 268435455.
    Set i to i + 1.
Show "" + item (n + 1) of dp + " " + checksum.
"#;

/// The same table read one item past its length: `item (j + 2)` reaches `n + 2` at `j = n`.
const SEEDED_OVERREAD: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable dp be a new Seq of Int.
Push 1 to dp.
Let mutable i be 1.
While i is at most n:
    Push i to dp.
    Set i to i + 1.
Let mutable total be 0.
Let mutable j be 1.
While j is at most n:
    Set total to total + item (j + 2) of dp.
    Set j to j + 1.
Show total.
"#;

/// The text of the program before its final `show(`: every loop of the programs above sits there.
fn loops_of(rust: &str) -> &str {
    &rust[..rust.rfind("show(").expect("the program shows its result")]
}

#[test]
fn a_seeded_table_is_read_and_written_unchecked() {
    let rust = compile_to_rust(SEEDED_DP).unwrap();
    let loops = loops_of(&rust);
    assert!(
        !loops.contains("dp.logos_item"),
        "`length(dp) >= n + 1` proves every `item (j + 1)`, `item (j - coin + 1)` and checksum read of \
         `dp` in bounds; no loop may keep a checked accessor on it. Got:\n{rust}"
    );
}

#[test]
fn the_coins_benchmark_sweeps_its_table_unchecked() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/programs/coins/main.lg");
    let source = std::fs::read_to_string(&path).expect("benchmarks/programs/coins/main.lg");
    let rust = compile_to_rust(&source).unwrap();
    let loops = loops_of(&rust);
    assert!(
        !loops.contains("dp.logos_item"),
        "the coins benchmark's DP sweep and checksum must index `dp` unchecked. Got:\n{rust}"
    );
}

#[test]
fn a_read_past_the_seeded_length_keeps_its_check() {
    let rust = compile_to_rust(SEEDED_OVERREAD).unwrap();
    assert!(
        loops_of(&rust).contains("dp.logos_item"),
        "`item (j + 2)` reaches `n + 2 > length(dp) = n + 1`; its check must stay. Got:\n{rust}"
    );
}

/// Every tier's outcome for `source` at each size: the tree-walker, the VM, the AOT-Rust binary and the
/// C binary, each as (stdout trimmed, failed?, stderr).
fn every_tier(source: &str, sizes: &[&str]) -> Vec<(String, Vec<(String, String, bool, String)>)> {
    let aot = compile_logos(source);
    assert!(aot.success, "AOT build failed:\n{}", aot.stderr);
    let c_code = compile_to_c(source).unwrap_or_else(|e| panic!("LOGOS→C: {e:?}"));
    let dir = tempfile::tempdir().unwrap();
    let (c_src, c_bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&c_src, &c_code).unwrap();
    let built = build_c_release(&c_src, &c_bin).unwrap();
    assert!(built.status.success(), "C build failed:\n{}", String::from_utf8_lossy(&built.stderr));
    let run = |bin: &std::path::Path, n: &str| {
        let out = std::process::Command::new(bin).arg(n).output().unwrap();
        (
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
            !out.status.success(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    };
    sizes
        .iter()
        .map(|n| {
            let argv = vec!["prog".to_string(), n.to_string()];
            let tw = run_strict_tw_with_args(source, &argv);
            let vm = run_strict_vm_with_args(source, &argv);
            let mut rows = Vec::new();
            for (tier, o) in [("tree-walker", tw), ("vm", vm)] {
                let err = o.error.clone().unwrap_or_default();
                rows.push((tier.to_string(), o.output.trim().to_string(), o.error.is_some(), err));
            }
            for (tier, bin) in [("aot-rust", aot.binary_path.as_path()), ("c", c_bin.as_path())] {
                let (out, failed, err) = run(bin, n);
                rows.push((tier.to_string(), out, failed, err));
            }
            (n.to_string(), rows)
        })
        .collect()
}

#[test]
fn a_seeded_table_answers_exactly_at_every_size_on_every_tier() {
    let expected = [
        ("0", "1 1"),
        ("1", "1 32"),
        ("4", "1 954305"),
        ("5", "2 29583457"),
        ("7", "2 243979361"),
        ("100", "293 235887968"),
        ("1000", "2103596 147082771"),
    ];
    let sizes: Vec<&str> = expected.iter().map(|(n, _)| *n).collect();
    for ((n, rows), (_, want)) in every_tier(SEEDED_DP, &sizes).into_iter().zip(expected) {
        for (tier, out, failed, err) in rows {
            assert!(!failed, "n = {n}: {tier} failed: {err}");
            assert_eq!(out, want, "n = {n}: {tier}");
        }
    }
}

#[test]
fn a_read_past_the_seeded_length_fails_on_every_tier() {
    for (n, rows) in every_tier(SEEDED_OVERREAD, &["3", "1"]) {
        let index = n.parse::<i64>().unwrap() + 2;
        for (tier, _, failed, err) in rows {
            assert!(failed, "n = {n}: {tier} must stop at item {index} of a {}-item table", index - 1);
            assert!(
                err.contains(&format!("Index {index} out of bounds")),
                "n = {n}: {tier} must name the missing item. stderr:\n{err}"
            );
        }
    }
}

#[test]
fn a_zero_trip_build_keeps_the_seed_alone() {
    // n <= 0: the build loop never runs, the table is the one seeded item, and every sweep is empty.
    for (n, rows) in every_tier(SEEDED_OVERREAD, &["0", "-3"]) {
        for (tier, out, failed, err) in rows {
            assert!(!failed, "n = {n}: {tier} failed: {err}");
            assert_eq!(out, "0", "n = {n}: {tier}");
        }
    }
}

/// A literal bound gives the exact length: two seeded items and ten appended make twelve.
#[test]
fn a_literal_bound_counts_the_seed_into_the_exact_length() {
    let source = |last: u32| {
        format!(
            "## Main
Let mutable a be a new Seq of Int.
Push 7 to a.
Push 8 to a.
Let mutable i be 1.
While i is at most 10:
    Push i to a.
    Set i to i + 1.
Let mutable s be 0.
Set i to 1.
While i is at most {last}:
    Set s to s + item i of a.
    Set i to i + 1.
Show s.
"
        )
    };
    let exact = source(12);
    let rust = compile_to_rust(&exact).unwrap();
    assert!(
        !loops_of(&rust).contains("a.logos_item"),
        "`length(a) = 12` proves `item i of a` for `i <= 12`. Got:\n{rust}"
    );
    let agreed = run_all_tiers(&exact);
    assert_eq!((agreed.output.trim(), agreed.error.as_deref()), ("70", None));
    common::assert_c_output(&exact, "70");
    common::assert_compiled_equals_interpreted(&exact);

    let over = source(13);
    assert!(
        loops_of(&compile_to_rust(&over).unwrap()).contains("a.logos_item"),
        "`item 13` of a twelve-item list keeps its check"
    );
    let agreed = run_all_tiers(&over);
    assert!(
        agreed.error.as_deref().is_some_and(|e| e.contains("Index 13 out of bounds")),
        "every tier stops at item 13: {:?}",
        agreed.error
    );
    common::assert_c_traps(&over, "Index 13 out of bounds");
}
