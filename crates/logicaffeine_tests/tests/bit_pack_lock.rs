//! `Opt::BitPack`: a local `Seq of Bool` held one bit per item.
//!
//! A `Seq of Bool` compiled to a `Vec<bool>`, a byte per item, so a sieve of 10^8 numbers streamed a 100 MB table
//! through the cache on every pass of its inner loop. A local sequence of flags used only through item reads, item
//! stores, pushes and its length is now a `LogosBits`, a bit per item: the sieve's table is 12.5 MB, and the benchmark
//! runs 2.8x faster, 2.7x the speed of C. Every item reads and stores as before, across word boundaries, from the back,
//! and past the end with the sentence every tier reports. A sequence used any other way keeps its `Vec<bool>`.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_c_output, assert_compiled_equals_interpreted, compile_to_rust, run_all_tiers};
use logicaffeine_compile::optimization::decorate_source;

const SIEVE: &str = "## To sieve (limit: Int) -> Int:
    Let mutable flags be a new Seq of Bool.
    Let mutable i be 0.
    While i is at most limit:
        Push false to flags.
        Set i to i + 1.
    Let mutable count be 0.
    Set i to 2.
    While i is at most limit:
        If item (i + 1) of flags equals false:
            Set count to count + 1.
            Let mutable j be i * i.
            While j is at most limit:
                Set item (j + 1) of flags to true.
                Set j to j + i.
        Set i to i + 1.
    Return count.

## Main
Show sieve(1000).
Show sieve(130).
Show sieve(64).
Show sieve(1).
";

#[test]
fn a_table_of_flags_is_one_bit_per_item() {
    let rust = compile_to_rust(SIEVE).unwrap();
    assert!(
        rust.contains("LogosBits"),
        "the sieve's flags are bits. Got:\n{rust}"
    );
    assert!(
        !rust.contains("Vec<bool>"),
        "no byte-per-flag table is left. Got:\n{rust}"
    );
}

#[test]
fn turning_the_optimization_off_keeps_the_bytes() {
    let rust = compile_to_rust(&decorate_source(SIEVE, &["bitpack"])).unwrap();
    assert!(!rust.contains("LogosBits"), "Got:\n{rust}");
}

#[test]
fn the_sieve_counts_alike_on_every_tier() {
    let agreed = run_all_tiers(SIEVE);
    assert_eq!(
        (agreed.output.trim(), agreed.error.as_deref()),
        ("168\n31\n18\n0", None)
    );
    assert_c_output(SIEVE, "168\n31\n18\n0");
    assert_compiled_equals_interpreted(SIEVE);
}

/// Pushes of every pattern across three 64-bit words, stores that set and clear, reads at the word edges, an
/// index from the back and the length.
const EDGES: &str = "## To probe (n: Int) -> Int:
    Let mutable flags be a new Seq of Bool.
    Let mutable i be 1.
    While i is at most n:
        Let third be i % 3 equals 0.
        Push third to flags.
        Set i to i + 1.
    Set item 64 of flags to true.
    Set item 65 of flags to false.
    Set item 63 of flags to false.
    Set item 128 of flags to true.
    Set item 129 of flags to true.
    Set item 129 of flags to false.
    Let mutable total be 0.
    Set i to 1.
    While i is at most length of flags:
        If item i of flags equals true:
            Set total to total + i.
        Set i to i + 1.
    If item -1 of flags equals true:
        Set total to total + 1000000.
    Return total * 1000 + length of flags.

## Main
Show probe(130).
Show probe(192).
";

#[test]
fn every_item_reads_back_across_word_boundaries() {
    let rust = compile_to_rust(EDGES).unwrap();
    assert!(rust.contains("LogosBits"), "Got:\n{rust}");
    let agreed = run_all_tiers(EDGES);
    assert!(agreed.error.is_none(), "{:?}", agreed.error);
    assert_c_output(EDGES, agreed.output.trim());
    assert_compiled_equals_interpreted(EDGES);
}

const PAST_THE_END: &str = "## To past (n: Int) -> Bool:
    Let mutable flags be a new Seq of Bool.
    Let mutable i be 1.
    While i is at most n:
        Push true to flags.
        Set i to i + 1.
    Return item (n + 1) of flags.

## Main
Show past(64).
";

#[test]
fn a_read_past_the_end_fails_alike() {
    let rust = compile_to_rust(PAST_THE_END).unwrap();
    assert!(rust.contains("LogosBits"), "Got:\n{rust}");
    let agreed = run_all_tiers(PAST_THE_END);
    assert!(
        agreed.error.as_deref().is_some_and(|e| e.contains("65")),
        "item 65 of a 64-item sequence fails, naming the index: {:?} {:?}",
        agreed.output,
        agreed.error
    );
    assert_compiled_equals_interpreted(PAST_THE_END);
}

/// Each program uses its flags in one way a bit table cannot serve: handed to a function, shown whole, iterated,
/// returned, copied, rebound, popped.
const OTHER_USES: [&str; 7] = [
    "## To count (xs: Seq of Bool) -> Int:\n    Return length of xs.\n\n## Main\nLet mutable flags be a new Seq of Bool.\nPush true to flags.\nPush false to flags.\nShow count(flags).\n",
    "## Main\nLet mutable flags be a new Seq of Bool.\nPush true to flags.\nPush false to flags.\nShow flags.\n",
    "## Main\nLet mutable flags be a new Seq of Bool.\nPush true to flags.\nPush false to flags.\nLet mutable n be 0.\nRepeat for f in flags:\n    If f equals true:\n        Set n to n + 1.\nShow n.\n",
    "## To make (n: Int) -> Seq of Bool:\n    Let mutable flags be a new Seq of Bool.\n    Let mutable i be 0.\n    While i is less than n:\n        Push true to flags.\n        Set i to i + 1.\n    Return flags.\n\n## Main\nShow length of make(3).\n",
    "## Main\nLet mutable flags be a new Seq of Bool.\nPush true to flags.\nLet other be copy of flags.\nPush false to flags.\nShow length of other.\nShow length of flags.\n",
    "## Main\nLet mutable flags be a new Seq of Bool.\nPush true to flags.\nLet mutable other be a new Seq of Bool.\nPush false to other.\nSet flags to other.\nShow item 1 of flags.\n",
    "## Main\nLet mutable flags be a new Seq of Bool.\nPush true to flags.\nPush false to flags.\nPop from flags.\nShow length of flags.\n",
];

#[test]
fn flags_used_any_other_way_keep_their_bytes_and_answer_alike() {
    for (k, program) in OTHER_USES.iter().enumerate() {
        let rust = compile_to_rust(program).unwrap_or_else(|e| panic!("program {k}: {e:?}"));
        assert!(
            !rust.contains("LogosBits"),
            "program {k} uses its flags past reads and stores:\n{rust}"
        );
        let agreed = run_all_tiers(program);
        assert!(agreed.error.is_none(), "program {k}: {:?}", agreed.error);
        assert_compiled_equals_interpreted(program);
    }
}

#[test]
fn the_sieve_benchmark_packs_its_flags() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/programs/sieve/main.lg");
    let rust = compile_to_rust(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(rust.contains("LogosBits"), "Got:\n{rust}");
}
