//! ★ LOCK — a `Seq of Int` holds tagged cells on AST C exactly where every reader decodes them. ★
//!
//! AST C holds an element that may outgrow a machine word as a tagged cell, in the same `Seq_i64` a word list uses,
//! so a cell is right only where the code reading it decodes it. The decision is taken for each flow class of
//! sequences: every local, parameter and return a list's cells move between by a binding, a `copy of`, a `Swap`, an
//! argument or a return. A class is tagged when some store into it may be exact and nothing outside the decoding
//! readers (an item read, `length of`, `contains`, `Push`, `Set item`, `Repeat`, `Show`) reads or fills it.
//!
//! - a list that only those readers see is exact, wherever its cells travel: through a function 400 times, into an
//!   alias, from a literal argument, back from a return, across a `Swap`;
//! - a list a builtin reads keeps raw words: `toBase`'s byte buffer prints its digits;
//! - a list that reaches a raw reader or source (`Pop`, a slice, a closure, a builtin's result, a record field, another
//!   body's binding) holds raw words: C prints the tree-walker's bytes, or stops loudly at the value past the word after printing a
//!   prefix of them, and never prints anything else.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test c_exact_seq_flow_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::{compile_to_c, tw_lines};

/// The sentence a value past the word stops C with where it holds a machine word.
const PAST_THE_WORD: &str = "integer value out of range for this int64 context";

/// What a program printed on AST C, and what it said if it failed.
struct Run {
    output: String,
    failure: Option<String>,
}

fn on_c(src: &str) -> Run {
    let c = compile_to_c(src).unwrap_or_else(|e| panic!("the C generator refused ({e:?}):\n{src}"));
    let dir = tempfile::tempdir().expect("temp dir");
    let (c_path, bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&c_path, &c).expect("write the C");
    let built = common::build_c_release(&c_path, &bin).expect("run the C compiler");
    assert!(built.status.success(), "the C does not compile:\n{}\n{src}", String::from_utf8_lossy(&built.stderr));
    let run = std::process::Command::new(&bin).output().expect("run the C program");
    Run {
        output: String::from_utf8_lossy(&run.stdout).into_owned(),
        failure: (!run.status.success()).then(|| String::from_utf8_lossy(&run.stderr).into_owned()),
    }
}

/// What the tree-walker prints for `src`, which must run clean.
fn reference(src: &str) -> String {
    let tw = tw_lines(src);
    assert_eq!(tw.error, None, "the tree-walker refused:\n{src}");
    tw.lines.iter().map(|line| format!("{line}\n")).collect()
}

/// AST C prints exactly what the tree-walker prints.
fn exact_on_c(src: &str) {
    let expected = reference(src);
    let run = on_c(src);
    assert_eq!(run.failure, None, "C failed:\n{src}\n{}", run.output);
    assert_eq!(run.output, expected, "C's answer:\n{src}");
}

/// AST C prints what the tree-walker prints, or stops loudly at a value past the word after printing a prefix of it.
fn exact_or_loud_on_c(src: &str) {
    let expected = reference(src);
    let run = on_c(src);
    match run.failure {
        None => assert_eq!(run.output, expected, "C's answer:\n{src}"),
        Some(why) => {
            assert!(why.contains(PAST_THE_WORD), "C failed some other way ({why}):\n{src}");
            assert!(expected.starts_with(&run.output), "C printed other bytes before it stopped:\n{}\n{src}", run.output);
        }
    }
}

#[test]
fn a_list_carried_through_a_function_four_hundred_times_stays_exact() {
    exact_on_c(
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

#[test]
fn an_alias_and_its_source_read_their_cells_alike() {
    exact_on_c(
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

/// The function's result is a word (a scalar slot keeps its word in this increment), so the element past the word stops
/// it loudly; the list itself is built promoted from its literal and read exactly where a list is read.
#[test]
fn a_literal_handed_straight_to_a_function_is_read_exactly_there() {
    exact_or_loud_on_c(
        "## To first (xs: Seq of Int) -> Int:
    Return item 1 of xs.

## Main
Show first([2 ** 70, 1]).
Show first([3, 4]).
",
    );
}

#[test]
fn a_list_returned_with_a_value_past_the_word_reads_back_exactly() {
    exact_on_c(
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
fn a_swap_carries_the_cells_both_ways() {
    exact_on_c(
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

/// `contains` and `Set item` are exact on a promoted list; `Repeat` binds each element to a word (a scalar slot keeps
/// its word in this increment), so the sum of elements past the word stops loudly.
#[test]
fn contains_repeat_and_set_item_read_and_write_the_cells() {
    exact_or_loud_on_c(
        "## Main
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
",
    );
}

#[test]
fn a_byte_buffer_a_builtin_reads_keeps_its_raw_words() {
    exact_on_c("## Main\nShow toBase(255, 16).\nShow toBase(1000000007, 36).\nShow toBinary(10).\n");
}

#[test]
fn a_list_a_pop_reads_keeps_raw_words() {
    exact_or_loud_on_c(
        "## Main
Let mutable xs be [1].
Push 2 ** 70 to xs.
Pop from xs into v.
Show v.
",
    );
}

#[test]
fn a_list_a_slice_reads_keeps_raw_words() {
    exact_or_loud_on_c(
        "## Main
Let mutable xs be [1].
Push 2 ** 70 to xs.
Show items 1 through 2 of xs.
",
    );
}

#[test]
fn a_list_a_closure_reads_keeps_raw_words() {
    exact_or_loud_on_c(
        "## Main
Let mutable xs be [1].
Push 2 ** 70 to xs.
Let pick be (n: Int) -> item n of xs.
Show pick(2).
",
    );
}

#[test]
fn a_list_a_builtin_made_keeps_raw_words() {
    exact_or_loud_on_c(
        "## Main
Let mutable bs be text_bytes(\"AB\").
Push 2 ** 70 to bs.
Show item 3 of bs.
Show item 1 of bs.
",
    );
}

#[test]
fn a_list_in_a_record_field_keeps_raw_words() {
    exact_or_loud_on_c(
        "## A Bag has:
    An items: Seq of Int.

## Main
Let mutable b be a new Bag with items [1, 2].
Push 2 ** 70 to b's items.
Show item 3 of b's items.
Show b's items.
",
    );
}

#[test]
fn a_list_shared_by_a_raw_reader_and_an_exact_store_never_prints_other_bytes() {
    exact_or_loud_on_c(
        "## To head (xs: Seq of Int) -> Int:
    Return item 1 of xs.

## Main
Let mutable xs be [1].
Push 2 ** 70 to xs.
Show head(xs).
Show head(text_bytes(\"A\")).
Pop from xs into v.
Show v.
",
    );
}

#[test]
fn a_record_fields_list_read_into_a_local_keeps_raw_words() {
    exact_or_loud_on_c(
        "## A Bag has:
    An items: Seq of Int.

## Main
Let mutable b be a new Bag with items [1, 2].
Let mutable xs be b's items.
Push 2 ** 70 to xs.
Show item 1 of xs.
Show xs.
",
    );
}

#[test]
fn a_list_another_body_pushes_into_keeps_raw_words() {
    exact_or_loud_on_c(
        "## To grow:
    Push 2 ** 70 to xs.

## Main
Let mutable xs be [1].
Call grow.
Show item 1 of xs.
Show xs.
",
    );
}

#[test]
fn a_literal_past_the_word_bound_to_a_list_a_slice_reads_stops_loudly() {
    exact_or_loud_on_c(
        "## Main
Let mutable xs be [2 ** 70, 1].
Show items 1 through 1 of xs.
",
    );
}

#[test]
fn a_list_read_into_a_word_local_in_a_loop_builds_and_counts() {
    exact_on_c(
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
fn a_list_promotes_at_its_first_value_past_the_word_and_stays_exact() {
    exact_on_c(
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
fn a_promoted_element_read_where_a_word_holds_it_stops_loudly() {
    exact_or_loud_on_c(
        "## Main
Let mutable xs be [1].
Push 2 ** 70 to xs.
Let w be abs(item 2 of xs).
Show w.
",
    );
}

#[test]
fn contains_and_set_item_are_exact_on_a_promoted_list() {
    exact_on_c(
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

/// A list filled from a scalar only arithmetic over words nothing bounds makes exact (knapsack's `best`, the larger of
/// an item and an item plus a value) keeps its words: the scalar is stored narrowing, loudly past the word, as a checked
/// machine word is, so the hot loop reads and writes words with no `exact` bit to ask.
#[test]
fn a_list_a_word_bounded_loop_fills_from_a_speculative_scalar_keeps_its_words() {
    let src = "## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable vals be [3, 1, 4, 1, 5].
Let mutable prev be [0, 0, 0, 0, 0, 0].
Let mutable i be 0.
While i is less than n:
    Let mutable curr be a new Seq of Int.
    Let vi be item ((i % 5) + 1) of vals.
    Let mutable w be 1.
    While w is at most 6:
        Let mutable best be item w of prev.
        If w is greater than 1:
            Let take be item (w - 1) of prev + vi.
            If take is greater than best:
                Set best to take.
        Push best to curr.
        Set w to w + 1.
    Set prev to curr.
    Set i to i + 1.
Show prev.
";
    let c = compile_to_c(src).unwrap_or_else(|e| panic!("the C generator refused ({e:?}):\n{src}"));
    for exact_access in ["seq_i64_get_lint", "seq_i64_push_lint", "seq_i64_get_word", "seq_i64_push_word"] {
        assert!(!c.contains(&format!("{exact_access}(&prev")) && !c.contains(&format!("{exact_access}(&curr")), "the lists keep their words, no `{exact_access}`:\n{c}");
    }
    let dir = tempfile::tempdir().expect("temp dir");
    let (c_path, bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&c_path, &c).expect("write the C");
    let built = common::build_c_release(&c_path, &bin).expect("run the C compiler");
    assert!(built.status.success(), "the C does not compile:\n{}", String::from_utf8_lossy(&built.stderr));
    let run = std::process::Command::new(&bin).arg("20").output().expect("run the C program");
    let tw = logicaffeine_compile::compile::tw_lines_with_args(src, &["main".to_string(), "20".to_string()]);
    let expected: String = tw.lines.iter().map(|line| format!("{line}\n")).collect();
    assert_eq!(String::from_utf8_lossy(&run.stdout), expected, "C's answer:\n{src}");
}
