//! Phase D — Differential corpus (PE_IMPROVE §4.2/§5, closes gap G8).
//!
//! The centerpiece safety case: over a broad corpus exercising every B1–B5 feature, the
//! genuine PE residual run through the tree-walker (`run_p1`) must observably agree with the
//! production tree-walker on the source (`run_treewalk`) — same output stream, same
//! value/error class. The tree-walker is the independent oracle; any divergence is a real PE
//! bug. This is "robust to the point of absurdity": one assertion, dozens of programs.
//!
//! NOTE: programs use `"\` + real newlines + real indentation (NOT `\n\` continuation).

mod pe_support;

use pe_support::*;

/// A broad, hand-curated corpus spanning the operation surface the PE now folds.
fn differential_corpus() -> Vec<(&'static str, &'static str)> {
    vec![
        // --- B1: arithmetic / text / coercion ---
        ("int_arith", "## Main\nShow 2 + 3 * 4 - 1."),
        ("int_div_mod", "## Main\nShow 17 / 5.\nShow 17 % 5."),
        ("int_bitwise", "## Main\nShow 6 xor 3.\nShow 1 shifted left by 4."),
        ("float_arith", "## Main\nShow 1.5 + 2.25.\nShow 10.0 / 4.0."),
        ("mixed_int_float", "## Main\nShow 3 + 0.5."),
        ("text_concat", "## Main\nLet n be 42.\nShow \"n is {n}\"."),
        ("bool_logic", "## Main\nShow true and false.\nShow true or false.\nShow not true."),
        ("comparisons", "## Main\nShow 3 is less than 5.\nShow 5 is at most 5."),
        // --- dynamic accumulator (forces a residual loop) ---
        ("dynamic_sum", "## Main\nLet mutable s be 0.\nRepeat for i from 1 to 200000:\n    Set s to s + 1.\nShow s."),
        // --- B2: structs / partial-static ---
        ("struct_field", "## A Box has:\n    A base: Int.\n    A flex: Int.\n\n## Main\nLet b be a new Box with base 3 and flex 7.\nShow b's base.\nShow b's flex."),
        ("struct_partial", "## A Box has:\n    A base: Int.\n    A flex: Int.\n\n## Main\nLet mutable d be 0.\nRepeat for i from 1 to 100:\n    Set d to d + 1.\nLet b be a new Box with base 5 and flex d.\nShow b's base.\nShow b's flex."),
        ("struct_setfield", "## A Box has:\n    A base: Int.\n    A flex: Int.\n\n## Main\nLet mutable b be a new Box with base 1 and flex 7.\nSet b's base to 9.\nShow b's base.\nShow b's flex."),
        // --- B2.2: lists / tuples ---
        ("list_index", "## Main\nLet xs be [10, 20, 30].\nShow item 2 of xs.\nShow length of xs."),
        ("tuple_index", "## Main\nLet t be (1, 2, 3).\nShow item 3 of t."),
        ("list_dynamic", "## Main\nLet mutable d be 0.\nRepeat for i from 1 to 100:\n    Set d to d + 1.\nLet xs be [1, d, 3].\nShow item 1 of xs.\nShow item 2 of xs."),
        // --- B3: loops ---
        ("while_static", "## Main\nLet mutable i be 3.\nLet mutable s be 0.\nWhile i is greater than 0:\n    Set s to s + i.\n    Set i to i - 1.\nShow s."),
        ("repeat_range", "## Main\nLet mutable s be 0.\nRepeat for i from 1 to 5:\n    Set s to s + i.\nShow s."),
        ("repeat_break", "## Main\nLet mutable s be 0.\nRepeat for i from 1 to 10:\n    Set s to s + i.\n    If i equals 3:\n        Break.\nShow s."),
        ("nested_loops", "## Main\nLet mutable s be 0.\nRepeat for i from 1 to 3:\n    Repeat for j from 1 to 3:\n        Set s to s + 1.\nShow s."),
        // --- B3 MSG: recursion ---
        ("factorial", "## To fact (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * fact(n - 1).\n\n## Main\nShow fact(6)."),
        ("recursive_count_dynamic", "## To count (n: Int) and (acc: Int) -> Int:\n    If n equals 0:\n        Return acc.\n    Return count(n - 1, acc + 1).\n\n## Main\nLet mutable d be 0.\nRepeat for i from 1 to 50:\n    Set d to d + 1.\nShow count(d, 0)."),
        // --- B4: flow-sensitive refinement ---
        ("guard_refine", "## Main\nLet mutable x be 0.\nRepeat for i from 1 to 100:\n    Set x to x + 1.\nIf x equals 100:\n    Show x.\nOtherwise:\n    Show 0."),
        ("nested_guard", "## Main\nLet mutable x be 0.\nRepeat for i from 1 to 100:\n    Set x to x + 1.\nIf x equals 5:\n    Show \"a\".\nOtherwise:\n    If x equals 5:\n        Show \"b\".\n    Otherwise:\n        Show \"c\"."),
        // --- B5: maps / sets / text / closures ---
        ("map_ops", "## Main\nLet m be a new Map of Text to Int.\nSet item \"a\" of m to 1.\nSet item \"b\" of m to 2.\nShow item \"b\" of m.\nShow length of m."),
        ("set_ops", "## Main\nLet s be a new Set of Int.\nAdd 3 to s.\nAdd 9 to s.\nShow length of s.\nIf s contains 3:\n    Show \"yes\".\nOtherwise:\n    Show \"no\"."),
        ("text_length", "## Main\nShow length of \"hello world\"."),
        ("closure_hof", "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nShow apply((n: Int) -> n * 2, 21)."),
        ("closure_dynamic", "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nLet mutable d be 0.\nRepeat for i from 1 to 50:\n    Set d to d + 1.\nShow apply((n: Int) -> n + 1, d)."),
        // --- mixed / functions ---
        ("multi_function", "## To double (n: Int) -> Int:\n    Return n * 2.\n\n## To inc (n: Int) -> Int:\n    Return n + 1.\n\n## Main\nShow double(inc(20))."),
        // --- edge cases (robustness) ---
        ("negative_ints", "## Main\nShow 0 - 5.\nShow 3 * (0 - 4)."),
        ("multi_show_ordering", "## Main\nShow 1.\nShow 2.\nShow 3."),
        ("empty_range_loop", "## Main\nLet mutable s be 7.\nRepeat for i from 1 to 0:\n    Set s to s + i.\nShow s."),
        ("while_false_eliminated", "## Main\nLet mutable s be 5.\nWhile s is greater than 100:\n    Set s to s + 1.\nShow s."),
        ("return_mid_loop", "## To f () -> Int:\n    Let mutable s be 0.\n    Repeat for i from 1 to 10:\n        Set s to s + i.\n        If i equals 4:\n            Return s.\n    Return s.\n\n## Main\nShow f()."),
        ("mutual_recursion", "## To isEven (n: Int) -> Bool:\n    If n equals 0:\n        Return true.\n    Return isOdd(n - 1).\n\n## To isOdd (n: Int) -> Bool:\n    If n equals 0:\n        Return false.\n    Return isEven(n - 1).\n\n## Main\nShow isEven(10)."),
        ("float_formatting", "## Main\nShow 3.14.\nShow 1.0 + 2.0."),
        ("alias_mutation", "## Main\nLet mutable d be 0.\nRepeat for i from 1 to 100:\n    Set d to d + 1.\nLet s be [1, 2, 3].\nLet a be s.\nSet item 1 of a to d.\nShow item 1 of s."),
        ("deeply_nested_arith", "## Main\nShow ((1 + 2) * (3 + 4)) - ((5 - 1) * 2)."),
        ("closure_capture", "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\nLet c be 100.\nShow apply((n: Int) -> n + c, 5)."),
        // --- rich CRDTs: the PE residual must drive OR-Set / RGA identically to the
        //     reference engine (locks that the partial evaluator carries CRDT support too) ---
        ("crdt_shared_set", "## Definition\nA Party is Shared and has:\n    a guests, which is a SharedSet of Text.\n\n## Main\nLet mutable p be a new Party.\nAdd \"Alice\" to p's guests.\nIf p's guests contains \"Alice\":\n    Show \"found\".\nOtherwise:\n    Show \"missing\".\nShow length of p's guests."),
        ("crdt_shared_seq", "## Definition\nA Document is Shared and has:\n    a lines, which is a SharedSequence of Text.\n\n## Main\nLet mutable d be a new Document.\nAppend \"Line 1\" to d's lines.\nAppend \"Line 2\" to d's lines.\nShow length of d's lines."),
        ("crdt_set_add_wins", "## Definition\nA Party is Shared and has:\n    a guests, which is a SharedSet of Text.\n\n## Main\nLet mutable a be a new Party.\nLet mutable b be a new Party.\nAdd \"X\" to a's guests.\nAdd \"X\" to b's guests.\nRemove \"X\" from a's guests.\nMerge b into a.\nIf a's guests contains \"X\":\n    Show \"present\".\nOtherwise:\n    Show \"absent\"."),
        // counter CRDT through the PE — the struct-mutating `Increase`/`Decrease`/`Merge`
        // must force the struct dynamic so the residual still drives the counter.
        ("crdt_counter", "## Definition\nA Game is Shared and has:\n    a score, which is a Tally.\n\n## Main\nLet mutable g be a new Game.\nIncrease g's score by 100.\nDecrease g's score by 30.\nShow g's score."),
        ("crdt_counter_merge", "## Definition\nA Counter is Shared and has:\n    a points, which is ConvergentCount.\n\n## Main\nLet mutable a be a new Counter.\nLet mutable b be a new Counter.\nIncrease a's points by 10.\nIncrease b's points by 5.\nMerge b into a.\nShow a's points."),
        // OR-Set over Int elements (not just Text) + remove path.
        ("crdt_set_int_remove", "## Definition\nA Bag is Shared and has:\n    a items, which is a SharedSet of Int.\n\n## Main\nLet mutable bag be a new Bag.\nAdd 3 to bag's items.\nAdd 9 to bag's items.\nRemove 3 from bag's items.\nIf bag's items contains 9:\n    Show \"has-9\".\nOtherwise:\n    Show \"no-9\".\nShow length of bag's items."),
    ]
}

// ── Sharding ─────────────────────────────────────────────────────────────────
// nextest parallelizes ACROSS tests, never WITHIN one, so each multi-minute
// differential below is split into independent `#[test]` shards that fan across
// otherwise-idle cores. Every shard processes a DISJOINT slice of the SAME single
// source — the curated `differential_corpus()` or the `0..GEN_SEEDS` seed space,
// selected by `index % SHARDS` — so the shards together cover EXACTLY what the
// original single test did. The `*_partition_*` guards prove the tiling (nothing
// dropped or double-counted). Sharding changes scheduling only, never coverage.

/// Number of parallel shards the curated-corpus differentials are split across.
const CORPUS_SHARDS: usize = 4;

/// The indices into `differential_corpus()` owned by `shard` — every `i` with
/// `i % CORPUS_SHARDS == shard`. Derived from the one shared corpus, so a shard
/// can never invent or drop a program.
fn corpus_shard(shard: usize) -> impl Iterator<Item = usize> {
    (0..differential_corpus().len()).filter(move |i| i % CORPUS_SHARDS == shard)
}

/// One shard of the lenient leg: the PE residual's output stream must match the
/// production tree-walker's, for each owned corpus program (value/error class
/// compared leniently — `Nothing` ≡ `Error` at the engine boundary).
fn run_corpus_output_shard(shard: usize) {
    let corpus = differential_corpus();
    let mut checked = 0usize;
    let mut failures = Vec::new();
    for i in corpus_shard(shard) {
        let (name, program) = corpus[i];
        checked += 1;
        let tw = run_treewalk(program);
        let p1 = run_p1(program);
        if tw.output != p1.output {
            failures.push(format!(
                "[{}] output differs:\n  tree-walk: {:?}\n  P1:        {:?}",
                name, tw.output, p1.output
            ));
        }
    }
    assert!(checked > 0, "corpus output shard {shard}/{CORPUS_SHARDS} checked no programs");
    assert!(
        failures.is_empty(),
        "differential corpus divergences (shard {shard}/{CORPUS_SHARDS}, {} of them):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// One shard of the stronger leg: full value+output agreement under the harness's
/// behavioral comparison, for each owned corpus program the oracle can evaluate.
/// Now name-tagged on divergence (the old single test discarded the program name).
fn run_corpus_strict_shard(shard: usize) {
    let corpus = differential_corpus();
    let mut owned = 0usize;
    let mut failures = Vec::new();
    for i in corpus_shard(shard) {
        owned += 1;
        let (name, program) = corpus[i];
        let tw = run_treewalk(program);
        let p1 = run_p1(program);
        // Skip programs the oracle itself errors on (corpus is meant to be well-formed).
        if tw.is_value() {
            if let Some(diff) = behavior_diff(&p1, &tw, CmpMode::Lenient) {
                failures.push(format!("[{name}] {diff}"));
            }
        }
    }
    assert!(owned > 0, "corpus strict shard {shard}/{CORPUS_SHARDS} owns no programs");
    assert!(
        failures.is_empty(),
        "strict differential divergences (shard {shard}/{CORPUS_SHARDS}, {} of them):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

macro_rules! corpus_shards {
    ($($out:ident, $strict:ident => $idx:expr;)+) => {
        $(
            /// Lenient output-stream leg of one corpus shard — see `run_corpus_output_shard`.
            #[test] fn $out() { run_corpus_output_shard($idx); }
            /// Strict value+output leg of one corpus shard — see `run_corpus_strict_shard`.
            #[test] fn $strict() { run_corpus_strict_shard($idx); }
        )+
    };
}
corpus_shards! {
    interp_vs_treewalk_corpus_s0, interp_vs_treewalk_corpus_strict_s0 => 0;
    interp_vs_treewalk_corpus_s1, interp_vs_treewalk_corpus_strict_s1 => 1;
    interp_vs_treewalk_corpus_s2, interp_vs_treewalk_corpus_strict_s2 => 2;
    interp_vs_treewalk_corpus_s3, interp_vs_treewalk_corpus_strict_s3 => 3;
}

/// Coverage guard: the `CORPUS_SHARDS` shards tile `differential_corpus()` exactly
/// — every program owned by exactly one shard, every shard non-empty. Proves the
/// shard fns together check the identical program set the original two corpus
/// tests did (no program dropped or double-counted).
#[test]
fn corpus_partition_tiles_corpus() {
    let len = differential_corpus().len();
    let mut hits = vec![0u32; len];
    for shard in 0..CORPUS_SHARDS {
        for i in corpus_shard(shard) {
            hits[i] += 1;
        }
    }
    assert!(
        hits.iter().all(|&h| h == 1),
        "corpus not tiled exactly once by {CORPUS_SHARDS} shards: {hits:?}"
    );
    for shard in 0..CORPUS_SHARDS {
        assert!(corpus_shard(shard).count() > 0, "corpus shard {shard} owns no programs");
    }
}

/// The generative differential explores `GEN_SEEDS` seeds, split across `GEN_SHARDS`
/// parallel `#[test]` shards by `seed % GEN_SHARDS`. Each seed in `0..GEN_SEEDS`
/// lands in exactly one shard (`generative_partition_tiles_seed_space`), so the
/// shards together explore the identical seed set the original single
/// `generative_differential_arith` test did.
const GEN_SEEDS: u64 = 200;
const GEN_SHARDS: u64 = 10;

/// One shard of the "robust to the point of absurdity" generative fuzzer — every
/// seed in `0..GEN_SEEDS` with `seed % GEN_SHARDS == shard`. Over each randomly-
/// generated well-typed total Int program, the PE residual (run through the
/// tree-walker) must agree with the production tree-walker. Deterministic
/// (seeded), so any failure is reproducible from its seed.
fn run_generative_shard(shard: u64) {
    let mut checked = 0u64;
    let mut failures = Vec::new();
    for seed in (0..GEN_SEEDS).filter(|s| s % GEN_SHARDS == shard) {
        checked += 1;
        let program = gen_program(seed, Shape::RandomArith(seed));
        let tw = run_treewalk(&program);
        let p1 = run_p1(&program);
        if tw.output != p1.output {
            failures.push(format!(
                "[seed {}] output differs:\n  tree-walk: {:?}\n  P1:        {:?}\nprogram:\n{}",
                seed, tw.output, p1.output, program
            ));
            if failures.len() >= 5 {
                break;
            }
        }
    }
    assert!(checked > 0, "generative shard {shard}/{GEN_SHARDS} explored no seeds");
    assert!(
        failures.is_empty(),
        "generative differential divergences (shard {shard}/{GEN_SHARDS}):\n{}",
        failures.join("\n---\n")
    );
}

macro_rules! generative_shards {
    ($($name:ident => $idx:expr;)+) => {
        $(
            /// One generative-differential shard — see `run_generative_shard`.
            #[test] fn $name() { run_generative_shard($idx); }
        )+
    };
}
generative_shards! {
    generative_differential_arith_s0 => 0;
    generative_differential_arith_s1 => 1;
    generative_differential_arith_s2 => 2;
    generative_differential_arith_s3 => 3;
    generative_differential_arith_s4 => 4;
    generative_differential_arith_s5 => 5;
    generative_differential_arith_s6 => 6;
    generative_differential_arith_s7 => 7;
    generative_differential_arith_s8 => 8;
    generative_differential_arith_s9 => 9;
}

/// Coverage guard: the `GEN_SHARDS` shards tile `0..GEN_SEEDS` exactly — every
/// seed owned by exactly one shard, every shard non-empty. Proves the shard fns
/// explore the identical seed set the original single test did.
#[test]
fn generative_partition_tiles_seed_space() {
    let mut hits = vec![0u32; GEN_SEEDS as usize];
    for shard in 0..GEN_SHARDS {
        for seed in (0..GEN_SEEDS).filter(|s| s % GEN_SHARDS == shard) {
            hits[seed as usize] += 1;
        }
    }
    assert!(
        hits.iter().all(|&h| h == 1),
        "seed space 0..{GEN_SEEDS} not tiled exactly once by {GEN_SHARDS} shards"
    );
    for shard in 0..GEN_SHARDS {
        let n = (0..GEN_SEEDS).filter(|s| s % GEN_SHARDS == shard).count();
        assert!(n > 0, "generative shard {shard}/{GEN_SHARDS} would explore no seeds");
    }
}
