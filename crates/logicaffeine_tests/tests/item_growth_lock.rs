//! ★ LOCK — a counted loop's arithmetic on the items it grows is raw under one comparison. ★
//!
//! A count kept in a sequence (`Set item v of counts to item v of counts + 1`) and a dynamic-programming row rolled
//! from pass to pass (`Push best to curr ... Set prev to curr`, `best` an item of `prev` plus a bounded value)
//! grow with the passes of the loop that drives them, so their additions carry an overflow check that keeps the
//! loop from vectorizing (histogram ran 1.5x and knapsack 2x slower than 0.10.1 on that check). The item-growth
//! analysis (`optimize::item_growth`) bounds the items by a line in the passes made, and the loop is versioned on
//! its limit: `If limit <= C: <the loop, those additions raw> Otherwise: <the loop as written>`. This file holds the
//! guard and the raw form on AOT-Rust and C, the gate (`## No accumbound`), and every program — both sides of the
//! guard — to the VM's output on the tree-walker, the generated Rust and both C backends.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test item_growth_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

use logicaffeine_compile::compile::{compile_to_c, compile_to_rust, optimizations_fired};
use logicaffeine_compile::optimization::decorate_source;
use tier_parity::{Backend, Program};

/// The child side of every isolated backend run. Does nothing unless `LOGOS_PARITY_JOB` is set.
#[test]
#[ignore = "the worker the parity sweep re-executes this binary into"]
fn tier_parity_worker() {
    tier_parity::worker_main();
}

/// The histogram benchmark's shape: a count per bucket, one increment per pass.
fn counts(n: u64) -> String {
    format!(
        r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("{n}").
Let mutable counts be a new Seq of Int.
Let mutable k be 0.
While k is less than 1000:
    Push 0 to counts.
    Set k to k + 1.
Let mutable seed be 42.
Let mutable i be 0.
While i is less than n:
    Set seed to (seed * 1103515245 + 12345) % 2147483648.
    Let v be (seed / 65536) % 1000.
    Set item (v + 1) of counts to (item (v + 1) of counts) + 1.
    Set i to i + 1.
Let mutable h be 0.
Set k to 1.
While k is at most 1000:
    Set h to (h * 31 + item k of counts) % 1000000007.
    Set k to k + 1.
Show h.
"#
    )
}

/// The knapsack benchmark's shape: a row rolled from pass to pass, each item an item of the last row plus a value.
fn rolled(n: u64) -> String {
    format!(
        r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("{n}").
Let capacity be n * 5.
Let mutable weights be a new Seq of Int.
Let mutable vals be a new Seq of Int.
Let mutable seed be 42.
Let mutable i be 0.
While i is less than n:
    Set seed to (seed * 1103515245 + 12345) % 2147483648.
    Push (seed / 65536) % 50 + 1 to weights.
    Set seed to (seed * 1103515245 + 12345) % 2147483648.
    Push (seed / 65536) % 100 + 1 to vals.
    Set i to i + 1.
Let cols be capacity + 1.
Let mutable prev be a new Seq of Int.
Set i to 0.
While i is less than cols:
    Push 0 to prev.
    Set i to i + 1.
Set i to 0.
While i is less than n:
    Let mutable curr be a new Seq of Int.
    Let wi be item (i + 1) of weights.
    Let vi be item (i + 1) of vals.
    Let mutable w be 0.
    While w is at most capacity:
        Let mutable best be item (w + 1) of prev.
        If w is at least wi:
            Let take be item (w - wi + 1) of prev + vi.
            If take is greater than best:
                Set best to take.
        Push best to curr.
        Set w to w + 1.
    Set prev to curr.
    Set i to i + 1.
Show item (capacity + 1) of prev.
"#
    )
}

/// Two counts, each growing by 2^45 per pass. The analysis cannot tell the two slots apart, so it bounds the
/// growth at 2^46 per pass and guards the loop at about 131,071 passes; each slot really grows by 2^45, so a run
/// past the guard takes the loop as written and still never leaves a word.
fn two_counts(n: u64) -> String {
    format!(
        r#"## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt("{n}").
Let mutable s be a new Seq of Int.
Push 0 to s.
Push 0 to s.
Let mutable i be 0.
While i is less than n:
    Set item 1 of s to (item 1 of s) + 35184372088832.
    Set item 2 of s to (item 2 of s) + 35184372088832.
    Set i to i + 1.
Show item 1 of s.
Show item 2 of s.
"#
    )
}

/// The branch the guard `limit <= C` takes into: the block its `{` opens, through the brace that closes it.
fn guarded_branch<'s>(code: &'s str, guard: &str) -> &'s str {
    let start = code
        .find(guard)
        .unwrap_or_else(|| panic!("the guard `{guard}` is emitted:\n{code}"));
    let open = start + code[start..].find('{').expect("the guarded block opens");
    let mut depth = 0usize;
    for (i, c) in code[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &code[open..=open + i];
                }
            }
            _ => {}
        }
    }
    panic!("the guarded block closes:\n{code}")
}

#[test]
fn a_count_kept_in_a_sequence_adds_raw_under_one_comparison() {
    let src = counts(1000);
    let rust = compile_to_rust(&src).expect("compiles to Rust");
    let fast = guarded_branch(&rust, "(n <= 9223372036854775807_i64)");
    assert!(
        !fast.contains("logos_add_i64("),
        "the guarded loop adds the count raw:\n{fast}"
    );
    assert!(
        rust.contains("logos_add_i64("),
        "the loop as written keeps its check in the other branch:\n{rust}"
    );
    let c = compile_to_c(&src).expect("compiles to C");
    assert!(
        c.contains("<= 9223372036854775807LL"),
        "direct C versions the same loop on the same comparison"
    );
    assert!(optimizations_fired(&src).contains(&"accumbound"));
}

#[test]
fn a_rolled_row_adds_raw_under_one_comparison() {
    let src = rolled(200);
    let rust = compile_to_rust(&src).expect("compiles to Rust");
    let fast = guarded_branch(&rust, "(n <= 92233720368547758_i64)");
    assert!(
        fast.contains("let take = (") && !fast.contains("let take = logos_add_i64("),
        "the guarded loop adds an item of the last row and a value raw:\n{fast}"
    );
    assert!(
        rust.contains("let take = logos_add_i64("),
        "the loop as written keeps its check in the other branch"
    );
}

#[test]
fn accumbound_off_leaves_the_loop_as_written() {
    for src in [counts(1000), rolled(200)] {
        let off = decorate_source(&src, &["accumbound"]);
        let rust = compile_to_rust(&off).expect("compiles to Rust");
        assert!(
            !rust.contains("(n <= 9223372036854775807") && !rust.contains("(n <= 92233720368547758"),
            "`## No accumbound` versions nothing:\n{rust}"
        );
    }
}

#[test]
fn every_program_means_the_same_on_every_backend_on_both_sides_of_the_guard() {
    let programs = vec![
        Program::new("item_growth/counts", counts(200_000)),
        Program::new("item_growth/rolled", rolled(300)),
        Program::new("item_growth/two_counts_below_the_guard", two_counts(100_000)),
        Program::new("item_growth/two_counts_past_the_guard", two_counts(140_000)),
    ];
    let refused: Vec<&str> = programs
        .iter()
        .filter(|p| tier_parity::rejection(&p.source).is_some())
        .map(|p| p.id.as_str())
        .collect();
    assert!(refused.is_empty(), "the front end refuses {refused:?}, which then pin nothing");
    let rows = tier_parity::sweep(
        &programs,
        &[
            Backend::TreeWalker,
            Backend::Vm,
            Backend::RustRelease,
            Backend::C,
            Backend::CBytecode,
        ],
        4,
    )
    .unwrap_or_else(|why| panic!("the sweep could not start: {why}"));
    let bad: Vec<String> = rows
        .iter()
        .filter(|row| !row.matches())
        .map(|row| format!("{} on {}: {} {}", row.program, row.backend.name(), row.status(), row.detail()))
        .collect();
    assert!(bad.is_empty(), "{} runs differ from the VM:\n{}", bad.len(), bad.join("\n"));
}

/// Both sides of the two-count guard: the guard sits between the two runs, so the parity above pins the loop as
/// written as well as its raw copy.
#[test]
fn the_two_count_runs_sit_on_both_sides_of_the_guard() {
    let rust = compile_to_rust(&two_counts(100_000)).expect("compiles to Rust");
    let at = rust.find("(n <= ").expect("the loop is versioned");
    let bound: u64 = rust[at + 6..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .expect("the guard's bound is a literal");
    assert!(
        (100_000..140_000).contains(&bound),
        "the guard ({bound}) lies between the two runs"
    );
}
