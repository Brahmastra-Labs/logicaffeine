//! ★ LOCK — a `Seq of Int` whose elements can outgrow a machine word is exact on AOT-Rust, and a list that cannot keeps
//! its words. ★
//!
//! The Rust tier holds a list of `Int` as machine words (`Vec<i64>`, a fixed `[i64; N]`, a shared `LogosSeq<i64>`).
//! A list some store into may leave the word, a store the range oracle cannot prove stays inside it, is an
//! `IntVec` instead: words until the first value past the word is stored, then exact integers. The decision is taken
//! for each flow class of sequences (`analysis::seq_flow`): every local, parameter and return a list moves between by
//! a binding, a `copy of`, a `Swap`, an argument or a return.
//!
//! - a list that only an item read, `length of`, `contains`, `Push`, `Set item`, `Repeat` and `Show` see is exact,
//!   wherever it travels: through a function 400 times, into an alias, from a literal argument, back from a return,
//!   across a `Swap`;
//! - a list that reaches anything else (`Pop`, a slice, a closure, a builtin, a record field, another body's binding)
//!   holds words: the program prints the tree-walker's bytes, or stops with the overflow sentence after printing a
//!   prefix of them, and never prints anything else;
//! - a list whose every store the oracle proves stays in the word keeps its machine words.
//!
//! Every row runs on a Debug build and a Release build of the generated program.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test rust_exact_seq_flow_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::time::Duration;

use common::RustProfile;
use logicaffeine_compile::compile::{compile_program_full, compile_to_rust, tw_lines};

/// The sentence a value past the word stops the Rust tier with where it holds a machine word.
const PAST_THE_WORD: &str = "does not fit a 64-bit Int";

/// What the program printed under one profile, and what it said if it failed.
struct Run {
    profile: RustProfile,
    output: String,
    failure: Option<String>,
}

/// `src` built and run under both profiles, with `args` on its command line.
fn on_rust(src: &str, args: &[&str]) -> Vec<Run> {
    let compiled = compile_program_full(src).unwrap_or_else(|e| panic!("the Rust generator refused ({e:?}):\n{src}"));
    [RustProfile::Debug, RustProfile::Release]
        .into_iter()
        .map(|profile| {
            let built = common::build_generated(&compiled.rust_code, &compiled.dependencies, profile)
                .unwrap_or_else(|why| {
                    panic!("the Rust ({profile:?}) does not build:\n{why}\n{src}\n{}", compiled.rust_code)
                });
            let run = common::run_within(&built.binary, args, Duration::from_secs(60));
            assert!(!run.timed_out, "the Rust ({profile:?}) still runs after 60 s:\n{src}");
            Run { profile, output: run.stdout, failure: (!run.success).then_some(run.stderr) }
        })
        .collect()
}

/// What the tree-walker prints for `src`, which must run clean.
fn reference(src: &str) -> String {
    let tw = tw_lines(src);
    assert_eq!(tw.error, None, "the tree-walker refused:\n{src}");
    tw.lines.iter().map(|line| format!("{line}\n")).collect()
}

/// What the tree-walker prints for `src` run with the one argument `arg`.
fn reference_with(src: &str, arg: &str) -> String {
    let tw = logicaffeine_compile::compile::tw_lines_with_args(src, &["main".to_string(), arg.to_string()]);
    assert_eq!(tw.error, None, "the tree-walker refused:\n{src}");
    tw.lines.iter().map(|line| format!("{line}\n")).collect()
}

/// The Rust tier prints exactly what the tree-walker prints, under both profiles.
fn exact_on_rust(src: &str) {
    let expected = reference(src);
    for run in on_rust(src, &[]) {
        assert_eq!(run.failure, None, "the Rust ({:?}) failed:\n{src}\n{}", run.profile, run.output);
        assert_eq!(run.output, expected, "the Rust ({:?})'s answer:\n{src}", run.profile);
    }
}

/// The Rust tier prints what the tree-walker prints, or stops with the overflow sentence after printing a prefix of
/// it, under both profiles.
fn exact_or_loud_on_rust(src: &str, args: &[&str], expected: &str) {
    for run in on_rust(src, args) {
        match run.failure {
            None => assert_eq!(run.output, expected, "the Rust ({:?})'s answer:\n{src}", run.profile),
            Some(why) => {
                assert!(
                    why.contains("Integer overflow: ") && why.contains(PAST_THE_WORD),
                    "the Rust ({:?}) failed some other way ({why}):\n{src}",
                    run.profile
                );
                assert!(
                    expected.starts_with(&run.output),
                    "the Rust ({:?}) printed other bytes before it stopped:\n{}\n{src}",
                    run.profile,
                    run.output
                );
            }
        }
    }
}

#[test]
fn a_list_carried_through_a_function_four_hundred_times_stays_exact() {
    exact_on_rust(
        "## To next (xs: Seq of Int) -> Seq of Int:
    Let mutable out be a new Seq of Int.
    Push (item 1 of xs) + 1 to out.
    Return out.

## Main
Let mutable state be a new Seq of Int.
Push 9223372036854775557 to state.
Let mutable i be 0.
While i is less than 400:
    Set state to next(state).
    Set i to i + 1.
Show item 1 of state.
",
    );
}

/// The list the program grows is as long as its input says, so no fixed buffer holds it: its last element passes the
/// word on the 39th call, where an unchecked `x * 3` would wrap.
#[test]
fn a_list_grown_by_a_loop_over_its_elements_never_wraps() {
    let src = "## To grow (xs: Seq of Int) -> Seq of Int:
    Let mutable out be a new Seq of Int.
    Repeat for x in xs:
        Push x * 3 to out.
    Return out.

## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable state be a new Seq of Int.
Let mutable i be 1.
While i is at most n:
    Push i to state.
    Set i to i + 1.
Let mutable k be 0.
While k is less than 45:
    Set state to grow(state).
    Set k to k + 1.
Show item 2 of state.
Show length of state.
";
    let three_to_the_45: i128 = 2_954_312_706_550_833_698_643;
    exact_or_loud_on_rust(src, &["4"], &format!("{}\n4\n", 2 * three_to_the_45));
}

/// A loop over a parameter's list binds each element as the `Int` it is: arithmetic on it is checked or exact, never
/// a raw machine-word operator that wraps in a release build.
#[test]
fn arithmetic_on_a_loop_variable_over_a_parameters_list_never_wraps() {
    let src = "## To f (xs: Seq of Int) -> Int:
    Let mutable t be 0.
    Repeat for y in xs:
        Set t to t + y * 2.
    Return t.

## To g (xs: Seq of Int):
    Repeat for x in xs:
        Show x * 3.
        Show x + 1000.

## Main
Show f([5, 7]).
Call g with [4, 9223372036854775000].
Show f([9223372036854775000, 5]).
";
    exact_or_loud_on_rust(src, &[], &reference(src));
}

#[test]
fn a_list_built_to_a_fixed_length_with_a_value_past_the_word_is_exact() {
    exact_on_rust(
        "## To pack (a: Int) -> Seq of Int:
    Let mutable out be a new Seq of Int.
    Push a to out.
    Push a * a to out.
    Push a * a * a to out.
    Return out.

## Main
Let d be pack(4294967296).
Show item 3 of d.
Show (item 1 of d) + (item 2 of d).
",
    );
}

#[test]
fn an_alias_and_its_source_read_their_elements_alike() {
    exact_on_rust(
        "## Main
Let mutable xs be [1, 2].
Let mutable ys be xs.
Push 2 ** 70 to ys.
Show ys.
Show xs.
Show item 3 of ys.
Show item 1 of xs.
",
    );
}

#[test]
fn a_literal_handed_straight_to_a_function_is_read_exactly_there() {
    let src = "## To first (xs: Seq of Int) -> Int:
    Return item 1 of xs.

## Main
Show first([2 ** 70, 1]).
Show first([3, 4]).
";
    exact_or_loud_on_rust(src, &[], &reference(src));
}

#[test]
fn a_list_returned_with_a_value_past_the_word_reads_back_exactly() {
    exact_on_rust(
        "## To grow (xs: Seq of Int) -> Seq of Int:
    Let mutable out be copy of xs.
    Push 2 ** 64 to out.
    Return out.

## Main
Let ys be grow([1, 2]).
Show ys.
Show length of ys.
Show item 3 of ys.
",
    );
}

#[test]
fn a_swap_carries_the_elements_both_ways() {
    exact_on_rust(
        "## Main
Let mutable a be [2 ** 70].
Let mutable b be [5].
Swap a and b.
Show a.
Show b.
Show item 1 of b.
",
    );
}

#[test]
fn contains_repeat_and_set_item_read_and_write_the_elements() {
    let src = "## Main
Let mutable xs be [1, 2, 3].
Push 2 ** 70 to xs.
Show xs contains 2 ** 70.
Show xs contains 4.
Set item 1 of xs to 2 ** 80.
Let mutable total be 0.
Repeat for x in xs:
    Set total to total + x.
Show total.
Show xs.
";
    exact_or_loud_on_rust(src, &[], &reference(src));
}

#[test]
fn contains_and_set_item_are_exact_on_a_promoted_list() {
    exact_on_rust(
        "## Main
Let mutable xs be [1, 2, 3].
Push 2 ** 70 to xs.
Show xs contains 2 ** 70.
Show xs contains 4.
Set item 1 of xs to 2 ** 80.
Show item 1 of xs.
Show xs.
",
    );
}

#[test]
fn a_list_promotes_at_its_first_value_past_the_word_and_stays_exact() {
    exact_on_rust(
        "## Main
Let mutable xs be [9223372036854775806, 5].
Set item 1 of xs to (item 1 of xs) + 1.
Show xs.
Set item 1 of xs to (item 1 of xs) + 1.
Show item 1 of xs.
Show item 2 of xs.
Show xs.
Push (item 1 of xs) + (item 2 of xs) to xs.
Show item 3 of xs.
Show xs contains 5.
Show length of xs.
",
    );
}

#[test]
fn a_list_read_into_a_word_local_in_a_loop_builds_and_counts() {
    exact_on_rust(
        "## Main
Let mutable counts be [0, 0, 0].
Let mutable i be 0.
While i is less than 9:
    Set item ((i % 3) + 1) of counts to (item ((i % 3) + 1) of counts) + 1.
    Set i to i + 1.
Let mutable top be 0.
Let mutable j be 1.
While j is at most 3:
    If item j of counts is greater than top:
        Set top to item j of counts.
    Set j to j + 1.
Show top.
Show counts.
",
    );
}

#[test]
fn a_promoted_element_read_where_a_word_holds_it_stops_loudly() {
    let src = "## Main
Let mutable xs be [1].
Push 2 ** 70 to xs.
Let w be abs(item 2 of xs).
Show w.
";
    exact_or_loud_on_rust(src, &[], &reference(src));
}

#[test]
fn a_byte_buffer_a_builtin_reads_keeps_its_words() {
    exact_on_rust("## Main\nShow toBase(255, 16).\nShow toBase(1000000007, 36).\nShow toBinary(10).\n");
}

#[test]
fn a_list_a_pop_reads_keeps_words() {
    let src = "## Main
Let mutable xs be [1].
Push 2 ** 70 to xs.
Pop from xs into v.
Show v.
";
    exact_or_loud_on_rust(src, &[], &reference(src));
}

#[test]
fn a_list_a_slice_reads_keeps_words() {
    let src = "## Main
Let mutable xs be [1].
Push 2 ** 70 to xs.
Show items 1 through 2 of xs.
";
    exact_or_loud_on_rust(src, &[], &reference(src));
}

#[test]
fn a_list_a_closure_reads_keeps_words() {
    let src = "## Main
Let mutable xs be [1].
Push 2 ** 70 to xs.
Let pick be (n: Int) -> item n of xs.
Show pick(2).
";
    exact_or_loud_on_rust(src, &[], &reference(src));
}

#[test]
fn a_list_a_builtin_made_keeps_words() {
    let src = "## Main
Let mutable bs be text_bytes(\"AB\").
Push 2 ** 70 to bs.
Show item 3 of bs.
Show item 1 of bs.
";
    exact_or_loud_on_rust(src, &[], &reference(src));
}

#[test]
fn a_list_in_a_record_field_keeps_words() {
    let src = "## A Bag has:
    An items: Seq of Int.

## Main
Let mutable b be a new Bag with items [1, 2].
Push 2 ** 70 to b's items.
Show item 3 of b's items.
Show b's items.
";
    exact_or_loud_on_rust(src, &[], &reference(src));
}

#[test]
fn a_list_shared_by_a_word_reader_and_an_exact_store_never_prints_other_bytes() {
    let src = "## To head (xs: Seq of Int) -> Int:
    Return item 1 of xs.

## Main
Let mutable xs be [1].
Push 2 ** 70 to xs.
Show head(xs).
Show head(text_bytes(\"A\")).
Pop from xs into v.
Show v.
";
    exact_or_loud_on_rust(src, &[], &reference(src));
}

#[test]
fn a_record_fields_list_read_into_a_local_keeps_words() {
    let src = "## A Bag has:
    An items: Seq of Int.

## Main
Let mutable b be a new Bag with items [1, 2].
Let mutable xs be b's items.
Push 2 ** 70 to xs.
Show item 1 of xs.
Show xs.
";
    exact_or_loud_on_rust(src, &[], &reference(src));
}

#[test]
fn a_list_another_body_pushes_into_keeps_words() {
    let src = "## To grow:
    Push 2 ** 70 to xs.

## Main
Let mutable xs be [1].
Call grow.
Show item 1 of xs.
Show xs.
";
    exact_or_loud_on_rust(src, &[], &reference(src));
}

#[test]
fn a_literal_past_the_word_bound_to_a_list_a_slice_reads_stops_loudly() {
    let src = "## Main
Let mutable xs be [2 ** 70, 1].
Show items 1 through 1 of xs.
";
    exact_or_loud_on_rust(src, &[], &reference(src));
}

/// The fast forms stay: a list every store into which the oracle proves stays in the word is never an `IntVec`.
#[test]
fn a_list_whose_stores_the_oracle_proves_fit_keeps_its_words() {
    let src = "## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable xs be a new Seq of Int.
Let mutable i be 0.
While i is less than 1000:
    Push (i * i) % 1000 to xs.
    Set i to i + 1.
Let mutable total be 0.
Repeat for x in xs:
    Set total to total + x.
Show total + n.
";
    let rust = compile_to_rust(src).expect("Rust for the program");
    assert!(!rust.contains("IntVec"), "a list of proven words is held as words:\n{rust}");
    exact_or_loud_on_rust(src, &["0"], "461500\n");
}

/// An `IntVec` names itself where the decision makes one, and nowhere else: the four-hundred-call list is one, the
/// byte buffer a builtin fills is not.
#[test]
fn the_decision_names_the_lists_it_makes_exact() {
    let carried = compile_to_rust(
        "## To next (xs: Seq of Int) -> Seq of Int:
    Let mutable out be a new Seq of Int.
    Push (item 1 of xs) + 1 to out.
    Return out.

## Main
Let mutable state be a new Seq of Int.
Push 9223372036854775557 to state.
Set state to next(state).
Show item 1 of state.
",
    )
    .expect("Rust for the program");
    assert!(carried.contains("IntVec"), "the carried list is exact:\n{carried}");
    let bytes = compile_to_rust("## Main\nShow toBase(255, 16).\n").expect("Rust for the program");
    assert!(!bytes.contains("IntVec"), "a builtin's byte buffer keeps its words:\n{bytes}");
}

/// Only a list of `Int` is ever an `IntVec`: a list of reals, of texts or of 32-bit words a store grows with arithmetic
/// that would promote a list of `Int` keeps its own representation, and runs as on the tree-walker.
#[test]
fn a_list_of_anything_but_int_is_never_an_int_vec() {
    for src in [
        "## To grow (xs: Seq of Real) -> Seq of Real:
    Let mutable out be a new Seq of Real.
    Repeat for x in xs:
        Push x * 1.5 to out.
    Return out.

## Main
Let mutable state be [1.0, 2.0].
Set state to grow(state).
Show state.
",
        "## To shout (xs: Seq of Text) -> Seq of Text:
    Let mutable out be a new Seq of Text.
    Repeat for x in xs:
        Push x + \"!\" to out.
    Return out.

## Main
Let mutable words be [\"a\", \"b\"].
Set words to shout(words).
Show words.
Show words contains \"a!\".
",
        "## To bump (xs: Seq of Word32) -> Seq of Word32:
    Let mutable out be a new Seq of Word32.
    Repeat for x in xs:
        Push x + word32(4294967295) to out.
    Return out.

## Main
Let mutable ws be a new Seq of Word32.
Push word32(7) to ws.
Set ws to bump(ws).
Show ws.
",
    ] {
        let rust = compile_to_rust(src).expect("Rust for the program");
        assert!(!rust.contains("IntVec"), "only a list of Int is an IntVec:\n{src}\n{rust}");
        exact_on_rust(src);
    }
}

/// A function that writes an item of the list it is handed and returns it writes the caller's list in place; the loop
/// over that list walks it as it was on entry.
#[test]
fn a_function_writing_an_item_of_its_list_hands_the_list_back() {
    exact_on_rust(
        "## To bump (xs: Seq of Int) -> Seq of Int:
    Set item 3 of xs to item 3 of xs + 100.
    Return xs.

## Main
Let mutable xs be [1, 2, 3].
Repeat for x in xs:
    Set xs to bump(xs).
    Show x.
Show xs.
",
    );
}

/// A list the exact extension holds (it stores an infinitesimal) is the extension's value, never an `IntVec`.
#[test]
fn a_list_the_exact_extension_holds_keeps_its_values() {
    exact_on_rust(
        "## Main
Let dt be an infinitesimal.
Let mutable n be 0.
While n < 10:
    Set n to n + 1.
Show n.
Set n to n + dt.
Show n.
Let mutable total be 0.
Repeat for i from 1 to 100:
    Set total to total + i.
Show total.
Set total to total * dt.
Show total.
Let mutable xs be a new Seq of Int.
Repeat for i from 1 to 5:
    Push i * i to xs.
Show xs.
Push dt to xs.
Show length of xs.
",
    );
}

/// A counter one loop declares and later loops reuse, one of them filling a list that may hold integers past the word,
/// stays one declared binding across all of them.
#[test]
fn a_counter_loops_share_while_they_fill_an_exact_list_stays_declared() {
    exact_on_rust(
        "## To bump (xs: Seq of Int) -> Seq of Int:
    Let mutable r be xs.
    Set item 1 of r to item 1 of r + 1.
    Return r.
## To run () -> Int:
    Let mutable m be a new Map of Int to Float.
    Let mutable k be 0.
    While k is less than 30:
        Set item k of m to k * 1.5.
        Set k to k + 1.
    Let mutable arr be a new Seq of Int.
    Set k to 0.
    While k is less than 30:
        Push 0 to arr.
        Set k to k + 1.
    Let mutable s be 0.0.
    Let mutable i be 0.
    While i is less than 5000:
        Set arr to bump(arr).
        Set s to s + item 1 of m.
        Set i to i + 1.
    Return item 1 of arr.
## Main
Show run().
",
    );
}

/// A list a loop declares and fills with one value, then hands to a list that may hold integers past the word, is an
/// `IntVec` from its own declaration: no fill rewrite declares it a machine-word list.
#[test]
fn a_list_a_loop_fills_then_hands_to_an_exact_list_is_declared_as_one() {
    exact_on_rust(
        "## Main
Let cols be 8.
Let mutable prev be a new Seq of Int.
Let mutable k be 0.
While k is less than cols:
    Push 1 to prev.
    Set k to k + 1.
Let mutable i be 0.
While i is less than 6:
    Let mutable curr be a new Seq of Int.
    Let mutable f be 0.
    While f is less than cols:
        Push 0 to curr.
        Set f to f + 1.
    Let mutable w be 0.
    While w is less than cols:
        Set item (w + 1) of curr to item (w + 1) of prev + w.
        Set w to w + 1.
    Set prev to curr.
    Set i to i + 1.
Let mutable total be 0.
Set k to 0.
While k is less than cols:
    Set total to total + item (k + 1) of prev.
    Set k to k + 1.
Show total.
",
    );
}

/// A counting list keeps its words: every item starts at zero and one counting loop adds one to an item at most once a
/// pass, so no item passes the loop's trip count, a word.
#[test]
fn a_list_a_counting_loop_increments_keeps_its_words() {
    let src = "## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable counts be a new Seq of Int.
Let mutable k be 0.
While k is less than 10:
    Push 0 to counts.
    Set k to k + 1.
Let mutable i be 0.
While i is less than n:
    Let v be (i * 7) % 10.
    Set item (v + 1) of counts to (item (v + 1) of counts) + 1.
    Set i to i + 1.
Show counts.
";
    let rust = compile_to_rust(src).expect("Rust for the program");
    assert!(!rust.contains("IntVec"), "a counting list is held as words:\n{rust}");
    exact_or_loud_on_rust(src, &["1000"], "[100, 100, 100, 100, 100, 100, 100, 100, 100, 100]\n");
}

/// A list a bounded counter less one fills keeps its words: the counter is a machine word at least one, so the value
/// never leaves the word.
#[test]
fn a_list_a_counter_less_one_fills_keeps_its_words() {
    let src = "## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable positions be a new Seq of Int.
Let mutable i be 1.
While i is at most n:
    If i % 3 equals 0:
        Push i - 1 to positions.
    Set i to i + 1.
Show positions.
";
    let rust = compile_to_rust(src).expect("Rust for the program");
    assert!(!rust.contains("IntVec"), "a list a counter less one fills is held as words:\n{rust}");
    exact_or_loud_on_rust(src, &["10"], "[2, 5, 8]\n");
}

/// The counting certificate holds only where its proof does: an increment a nested loop makes, two increments a pass,
/// or an increment of a list a caller hands in may pass any count the input sets, so each list may still promote.
/// The natives a program that reads its command line declares.
const ARGS: &str = "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\n";

/// The Rust tier prints exactly what the tree-walker prints for `src` run with the one argument `arg`, under both
/// profiles.
fn exact_on_rust_with(src: &str, arg: &str) {
    let expected = reference_with(src, arg);
    for run in on_rust(src, &[arg]) {
        assert_eq!(run.failure, None, "the Rust ({:?}) failed:\n{src}\n{}", run.profile, run.output);
        assert_eq!(run.output, expected, "the Rust ({:?})'s answer:\n{src}", run.profile);
    }
}

#[test]
fn counting_loops_a_path_runs_one_of_keep_their_words() {
    for (shape, body) in [
        (
            "a counting loop in each branch of an `If`",
            "If n is greater than 5:\n    While i is less than n:\n        \
             Set item ((i % 3) + 1) of counts to (item ((i % 3) + 1) of counts) + 1.\n        Set i to i + 1.\n\
             Otherwise:\n    While i is less than n:\n        \
             Set item ((i % 2) + 1) of counts to (item ((i % 2) + 1) of counts) + 1.\n        Set i to i + 1.\n",
        ),
        (
            "an increment in each branch of an `If` a pass takes",
            "While i is less than n:\n    If i % 2 equals 0:\n        Set item 1 of counts to (item 1 of counts) + 1.\n    \
             Otherwise:\n        Set item 2 of counts to (item 2 of counts) + 1.\n    Set i to i + 1.\n",
        ),
    ] {
        let src = format!(
            "{ARGS}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\n\
             Let mutable counts be a new Seq of Int.\nPush 0 to counts. Push 0 to counts. Push 0 to counts.\n\
             Let mutable i be 0.\n{body}Show counts.\n"
        );
        let rust = compile_to_rust(&src).expect("Rust for the program");
        assert!(!rust.contains("IntVec"), "{shape}: one path runs one counting loop, so the list keeps its words:\n{rust}");
        exact_on_rust_with(&src, "9");
    }
}

#[test]
fn a_counted_list_that_starts_past_zero_is_exact() {
    let src = format!(
        "{ARGS}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\n\
         Let mutable counts be [9223372036854775807, 0].\nLet mutable i be 0.\nWhile i is less than n:\n    \
         Set item ((i % 2) + 1) of counts to (item ((i % 2) + 1) of counts) + 1.\n    Set i to i + 1.\nShow counts.\n"
    );
    let rust = compile_to_rust(&src).expect("Rust for the program");
    assert!(rust.contains("IntVec"), "a list born past zero is no counter's, so it may promote:\n{rust}");
    exact_on_rust_with(&src, "3");
}

#[test]
fn a_counted_copy_of_a_list_a_caller_hands_in_is_exact() {
    let src = format!(
        "{ARGS}## To tally (c: Seq of Int) and (n: Int) -> Seq of Int:\n    Let mutable d be c.\n    \
         Let mutable a be 0.\n    While a is less than n:\n        \
         Set item ((a % 2) + 1) of d to (item ((a % 2) + 1) of d) + 1.\n        Set a to a + 1.\n    Return d.\n\n\
         ## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\nLet mutable c be [0, 0].\n\
         Let mutable k be 0.\nWhile k is less than 3:\n    Set c to tally(c, n).\n    Set k to k + 1.\nShow c.\n"
    );
    let rust = compile_to_rust(&src).expect("Rust for the program");
    assert!(
        rust.contains("IntVec"),
        "each call counts on from the list its caller hands in, so no one loop bounds an item:\n{rust}"
    );
    exact_on_rust_with(&src, "5");
}

#[test]
fn a_list_declared_beside_another_before_their_fill_loop_is_exact() {
    let src = format!(
        "{ARGS}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\n\
         Let mutable a be a new Seq of Int.\nLet mutable b be a new Seq of Int.\nLet mutable i be 0.\n\
         While i is less than n:\n    Push i to a.\n    Push 9223372036854775800 to b.\n    Set i to i + 1.\n\
         Set item 1 of b to (item 1 of b) + item n of a.\nShow item 1 of b.\nShow length of a.\n"
    );
    let rust = compile_to_rust(&src).expect("Rust for the program");
    assert!(rust.contains("IntVec"), "`b` may pass the word:\n{rust}");
    exact_on_rust_with(&src, "10");
}

#[test]
fn a_list_a_fill_loop_builds_starts_as_that_many_words() {
    let src = format!(
        "{ARGS}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\n\
         Let mutable d be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than n:\n    Push 7 to d.\n    \
         Set i to i + 1.\nSet item 2 of d to (item 2 of d) * 2000000000000000000.\nShow d.\nShow i.\n"
    );
    let rust = compile_to_rust(&src).expect("Rust for the program");
    assert!(
        rust.contains("IntVec::from_words(vec![7; "),
        "a fill of words is one allocation, as a list of words' is:\n{rust}"
    );
    exact_on_rust_with(&src, "3");
}

#[test]
fn degree_counts_an_inner_loop_names_once_a_pass_keep_their_words() {
    let src = format!(
        "{ARGS}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\n\
         Let mutable deg be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than n:\n    Push 0 to deg.\n    \
         Set i to i + 1.\nLet mutable p be 1.\nWhile p is at most 5:\n    Set i to 0.\n    While i is less than n:\n        \
         If (i * p) % 3 is not 1:\n            Let c be item (i + 1) of deg.\n            \
         Set item (i + 1) of deg to c + 1.\n        Set i to i + 1.\n    Set p to p + 1.\nShow deg.\n"
    );
    let rust = compile_to_rust(&src).expect("Rust for the program");
    assert!(
        !rust.contains("IntVec"),
        "an inner loop names each item once a run, and the outer loop runs five times, so every count is a word:\n{rust}"
    );
    exact_on_rust_with(&src, "7");
}

#[test]
fn counts_no_one_loop_bounds_may_promote() {
    for (shape, body) in [
        (
            "an inner loop under two outer loops",
            "Let mutable q be 0.\nWhile q is less than n:\n    Set p to 0.\n    While p is less than n:\n        \
             Set i to 0.\n        While i is less than 2:\n            Let c be item (i + 1) of deg.\n            \
             Set item (i + 1) of deg to c + 1.\n            Set i to i + 1.\n        Set p to p + 1.\n    Set q to q + 1.\n",
        ),
        (
            "a store between the read and the increment",
            "While p is less than n:\n    Set i to 0.\n    While i is less than 2:\n        Let c be item (i + 1) of deg.\n        \
             Set item (i + 1) of deg to 5.\n        Set item (i + 1) of deg to c + 1.\n        Set i to i + 1.\n    \
             Set p to p + 1.\n",
        ),
    ] {
        let src = format!(
            "{ARGS}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\n\
             Let mutable deg be a new Seq of Int.\nPush 0 to deg. Push 0 to deg.\nLet mutable i be 0.\n\
             Let mutable p be 0.\n{body}Show deg.\n"
        );
        let rust = compile_to_rust(&src).expect("Rust for the program");
        assert!(rust.contains("IntVec"), "{shape}: no one loop bounds the counts, so they may promote:\n{rust}");
        exact_on_rust_with(&src, "3");
    }
}

#[test]
fn a_list_that_may_promote_reads_a_proven_index_unchecked() {
    let src = format!(
        "{ARGS}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\n\
         Let mutable d be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than n:\n    Push i to d.\n    \
         Set i to i + 1.\nSet item 1 of d to (item 1 of d) + 9223372036854775807.\nLet mutable s be 0.\n\
         Set i to 1.\nWhile i is at most n:\n    Set s to (s + item i of d) % 1000.\n    Set i to i + 1.\nShow s.\nShow d.\n"
    );
    let rust = compile_to_rust(&src).expect("Rust for the program");
    assert!(
        rust.contains("get_word_unchecked"),
        "an index the oracle proves is read without the check a `Vec<i64>` read skips too:\n{rust}"
    );
    exact_or_loud_on_rust(&src, &["4"], &reference_with(&src, "4"));
}

#[test]
fn a_list_its_fill_loop_pushes_starts_with_room_for_them() {
    for (shape, fill) in [
        (
            "a list declared beside the one the loop fills first",
            "Let mutable a be a new Seq of Int.\nLet mutable d be a new Seq of Int.\nLet mutable i be 0.\n\
             While i is less than n:\n    Push i to a.\n    Push 0 to d.\n    Set i to i + 1.\n",
        ),
        (
            "a list the loop fills with a value of its counter",
            "Let mutable d be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than n:\n    Push i * 3 to d.\n    \
             Set i to i + 1.\n",
        ),
        (
            "a list the loop pushes twice a pass",
            "Let mutable d be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than n:\n    Push i to d.\n    \
             Push 0 - i to d.\n    Set i to i + 1.\n",
        ),
    ] {
        let src = format!(
            "{ARGS}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\n{fill}\
             Set item 2 of d to (item 2 of d) + 9223372036854775807.\nSet item 1 of d to (item 1 of d) - n.\nShow d.\n"
        );
        let rust = compile_to_rust(&src).expect("Rust for the program");
        assert!(rust.contains("IntVec"), "{shape}: `d` may pass the word:\n{rust}");
        assert!(
            !rust.contains("IntVec::new()"),
            "{shape}: the loop pushes a counted number of words, which `d` starts with room for, as a list of words \
             does:\n{rust}"
        );
        exact_on_rust_with(&src, "4");
    }
}

/// A program that runs `body` over the list `d`, which `Let mutable d be …` (`start`) binds and `Show d` prints.
fn over_d(start: &str, body: &str) -> String {
    format!(
        "{ARGS}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\nLet mutable d be {start}.\n\
         {body}Show d.\n"
    )
}

#[test]
fn a_chain_over_items_of_the_list_it_is_stored_into_runs_on_its_words() {
    for (shape, body) in [
        (
            "an item one past another, at an index the oracle proves",
            "Let mutable i be 1.\nWhile i is less than n:\n    Set item (i + 1) of d to (item i of d) + 1.\n    \
             Set i to i + 1.\n",
        ),
        (
            "an item one past another, at an index the program reads",
            "Let k be n - 3.\nSet item (k + 1) of d to (item k of d) + n.\nSet item n of d to (item (k + 1) of d) + 1.\n",
        ),
        (
            "a sum of two items",
            "Set item n of d to (item 1 of d) + (item 2 of d).\n",
        ),
        (
            "a product of an item and a word",
            "Set item 3 of d to (item 1 of d) * n.\n",
        ),
        (
            "a copy of an item, before and after another store promotes the list",
            "Set item 3 of d to item 1 of d.\nSet item 2 of d to (item 1 of d) + n.\nSet item n of d to item 2 of d.\n",
        ),
        (
            "a push of an item past another",
            "Push (item n of d) + 1 to d.\nPush (item (length of d) of d) * 2 to d.\n",
        ),
    ] {
        let src = over_d("[9223372036854775805, 0, 0, 0]", body);
        let rust = compile_to_rust(&src).expect("Rust for the program");
        assert!(
            rust.contains(".words_mut()"),
            "{shape}: the store reads and writes the list's words through one view of them:\n{rust}"
        );
        assert!(
            !rust.contains("is_exact()"),
            "{shape}: the view is the one check that the list holds words:\n{rust}"
        );
        exact_on_rust_with(&src, "4");
    }
}

#[test]
fn a_chain_on_words_is_exact_at_every_edge_of_the_word() {
    for (start, body) in [
        ("[9223372036854775806, 0]", "Set item 2 of d to (item 1 of d) + 1.\nSet item 1 of d to (item 2 of d) + n.\n"),
        ("[-9223372036854775807, 0]", "Set item 2 of d to (item 1 of d) - 1.\nSet item 1 of d to (item 2 of d) - n.\n"),
        ("[-9223372036854775807, 0]", "Set item 2 of d to (item 1 of d) - 1.\nSet item 1 of d to (item 2 of d) / (0 - 1).\n"),
        ("[4611686018427387904, 0]", "Set item 2 of d to (item 1 of d) * 2.\nSet item 1 of d to (item 2 of d) * n.\n"),
        ("[3037000500, 0]", "Set item 2 of d to (item 1 of d) * (item 1 of d).\n"),
        ("[9223372036854775807, 0]", "Set item 2 of d to ((item 1 of d) + n) - n.\n"),
        ("[9223372036854775807, 9]", "Set item 2 of d to ((item 1 of d) * 3) % 7.\n"),
        ("[9223372036854775807]", "Push (item 1 of d) + n to d.\nPush ((item 2 of d) * 0) + 5 to d.\n"),
    ] {
        let src = over_d(start, body);
        exact_on_rust_with(&src, "2");
    }
}

#[test]
fn a_chain_over_items_of_another_list_is_stored_on_the_words_while_both_hold_them() {
    let src = format!(
        "{ARGS}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\n\
         Let mutable a be [9223372036854775806, 1, 2].\nLet mutable b be [0, 0, 0].\n\
         Set item 1 of b to (item 1 of a) + 1.\nShow b.\nSet item 2 of a to (item 1 of a) + n.\n\
         Set item 2 of b to (item 2 of a) - n.\nSet item 3 of b to (item 3 of a) * n.\nShow a.\nShow b.\n"
    );
    let rust = compile_to_rust(&src).expect("Rust for the program");
    assert!(rust.contains(".words_mut()"), "a store into `b` writes its words through one view of them:\n{rust}");
    exact_on_rust_with(&src, "2");
}

#[test]
fn a_chain_on_words_divides_by_zero_as_the_tree_walker_does() {
    let src = over_d(
        "[9223372036854775807, 0, 5]",
        "Show item 1 of d.\nSet item 2 of d to (item 1 of d) / (n - 2).\nShow item 2 of d.\n",
    );
    let tw = logicaffeine_compile::compile::tw_lines_with_args(&src, &["main".to_string(), "2".to_string()]);
    let said = tw.error.unwrap_or_else(|| panic!("the tree-walker divided by zero:\n{src}"));
    assert!(said.contains("Division by zero"), "the tree-walker's sentence ({said}):\n{src}");
    let printed: String = tw.lines.iter().map(|line| format!("{line}\n")).collect();
    for run in on_rust(&src, &["2"]) {
        let why = run.failure.unwrap_or_else(|| panic!("the Rust ({:?}) divided by zero:\n{src}", run.profile));
        assert!(why.contains("Division by zero"), "the Rust ({:?}) failed some other way ({why}):\n{src}", run.profile);
        assert_eq!(run.output, printed, "the Rust ({:?}) printed other bytes before it stopped:\n{src}", run.profile);
    }
}

/// A breadth-first walk over `n` nodes from node 1, each node `v` reaching `((v * 2) + e - 1) % n + 1` for `e` of 1
/// and 2: `dist` born `0 - 1` for every node, `Set item 1 of dist to 0`, and `guarded`, the step that visits `u`.
fn breadth_first(guarded: &str) -> String {
    format!(
        "{ARGS}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\n\
         Let mutable dist be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than n:\n    Push 0 - 1 to dist.\n    \
         Set i to i + 1.\nLet mutable queue be a new Seq of Int.\nPush 1 to queue.\nSet item 1 of dist to 0.\n\
         Let mutable front be 1.\nWhile front is at most length of queue:\n    Let v be item front of queue.\n    \
         Let mutable e be 1.\n    While e is at most 2:\n        Let mutable u be ((v * 2) + e - 1) % n + 1.\n\
         {guarded}        Set e to e + 1.\n    Set front to front + 1.\nShow dist.\nShow length of queue.\n"
    )
}

#[test]
fn a_list_each_item_of_which_one_guarded_step_writes_once_keeps_its_words() {
    for (shape, guarded) in [
        (
            "the step under the guard that the item is still the fill",
            "        If item u of dist equals 0 - 1:\n            Set item u of dist to (item v of dist) + 1.\n            \
             Push u to queue.\n",
        ),
        (
            "the fill on the right of the guard, the step one past another item, then a push",
            "        If 0 - 1 equals item u of dist:\n            Set item u of dist to 1 + (item v of dist).\n            \
             Push u to queue.\n",
        ),
        (
            "a push between the guard and the step",
            "        If item u of dist equals 0 - 1:\n            Push u to queue.\n            \
             Set item u of dist to (item v of dist) + 2.\n",
        ),
    ] {
        let src = breadth_first(guarded);
        let rust = compile_to_rust(&src).expect("Rust for the program");
        assert!(
            !rust.contains("dist: logicaffeine_data::IntVec"),
            "{shape}: each item of `dist` is the fill until one step writes it, past the fill, so no item passes the \
             fill's distance from the largest literal plus twice the list's length, which fits a word:\n{rust}"
        );
        exact_on_rust_with(&src, "11");
    }
}

#[test]
fn a_step_an_item_can_take_again_may_promote() {
    for (shape, guarded) in [
        (
            "a store of the fill after the step, which lets the guard pass again",
            "        If item u of dist equals 0 - 1:\n            Set item u of dist to (item v of dist) + 1.\n            \
             Push u to queue.\n        Set item 1 of dist to 0 - 1.\n",
        ),
        (
            "a step under no guard",
            "        If e equals 1:\n            Set item u of dist to (item v of dist) + 1.\n",
        ),
        (
            "a guard on another item",
            "        If item v of dist equals 0 - 1:\n            Set item u of dist to (item v of dist) + 1.\n",
        ),
        (
            "the index moved between the guard and the step",
            "        If item u of dist equals 0 - 1:\n            Set u to (u % n) + 1.\n            \
             Set item u of dist to (item v of dist) + 1.\n",
        ),
        (
            "a guard on a value other than the fill",
            "        If item u of dist equals 0:\n            Set item u of dist to (item v of dist) + 1.\n",
        ),
        (
            "a step past the bound the list's length gives",
            "        If item u of dist equals 0 - 1:\n            \
             Set item u of dist to (item v of dist) + 4611686018427387904.\n            Push u to queue.\n",
        ),
    ] {
        let src = breadth_first(guarded);
        let rust = compile_to_rust(&src).expect("Rust for the program");
        assert!(rust.contains("IntVec"), "{shape}: an item's steps are not bounded, so the list may promote:\n{rust}");
        exact_on_rust_with(&src, "11");
    }
}

#[test]
fn a_guarded_step_over_a_fill_far_below_zero_or_a_list_handed_in_may_promote() {
    let far = breadth_first(
        "        If item u of dist equals 0 - 1:\n            Set item u of dist to (item v of dist) + 1.\n            \
         Push u to queue.\n",
    )
    .replace("Push 0 - 1 to dist.", "Push 0 - 1 to dist.\n    Push 0 - 9223372036854775807 to dist.");
    let rust = compile_to_rust(&far).expect("Rust for the program");
    assert!(rust.contains("IntVec"), "a fill far below zero takes the bound past the word:\n{rust}");
    exact_on_rust_with(&far, "11");
    let handed = format!(
        "{ARGS}## To visit (dist: Seq of Int) and (u: Int) and (v: Int) -> Seq of Int:\n    Let mutable d be dist.\n    \
         If item u of d equals 0 - 1:\n        Set item u of d to (item v of d) + 1.\n    Return d.\n\n\
         ## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\nLet mutable dist be [0, 0 - 1, 0 - 1].\n\
         Set dist to visit(dist, 2, 1).\nSet dist to visit(dist, 3, 2).\nSet dist to visit(dist, 3, n).\nShow dist.\n"
    );
    let rust = compile_to_rust(&handed).expect("Rust for the program");
    assert!(rust.contains("IntVec"), "each call steps the list its caller hands in, born anywhere:\n{rust}");
    exact_on_rust_with(&handed, "2");
}

#[test]
fn a_store_inside_any_block_the_certificate_does_not_walk_counts_against_it() {
    let src = format!(
        "{ARGS}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\n\
         Let mutable deg be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than n:\n    Push 0 to deg.\n    \
         Set i to i + 1.\nSet i to 0.\nWhile i is less than n:\n    Set item (i + 1) of deg to (item (i + 1) of deg) + 1.\n    \
         Set i to i + 1.\nReversibly:\n    Set item 1 of deg to (item 1 of deg) + 9223372036854775807.\nShow deg.\n"
    );
    let rust = compile_to_rust(&src).expect("Rust for the program");
    assert!(
        rust.contains("IntVec"),
        "the store inside `Reversibly:` is no increment a counting loop bounds, so the list may promote:\n{rust}"
    );
    exact_on_rust_with(&src, "3");
}

#[test]
fn a_loop_over_a_list_that_may_promote_runs_on_its_words() {
    let src = format!(
        "{ARGS}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\n\
         Let mutable count be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than n:\n    Push 0 to count.\n    \
         Set i to i + 1.\nLet mutable r be n.\nWhile r is greater than 1:\n    Set item r of count to r.\n    Set r to r - 1.\n\
         Let mutable done be 1.\nLet mutable steps be 0.\nWhile done equals 1:\n    \
         Set item (r + 1) of count to (item (r + 1) of count) - 1.\n    If item (r + 1) of count is greater than 0:\n        \
         Set steps to steps + 1.\n    Otherwise:\n        Set r to r + 1.\n    If r equals n - 1:\n        Set done to 0.\n\
         Show count.\nShow steps.\n"
    );
    let rust = compile_to_rust(&src).expect("Rust for the program");
    assert!(rust.contains("IntVec"), "no certificate bounds a decrement in a loop no counter bounds:\n{rust}");
    assert!(
        rust.contains("if let Some(__wv"),
        "a loop over a list that may promote holds one view of its words, as a loop over words holds a slice:\n{rust}"
    );
    exact_on_rust_with(&src, "6");
}

#[test]
fn a_pass_that_finds_a_value_past_the_word_finishes_on_the_exact_integers() {
    for (shape, start, body, after) in [
        (
            "a step past the word, then a read of the item it stepped",
            "[0, 9223372036854775804, 9223372036854775805]",
            "While i is less than n:\n    Let k be (i % 2) + 2.\n    Set item k of d to (item k of d) + 1.\n    \
             If item k of d is greater than 9223372036854775806:\n        Set seen to seen + 1.\n    Set i to i + 1.\n",
            "",
        ),
        (
            "a store past the word inside `Otherwise`, the pass going on after the `If`",
            "[9223372036854775800, 0, 0]",
            "While i is less than n:\n    If i % 2 equals 0:\n        Set item 3 of d to i.\n    Otherwise:\n        \
             Set item 2 of d to (item 1 of d) + (i * 3).\n    Set seen to seen + item 3 of d.\n    Set i to i + 1.\n",
            "",
        ),
        (
            "a push past the word, then more pushes",
            "[9223372036854775806]",
            "While i is less than n:\n    Push (item (length of d) of d) + 1 to d.\n    Set i to i + 1.\n",
            "",
        ),
        (
            "two lists, the one stored into promoting while the other keeps its words",
            "[9223372036854775800, 1, 2]",
            "While i is less than n:\n    Let k be (i % 2) + 1.\n    Set item k of d to (item k of d) + (item 2 of e).\n    \
             Set item 2 of e to (item 2 of e) + 1.\n    Set i to i + 1.\n",
            "Show e.\n",
        ),
        (
            "a loop whose condition reads the list",
            "[9223372036854775790, 0]",
            "While item 1 of d is less than 9223372036854775807 + 20:\n    Set item 1 of d to (item 1 of d) + 3.\n    \
             Set item 2 of d to (item 2 of d) + 1.\n",
            "",
        ),
    ] {
        let src = format!(
            "{ARGS}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\nLet mutable d be {start}.\n\
             Let mutable e be [0, 0, 0].\nLet mutable i be 0.\nLet mutable seen be 0.\n{body}Show d.\nShow seen.\nShow i.\n{after}"
        );
        let rust = compile_to_rust(&src).expect("Rust for the program");
        assert!(rust.contains("IntVec"), "{shape}: `d` may pass the word:\n{rust}");
        exact_on_rust_with(&src, "7");
    }
}

#[test]
fn a_loop_that_reads_a_list_already_past_the_word_runs_as_written() {
    let src = format!(
        "{ARGS}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\n\
         Let mutable d be [1, 2, 3, 4].\nSet item 2 of d to (item 2 of d) + 9223372036854775807.\nLet mutable s be 0.\n\
         Let mutable i be 1.\nWhile i is at most length of d:\n    Set s to (s + item i of d) % 1000.\n    Set i to i + 1.\n\
         Let mutable big be 0.\nSet i to 1.\nWhile i is at most n:\n    If item ((i % 4) + 1) of d is greater than 9223372036854775807:\n        \
         Set big to big + 1.\n    Set i to i + 1.\nShow s.\nShow big.\nShow d.\n"
    );
    exact_on_rust_with(&src, "9");
}

/// A program that fills `xs` by `body`, a loop over `k` from 1 to `n`, then shows it.
fn filled_by(body: &str) -> String {
    format!(
        "{ARGS}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\n\
         Let mutable xs be a new Seq of Int.\nLet mutable k be 1.\n{body}Show xs.\nShow k.\n"
    )
}

#[test]
fn a_value_of_a_counter_a_loop_bounds_by_its_list_keeps_its_words() {
    for (shape, body) in [
        (
            "a push of seven times the counter, each pass",
            "While k is at most n:\n    Push k * 7 to xs.\n    Set k to k + 1.\n",
        ),
        (
            "a store at the counter of twice the counter, each pass, into a list filled before",
            "While k is at most n:\n    Push 0 to xs.\n    Set k to k + 1.\nSet k to 1.\n\
             While k is at most n:\n    Set item k of xs to k * 2.\n    Set k to k + 1.\n",
        ),
        (
            "a push one past the counter, after the counter steps",
            "While k is at most n:\n    Set k to k + 1.\n    Push k + 1 to xs.\n",
        ),
    ] {
        let src = filled_by(body);
        let rust = compile_to_rust(&src).expect("Rust for the program");
        assert!(
            !rust.contains("IntVec"),
            "{shape}: the loop runs no more passes than a list holds items, so the counter stays near 2^60:\n{rust}"
        );
        exact_on_rust_with(&src, "6");
    }
}

#[test]
fn a_value_of_a_counter_no_list_bounds_may_promote() {
    for (shape, body) in [
        (
            "nine times the counter, past the word at 2^60",
            "While k is at most n:\n    Push k * 9 to xs.\n    Set k to k + 1.\n",
        ),
        (
            "a push some passes skip",
            "While k is at most n:\n    If k % 2 equals 0:\n        Push k * 4611686018427387904 to xs.\n    Set k to k + 1.\n",
        ),
        (
            "a pop that shortens the list each pass",
            "Push 0 to xs.\nWhile k is at most n:\n    Push k * 4611686018427387904 to xs.\n    Pop from xs.\n    Set k to k + 1.\n",
        ),
        (
            "a counter the body sets",
            "While k is at most n:\n    Push k * 4 to xs.\n    Set k to k * 2.\n",
        ),
        (
            "the counter after the loop",
            "While k is at most n:\n    Push 0 to xs.\n    Set k to k + 1.\nPush k * 4611686018427387904 to xs.\n",
        ),
        (
            "a list a builtin is handed in the loop",
            "While k is at most n:\n    Push k * 4611686018427387904 to xs.\n    Show length of xs.\n    Set k to k + 1.\n",
        ),
    ] {
        let src = filled_by(body);
        let rust = compile_to_rust(&src).expect("Rust for the program");
        if !shape.contains("pop") && !shape.contains("builtin") {
            assert!(rust.contains("IntVec"), "{shape}: nothing bounds the counter, so the list may promote:\n{rust}");
        }
        exact_or_loud_on_rust(&src, &["6"], &reference_with(&src, "6"));
    }
}

#[test]
fn an_increment_no_single_counting_loop_bounds_may_promote() {
    let header = "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\n";
    let input = "Let arguments be args().\nLet n be parseInt(item 2 of arguments).\n";
    for (shape, src) in [
        (
            "an increment of a list a caller hands in",
            format!(
                "{header}## To tally (c: Seq of Int) and (n: Int) -> Seq of Int:\n    Let mutable d be c.\n    \
                 Let mutable a be 0.\n    While a is less than n:\n        Set item 1 of d to (item 1 of d) + 1.\n        \
                 Set a to a + 1.\n    Return d.\n\n## Main\n{input}Let mutable c be a new Seq of Int.\nPush 0 to c.\n\
                 Set c to tally(c, n).\nSet c to tally(c, n).\nShow c.\n"
            ),
        ),
    ] {
        let rust = compile_to_rust(&src).expect("Rust for the program");
        assert!(rust.contains("IntVec"), "{shape}: no single counting loop bounds it, so it may promote:\n{rust}");
        exact_or_loud_on_rust(&src, &["3"], &reference_with(&src, "3"));
    }
}

#[test]
#[ignore = "B30: the shared scalar decision is held off on AOT-Rust (exact_scalars::DECIDES_SCALARS) until a loop that writes a list runs its exact locals on machine words first"]
fn an_increment_a_register_carries_may_promote() {
    let head = "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\n## Main\n\
                Let arguments be args().\nLet n be parseInt(item 2 of arguments).\nLet mutable c be a new Seq of Int.\n\
                Push 0 to c.\nLet mutable a be 0.\nWhile a is less than n:\n";
    for (shape, body) in [
        (
            "an increment in a nested loop",
            "    Let mutable b be 0.\n    While b is less than n:\n        Set item 1 of c to (item 1 of c) + 1.\n        \
             Set b to b + 1.\n    Set a to a + 1.\nShow c.\n",
        ),
        (
            "two increments a pass",
            "    Set item 1 of c to (item 1 of c) + 1.\n    Set item 1 of c to (item 1 of c) + 1.\n    Set a to a + 1.\n\
             Show c.\n",
        ),
    ] {
        let src = format!("{head}{body}");
        let rust = compile_to_rust(&src).expect("Rust for the program");
        assert!(rust.contains("IntVec"), "{shape}: no single counting loop bounds it, so it may promote:\n{rust}");
        exact_or_loud_on_rust(&src, &["3"], &reference_with(&src, "3"));
    }
}

#[test]
fn a_counted_loop_reading_a_list_that_may_promote_runs_on_its_words_with_its_bounds_hints() {
    for (shape, before) in [
        ("a list that holds words when the loop begins", ""),
        ("a list past the word before the loop begins", "Set item 2 of d to (item 2 of d) + 9223372036854775807.\n"),
    ] {
        let src = format!(
            "{ARGS}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\n\
             Let mutable d be a new Seq of Int.\nLet mutable k be 1.\nWhile k is at most n:\n    Push k * 1000000000000000000 to d.\n    \
             Set k to k + 1.\n{before}Let mutable s be 0.\nLet mutable i be 1.\nWhile i is at most n:\n    \
             Set s to (s + item i of d) % 1000003.\n    Set i to i + 1.\nShow s.\nShow d.\n"
        );
        let rust = compile_to_rust(&src).expect("Rust for the program");
        assert!(rust.contains("IntVec"), "{shape}: `d` may pass the word:\n{rust}");
        assert!(
            rust.contains(".words()") && rust.contains("__d_bhl"),
            "{shape}: the reading loop holds a view of the list's words, and its bounds hint on that view's length:\n{rust}"
        );
        exact_on_rust_with(&src, "12");
    }
}

#[test]
fn a_counter_a_loop_runs_while_it_is_within_a_lists_length_keeps_its_words() {
    let walk = |grow: &str| {
        format!(
            "{ARGS}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\n\
             Let mutable q be a new Seq of Int.\nPush 1 to q.\nLet mutable front be 1.\nLet mutable seen be 0.\n\
             While front is at most length of q:\n    Set seen to seen + (front * 7).\n{grow}    Set front to front + 1.\n\
             Show seen.\nShow length of q.\n"
        )
    };
    let growing = walk("    If length of q is less than n:\n        Push front to q.\n");
    let rust = compile_to_rust(&growing).expect("Rust for the program");
    assert!(
        !rust.contains("let mut front: logicaffeine_data::LogosInt"),
        "the loop runs while the counter is within the list's length, which a list's 2^60 items bound:\n{rust}"
    );
    exact_on_rust_with(&growing, "9");
    exact_on_rust_with(&walk("    Push front to q.\n    Pop from q.\n"), "9");
}
