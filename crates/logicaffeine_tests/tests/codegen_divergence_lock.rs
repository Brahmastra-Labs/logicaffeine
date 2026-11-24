//! ★ LOCK — a program means on the compiled tiers what it means on the tree-walker. ★
//!
//! Each row is a program the tree-walker (the reference) runs. The VM, the AOT-Rust tier built debug and built
//! release, and AST C each print the same bytes, or fail with the same sentence after printing the same bytes,
//! within a time limit. The rows are shapes the Rust and C emitters once lowered to something else: a shift by a
//! count past the word, a capacity from a negative count, a loop that ends at the largest Int, a missing key in a
//! map of reals or of money, a pipe filled past its capacity with no receiver, a parameter named like a Rust
//! keyword, a CRDT record's Text field, a last-write-wins field name another record also uses, a closure bound
//! under its function type, a counter set from its own value right after its loop, a closure a function returns used
//! twice, a bit intrinsic called only inside a loop, a variable named like the length a loop hoists, a negative
//! capacity, a tail-recursive function's parameter named like a Rust keyword, a binding named like a Rust keyword that
//! fills a variant's field, an item stored at an index read from another list or at one named like a Rust keyword, a
//! program's own function named `show` beside the `Show` statement, a record's fields read into collections while
//! the record is used again, a byte-compare loop whose counter starts past zero or is read after the loop, a name bound
//! again to a value of another type, a copy of a list parameter the function changes, a record named like a list
//! type, a generic variant's field bound where its type is not written, a strided fill up to a literal bound, the
//! word of a quotient, the absolute value of literals' minimum, a name rebound between text and a number, a
//! `Return` in `Main`, an empty list or `nothing` whose element nothing constrains, a loop whose bound reads its
//! own counter, a copy of the items of a list the emitter writes as its closed form, a division by a parameter in a
//! loop, a filled list set again though it is not bound `mutable`, the CRDT statements on records and fields that
//! are not `Shared`, a `Pair` or `Triple` type, a record named `Box` beside a recursive variant, and a value compared
//! with `nothing`.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test codegen_divergence_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::time::Duration;

use common::RustProfile;
use logicaffeine_compile::compile::{compile_program_full, compile_to_c, tw_lines, vm_outcome};

/// How long a compiled program may run.
const LIMIT: Duration = Duration::from_secs(60);

/// One tier's run: what it printed, and, if it failed, what it said.
struct Run {
    tier: &'static str,
    output: String,
    failure: Option<String>,
}

/// Shown lines (or one sentence) as a process writes them.
fn written<S: AsRef<str>>(lines: &[S]) -> String {
    lines.iter().map(|l| format!("{}\n", l.as_ref())).collect()
}

/// A compiled program's run within [`LIMIT`].
fn ran(tier: &'static str, binary: &std::path::Path) -> Run {
    let run = common::run_within(binary, &[], LIMIT);
    let failure = if run.timed_out {
        Some(format!("still running after {LIMIT:?}"))
    } else {
        (!run.success).then_some(run.stderr)
    };
    Run {
        tier,
        output: run.stdout,
        failure,
    }
}

fn aot_rust(tier: &'static str, src: &str, profile: RustProfile) -> Run {
    let out = compile_program_full(src).unwrap_or_else(|e| panic!("no Rust for:\n{src}\n{e:?}"));
    match common::build_generated(&out.rust_code, &out.dependencies, profile) {
        Ok(built) => ran(tier, &built.binary),
        Err(why) => Run {
            tier,
            output: String::new(),
            failure: Some(format!("does not build:\n{why}\n{}", out.rust_code)),
        },
    }
}

fn ast_c(src: &str) -> Run {
    let tier = "AST C";
    let c = compile_to_c(src).unwrap_or_else(|e| panic!("no C for:\n{src}\n{e:?}"));
    let dir = tempfile::tempdir().expect("temp dir");
    let (c_path, bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&c_path, &c).expect("write the C");
    let built = common::build_c_release(&c_path, &bin).expect("run the C compiler");
    if !built.status.success() {
        return Run {
            tier,
            output: String::new(),
            failure: Some(format!(
                "does not build:\n{}",
                String::from_utf8_lossy(&built.stderr)
            )),
        };
    }
    ran(tier, &bin)
}

/// `src` on every tier but the reference.
fn every_tier(src: &str) -> Vec<Run> {
    let vm = vm_outcome(src);
    vec![
        Run {
            tier: "VM",
            output: vm.output,
            failure: vm.error.map(|why| written(&[why])),
        },
        aot_rust("AOT Rust (debug)", src, RustProfile::Debug),
        aot_rust("AOT Rust (release)", src, RustProfile::Release),
        ast_c(src),
    ]
}

/// `src` prints on every tier what it prints on the tree-walker.
fn means_the_same_everywhere(src: &str) {
    let tw = tw_lines(src);
    assert_eq!(tw.error, None, "the tree-walker refused:\n{src}");
    let reference = written(&tw.lines);
    let mut report = String::new();
    for run in every_tier(src) {
        if run.failure.is_some() || run.output != reference {
            report.push_str(&format!(
                "\n== {}\nprinted:\n{}failed: {:?}\n",
                run.tier, run.output, run.failure
            ));
        }
    }
    assert!(
        report.is_empty(),
        "the tree-walker prints:\n{reference}for:\n{src}{report}"
    );
}

/// `src` prints on every tier what the tree-walker prints, but AOT Rust, whose `Int` is a machine word until B9/B10's
/// exact AOT Rust lands, may stop instead where an `Int` passes the word: having printed what the tree-walker printed
/// up to there, with the sentence that names the exact value (`exact`). It never prints a wrapped number.
fn means_the_same_or_rust_stops_at_the_word(src: &str, exact: &str) {
    let tw = tw_lines(src);
    assert_eq!(tw.error, None, "the tree-walker refused:\n{src}");
    let reference = written(&tw.lines);
    let sentence = format!("Integer overflow: {exact} does not fit a 64-bit Int");
    let mut report = String::new();
    for run in every_tier(src) {
        let same = run.failure.is_none() && run.output == reference;
        let stopped = run.tier.starts_with("AOT Rust")
            && reference.starts_with(&run.output)
            && run.failure.as_deref().is_some_and(|why| why.trim_end() == sentence);
        if !same && !stopped {
            report.push_str(&format!(
                "\n== {}\nprinted:\n{}failed: {:?}\n",
                run.tier, run.output, run.failure
            ));
        }
    }
    assert!(
        report.is_empty(),
        "the tree-walker prints:\n{reference}for:\n{src}(AOT Rust may stop with: {sentence}){report}"
    );
}

/// `src` fails on every tier where the tree-walker fails, with its sentence, after printing what it printed.
fn fails_the_same_everywhere(src: &str) {
    let tw = tw_lines(src);
    let why = tw
        .error
        .as_deref()
        .unwrap_or_else(|| panic!("the tree-walker ran it:\n{src}"));
    let (reference, sentence) = (written(&tw.lines), written(&[why]));
    let mut report = String::new();
    for run in every_tier(src) {
        if run.failure.as_deref() != Some(sentence.as_str()) || run.output != reference {
            report.push_str(&format!(
                "\n== {}\nprinted:\n{}failed: {:?}\n",
                run.tier, run.output, run.failure
            ));
        }
    }
    assert!(
        report.is_empty(),
        "the tree-walker prints:\n{reference}then fails: {sentence}for:\n{src}{report}"
    );
}

#[test]
fn a_shift_count_is_taken_modulo_the_word() {
    means_the_same_everywhere(
        "## Main
Let counts be [60, 63, 64, 65, 127, 0 - 1, 0 - 64].
Repeat for c in counts:
    Show 1 shifted left by c.
    Show (0 - 8) shifted left by c.
    Show 1024 shifted right by c.
    Show (0 - 1024) shifted right by c.
",
    );
}

#[test]
fn a_negative_count_sizes_an_empty_list_and_an_empty_map() {
    means_the_same_everywhere(
        "## Main
Let words be split(\"a b c\", \" \").
Let n be 1 - length of words - length of words.
Let mutable xs be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i * 2 to xs.
    Set i to i + 1.
Show length of xs.
Let mutable m be a new Map of Int to Int.
Set i to 0.
While i is less than n:
    Set m at i to i.
    Set i to i + 1.
Show length of m.
",
    );
}

#[test]
fn a_counted_loop_may_end_at_the_largest_int() {
    means_the_same_everywhere(
        "## Main
Let words be split(\"a b c\", \" \").
Let top be 9223372036854775807 - 3 + length of words.
Let mutable count be 0.
Let mutable i be top - 2.
While i is at most top:
    Set count to count + 1.
    If i equals top:
        Show count.
        Return.
    Set i to i + 1.
Show count.
",
    );
}

#[test]
fn a_counter_set_from_its_own_value_after_its_loop_reads_the_value_the_loop_left() {
    means_the_same_everywhere(
        "## Main
Let mutable total be 0.
Let mutable i be 0.
While i is less than 5:
    Set total to total + i.
    Set i to i + 1.
Set i to i * 2.
Show total.
Show i.
",
    );
}

#[test]
fn a_closure_a_function_returns_may_be_used_twice() {
    means_the_same_everywhere(
        "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:
    Return f(x).

## To adder (n: Int) -> fn(Int) -> Int:
    Return (x: Int) -> x + n.

## Main
Let add5 be adder(5).
Show apply(add5, 10).
Let mutable total be 0.
Repeat for x in [1, 2, 3]:
    Set total to total + apply(add5, x).
Show total.
",
    );
}

#[test]
fn a_bit_intrinsic_called_only_inside_a_loop_is_defined() {
    means_the_same_everywhere(
        "## Main
Repeat for c in [1, 7, 31, 62]:
    Let up be 1 shifted left by c.
    Show count_ones(up - 1).
",
    );
}

#[test]
fn a_missing_key_in_a_map_of_reals_fails_as_on_the_tree_walker() {
    fails_the_same_everywhere(
        "## Main
Let mutable prices be a new Map of Text to Real.
Set prices at \"tea\" to 2.5.
Show item \"tea\" of prices.
Show item \"coffee\" of prices.
",
    );
}

#[test]
fn a_missing_key_in_a_map_of_money_fails_as_on_the_tree_walker() {
    fails_the_same_everywhere(
        "## Main
Let mutable m be a new Map of Text to Money.
Set item \"x\" of m to money(3, \"USD\").
Show item \"x\" of m.
Show item \"y\" of m.
",
    );
}

/// A pipe declared without a capacity holds the scheduler's default (32); the main task filling it with no receiver
/// blocks, and with every task blocked the run fails as the tree-walker's does.
#[test]
fn a_pipe_filled_past_its_capacity_with_no_receiver_deadlocks_as_on_the_tree_walker() {
    fails_the_same_everywhere(
        "## Main
Let ch be a Pipe of Int.
Let mutable i be 0.
While i is less than 40:
    Send i into ch.
    Set i to i + 1.
Let mutable total be 0.
Set i to 0.
While i is less than 40:
    Receive x from ch.
    Set total to total + x.
    Set i to i + 1.
Show total.
",
    );
}

#[test]
fn a_memoized_function_may_name_its_parameter_like_a_rust_keyword() {
    means_the_same_everywhere(
        "## To paths (loop: Int) -> Int:
    If loop is less than 2:
        Return 1.
    Return paths(loop - 1) + paths(loop - 2).

## Main
Show paths(30).
",
    );
}

#[test]
fn a_shared_records_text_field_merges_as_on_the_tree_walker() {
    means_the_same_everywhere(
        "## Definition
A Board is Shared and has:
    a points, which is ConvergentCount.
    a games, which is Int.
    a title, which is Text.

## Main
Let mutable local be a new Board.
Let mutable remote be a new Board.
Increase local's points by 3.
Increase remote's points by 4.
Set local's games to 2.
Set remote's games to 5.
Set local's title to \"home\".
Set remote's title to \"away\".
Merge remote into local.
Merge remote into local.
Show local's points.
Show local's games.
Show local's title.
",
    );
}

#[test]
fn a_last_write_wins_field_belongs_to_its_own_record() {
    means_the_same_everywhere(
        "## Definition
A Profile is Shared and has:
    a name, which is LastWriteWins of Text.
A Pet has:
    a name, which is Text.

## Main
Let mutable p be a new Pet.
Set p's name to \"Rex\".
Show p's name.
",
    );
}

#[test]
fn a_closure_bound_under_its_function_type_is_called_as_on_the_tree_walker() {
    means_the_same_everywhere(
        "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:
    Return f(x).

## Main
Let double: fn(Int) -> Int be (x: Int) -> x * 2.
Show apply(double, 21).
",
    );
}

#[test]
fn a_variable_named_like_a_hoisted_length_keeps_its_value() {
    means_the_same_everywhere(
        "## To total (xs: Seq of Int) and (xs_len: Int) -> Int:
    Let mutable sum be 0.
    Let mutable i be 1.
    While i is at most length of xs:
        Set sum to sum + xs_len.
        Set i to i * 2.
    Return sum.

## Main
Let xs be [1, 2, 3].
Show total(xs, 100).
",
    );
}

#[test]
fn a_negative_capacity_reserves_nothing() {
    means_the_same_everywhere(
        "## Main
Let words be split(\"a b c\", \" \").
Let n be 1 - length of words - length of words.
Let mutable xs be a new Seq of Int with capacity n.
Push 7 to xs.
Show length of xs.
Let mutable m be a new Map of Int to Int with capacity n.
Set m at 1 to 2.
Show length of m.
Let mutable s be a new Set of Int with capacity n.
Add 3 to s.
Show length of s.
Let mutable t be \"\" with capacity n.
Set t to t + \"ok\".
Show t.
",
    );
}

#[test]
fn a_tail_recursive_function_may_name_its_parameter_like_a_rust_keyword() {
    means_the_same_everywhere(
        "## To total (loop: Int) and (acc: Int) -> Int:
    If loop is at most 0:
        Return acc.
    Return total(loop - 1, acc + loop).

## Main
Show total(10, 0).
",
    );
}

#[test]
fn a_binding_named_like_a_rust_keyword_may_fill_a_variants_field() {
    means_the_same_everywhere(
        "## An Outcome is either:
    A Fine (n: Int).
    A Refused (reason: Text).

## To check (n: Int) -> Outcome:
    Let unsafe be \"n is {n}\".
    If n is greater than 2:
        Return a new Refused with reason unsafe.
    Return a new Fine with n n.

## Main
Inspect check(5):
    When Fine (n):
        Show n.
    When Refused (reason):
        Show reason.
",
    );
}

#[test]
fn an_item_may_be_stored_at_an_index_read_from_another_list() {
    means_the_same_everywhere(
        "## To place (cursor: Seq of Int) -> Seq of Int:
    Let mutable order be [0, 0, 0].
    Let mutable f be 1.
    While f is at most 3:
        Set item (item f of cursor) of order to f * 10.
        Set f to f + 1.
    Return order.

## Main
Show place([3, 1, 2]).
",
    );
}

#[test]
fn a_store_index_may_be_named_like_a_rust_keyword() {
    means_the_same_everywhere(
        "## Main
Let mutable xs be [1, 2, 3].
Let loop be 2.
Set item loop of xs to 9.
Set item (loop + 1) of xs to 7.
Show xs.
",
    );
}

#[test]
fn a_programs_own_function_named_show_leaves_the_show_statement_alone() {
    means_the_same_everywhere(
        "## To show (items: Seq of Int):
    Repeat for x in items:
        Show x + 1.

## Main
Call show with [1, 2].
",
    );
}

#[test]
fn a_records_fields_may_be_read_into_collections_while_the_record_is_used_again() {
    means_the_same_everywhere(
        "## A Inner has:
    a v, which is Int.

## A Outer has:
    a head, which is Inner.
    a name, which is Text.

## To tagOf (o: Outer) -> Text:
    Return o's name.

## To collect (outers: Seq of Outer) -> Seq of Text:
    Let mutable heads be a new Seq of Inner.
    Let mutable seen be a new Set of Text.
    Let mutable names be a new Seq of Text.
    Repeat for o in outers:
        Push o's head to heads.
        Push tagOf(o) to names.
        If not (seen contains o's name):
            Add o's name to seen.
            Push o's name to names.
    Return names.

## Main
Let a be a new Outer with head (a new Inner with v 1) and name \"a\".
Let b be a new Outer with head (a new Inner with v 2) and name \"b\".
Show collect([a, b]).
",
    );
}

#[test]
fn a_window_compare_counts_from_where_its_counter_starts() {
    means_the_same_everywhere(
        "## To check (text: Text) and (needle: Text) -> Int:
    Let mutable matched be 1.
    Let mutable j be 2.
    While j is less than 5:
        If item (1 + j) of text is not item (1 + j) of needle:
            Set matched to 0.
            Set j to 5.
        Set j to j + 1.
    Return matched.

## Main
Show check(\"abXYZQ\", \"QQXYZQ\").
Show check(\"abXYZQ\", \"abXYWQ\").
",
    );
}

#[test]
fn a_window_compares_counter_is_what_the_loop_left_it() {
    means_the_same_everywhere(
        "## To check (text: Text) and (needle: Text) -> Int:
    Let mutable matched be 1.
    Let mutable j be 0.
    While j is less than 5:
        If item (1 + j) of text is not item (1 + j) of needle:
            Set matched to 0.
            Set j to 5.
        Set j to j + 1.
    Return matched * 10 + j.

## Main
Show check(\"abcdeX\", \"abcdeY\").
Show check(\"abcdeX\", \"abQdeY\").
",
    );
}

#[test]
fn a_name_bound_again_takes_each_values_own_type() {
    means_the_same_everywhere(
        "## Main
Let x be 1.5.
Let x be x * 2.0.
Let x be \"{x}!\".
Show x.
Let y be \"seven\".
Show y.
Let y be 7.
Show y + 1.
",
    );
}

#[test]
fn a_copy_of_a_list_parameter_the_function_changes_is_a_list_of_its_own() {
    means_the_same_everywhere(
        "## To drainInto (src: Seq of Int) -> Int:
    Let dst be src.
    Pop from dst into at.
    Return at.

## To changed (xs: Seq of Int) (value: Int) -> Int:
    Let changedValues be xs.
    Set item 1 of changedValues to value.
    Return item 1 of changedValues + item 1 of xs.

## Main
Let input be [11, 13, 17].
Show drainInto(input).
Show changed(input, 5).
Show input.
",
    );
}

#[test]
fn a_record_named_like_a_list_type_is_the_record() {
    means_the_same_everywhere(
        "## A Vec has:
    x: Int.
    y: Int.
## Main
    Let a be a new Vec with x 3 and y 10.
    Let b be a.
    Show a's x.
    Show b's y.
    Show a's x + b's y.
",
    );
}

#[test]
fn a_record_named_like_a_list_type_built_without_fields_is_the_record() {
    means_the_same_everywhere(
        "## A Vec has:
    x: Int.

## To bump (v: Vec) -> Int:
    Return v's x + 1.

## Main
Let mutable a be a new Vec.
Set a's x to 41.
Show bump(a).
Let b be a.
Show b's x.
",
    );
}

#[test]
fn a_generic_variants_field_is_bound_where_its_type_is_not_written() {
    means_the_same_everywhere(
        "## A Maybe of [T] is either:
    A Nothing.
    A Just with value T.

## Main
Let m be a new Just with value \"s\".
Inspect m:
    When Nothing: Show 0.
    When Just (v):
        Let w be v.
        Show w.
",
    );
}

#[test]
fn a_strided_fill_may_run_up_to_a_literal_bound() {
    means_the_same_everywhere(
        "## Main
Let mutable flags be a new Seq of Bool.
Let mutable i be 0.
While i is less than 100:
    Push false to flags.
    Set i to i + 1.
Set i to 2.
While i is at most 100:
    Set item i of flags to true.
    Set i to i + 3.
Let mutable count be 0.
Set i to 1.
While i is at most 100:
    If item i of flags equals true:
        Set count to count + 1.
    Set i to i + 1.
Show count.
",
    );
}

#[test]
fn the_word_of_a_quotient_divides_by_a_promoted_divisor() {
    means_the_same_everywhere(
        "## To decompress (coeffs: Seq of Word16) and (d: Int) -> Seq of Word16:
    Let mutable pd be 1.
    Repeat for i from 1 to d:
        Set pd to pd * 2.
    Let half be pd / 2.
    Let mutable out be a new Seq of Word16.
    Repeat for x in coeffs:
        Let v be intOfWord16(x).
        Push word16((v * 3329 + half) / pd) to out.
    Return out.

## Main
Repeat for w in decompress([word16(1), word16(2), word16(100)], 4):
    Show intOfWord16(w).
",
    );
}

#[test]
fn the_word_of_a_quotient_divides_the_exact_dividend() {
    means_the_same_everywhere(
        "## To lowWord (a: Int) and (b: Int) and (c: Int) -> Int:
    Return intOfWord16(word16((a * b) / c)).

## Main
Show lowWord(6917529027641081856, 4, 1125899906842624).
Show lowWord(1000, 3, 7).
",
    );
}

#[test]
fn the_absolute_value_of_literals_minimum_is_an_int() {
    means_the_same_everywhere(
        "## Main
Show abs(min(-42, 7)).
Show max(min(15, 10), 1).
Show abs(-42 * 2 + 1).
Show abs(min(-4.5, 2.0)).
",
    );
}

#[test]
fn a_text_name_bound_again_to_a_number_adds() {
    means_the_same_everywhere(
        "## To relabel (s: Text) and (n: Int) -> Int:
    Let y be s + \"!\".
    Show y.
    Show y.
    Let y be n * 2.
    Return y + 1.

## Main
Show relabel(\"seven\", 7).
",
    );
}

#[test]
fn a_number_name_bound_again_to_text_is_shown_as_often_as_it_is_read() {
    means_the_same_everywhere(
        "## To relabel (s: Text) and (n: Int) -> Text:
    Let y be n * 2.
    Show y.
    Let y be s + \"!\".
    Show y.
    Show y.
    Return y.

## Main
Show relabel(\"seven\", 7).
",
    );
}

#[test]
fn a_return_in_main_computes_its_value_and_ends_the_program() {
    means_the_same_everywhere(
        "## To loud (n: Int) -> Int:
    Show \"computing {n}\".
    Return n * 2.

## Main
Let x be 5.
Show x.
If x is greater than 3:
    Return loud(x).
Show 99.
",
    );
    means_the_same_everywhere(
        "## Main
Let i be 10.
While i is at least 1:
    Set i to i - 1.
Show i.
Return i.
",
    );
}

#[test]
fn an_empty_list_nothing_fills_has_a_length() {
    means_the_same_everywhere(
        "## Main
Let xs be a new Seq of Text.
Show length of xs.
Show xs contains \"a\".
Let e be [].
Show length of e.
",
    );
}

#[test]
fn nothing_no_value_types_is_shown_tested_and_inspected() {
    means_the_same_everywhere(
        "## Main
Let o be some 5.
Show o.
Let n be none.
Show n.
Let v be none.
If v is none:
    Show \"absent\".
Otherwise:
    Show \"present\".
Inspect v:
    When Some (k):
        Show \"some\".
    When None:
        Show \"none\".
",
    );
}

#[test]
fn a_loop_whose_bound_reads_its_counter_stops_where_the_bound_moves_to() {
    means_the_same_everywhere(
        "## To isPrime (n: Int) -> Bool:
    If n < 2:
        Return false.
    Let d be 2.
    While d * d <= n:
        If n % d = 0:
            Return false.
        Set d to d + 1.
    Return true.

## To isSumOfTwoPrimes (n: Int) -> Bool:
    Let p be 2.
    While p <= n - p:
        If isPrime(p) and isPrime(n - p):
            Return true.
        Set p to p + 1.
    Return false.

## To meet (n: Int) -> Int:
    Let mutable p be 0.
    While p <= n - p:
        Set p to p + 1.
    Return p.

## Main
Let mutable count be 0.
Repeat for n from 4 to 60:
    If n % 2 = 0 and isSumOfTwoPrimes(n):
        Set count to count + 1.
Show count.
Show meet(20).
Show meet(7).
",
    );
}

#[test]
fn a_copy_of_a_list_written_as_its_closed_form_copies_its_items() {
    means_the_same_everywhere(
        "## Main
Let mutable src be a new Seq of Int.
Let mutable j be 0.
While j is less than 6:
    Push j * 3 to src.
    Set j to j + 1.
Let mutable total be 0.
Let mutable iter be 0.
While iter is less than 4000:
    Let mutable scratch be a new Seq of Int.
    Set j to 1.
    While j is at most 6:
        Push item j of src to scratch.
        Set j to j + 1.
    Let mutable lo be 1.
    Let mutable hi be 6.
    While lo is less than hi:
        Let tmp be item lo of scratch.
        Set item lo of scratch to item hi of scratch.
        Set item hi of scratch to tmp.
        Set lo to lo + 1.
        Set hi to hi - 1.
    Set total to total + item 1 of scratch.
    Set iter to iter + 1.
Show total.
",
    );
    means_the_same_everywhere(
        "## Main
Let n be 3.
Let mutable base be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i to base.
    Set i to i + 1.
Let mutable collected be a new Seq of Seq of Int.
Let mutable iter be 0.
While iter is less than n:
    Let mutable work be a new Seq of Int.
    Set i to 1.
    While i is at most n:
        Push item i of base to work.
        Set i to i + 1.
    Push work to collected.
    Set iter to iter + 1.
Show length of collected.
",
    );
}

#[test]
fn a_parameter_divides_in_a_loop_and_a_filled_list_is_set_again() {
    means_the_same_everywhere(
        "## To spread (n: Int) -> Int:
    Let mutable hits be 0.
    Let mutable i be 0.
    While i is less than n:
        Let slot be (i * 31 + 7) % n.
        If slot is not i:
            Set hits to hits + slot.
        Set i to i + 1.
    Return hits.

## To marks (n: Int) -> Int:
    Let dist be a new Seq of Int.
    Let mutable i be 0.
    While i is less than n:
        Push 0 - 1 to dist.
        Set i to i + 1.
    Set item 1 of dist to 0.
    Set item n of dist to 5.
    Return item 1 of dist + item n of dist + length of dist.

## Main
Show spread(10).
Show spread(97).
Show marks(6).
",
    );
}

#[test]
fn merge_and_converge_fold_a_plain_record_field_by_field() {
    means_the_same_everywhere(
        "## Definition
A Vec has:
    x: Int.
    y: Int.
## Main
Let mutable a be a new Vec with x 3 and y 10.
Let mutable b be a new Vec with x 5 and y 2.
Merge b into a.
Show a's x.
Show a's y.
",
    );
    means_the_same_everywhere(
        "## Definition
A Vec has:
    x: Int.
    y: Int.
## Main
Let mutable a be a new Vec with x 3 and y 10.
Let mutable b be a new Vec with x 5 and y 2.
Converge b into a.
Show a's x.
Show a's y.
",
    );
    means_the_same_everywhere(
        "## Definition
A Tally has:
    n: Int.

## Main
Let mutable a be a new Tally with n 1.
Let c be a.
Let mutable b be a new Tally with n 5.
Merge b into a.
Show a's n.
Show c's n.
Converge b into c.
Show c's n.
",
    );
    means_the_same_everywhere(
        "## Definition
A Vec has:
    x: Int.
    y: Int.
## Main
Let mutable a be a new Vec with x 3 and y 10.
Let mutable b be a new Vec with x 5 and y 2.
Merge b into a.
Merge b into a.
Show a's x.
Show a's y.
",
    );
    means_the_same_everywhere(
        "## A Counter has:
    a: Int.
    b: Int.
## Main
    Let mutable x be a new Counter with a 5.
    Let mutable y be a new Counter with b 3.
    Merge y into x.
    Show x's a.
    Show x's b.
",
    );
}

#[test]
fn increase_decrease_and_append_act_on_plain_fields() {
    means_the_same_everywhere(
        "## A Counter has:
    n: Int.
## Main
    Let mutable a be a new Counter with n 1.
    Let mutable b be a new Counter with n 10.
    Increase a's n by 5.
    Decrease a's n by 2.
    Merge b into a.
    Show a's n.
",
    );
    means_the_same_everywhere(
        "## A Bag has:
    nums: Seq of Int.

## Main
    Let d be a new Bag.
    Append 5 to d's nums.
    Append 7 to d's nums.
    Show length of d's nums.
    Show item 2 of d's nums.
",
    );
}

#[test]
fn merge_folds_a_set_field_and_a_map_field_as_the_tree_walker_does() {
    means_the_same_everywhere(
        "## Definition
A Pool has:
    a tags: Set of Text.
    a counts: Map of Text to Int.

## Main
Let mutable a be a new Pool.
Let mutable b be a new Pool.
Add \"x\" to a's tags.
Add \"y\" to b's tags.
Set a's counts[\"k\"] to 1.
Set b's counts[\"k\"] to 2.
Set b's counts[\"j\"] to 7.
Merge b into a.
Show a's tags.
Show a's counts.
Converge b into a.
Show a's counts.
",
    );
    means_the_same_everywhere(
        "## A Bag has:
    An items: Seq of Int.

## Main
Let mutable a be a new Bag with items [1].
Push 2 to a's items.
Let mutable b be a new Bag with items [5].
Push 6 to b's items.
Merge b into a.
Push 7 to a's items.
Push 8 to b's items.
Show a's items.
Show b's items.
",
    );
}

#[test]
fn a_pair_or_triple_type_is_the_tuple_it_names() {
    means_the_same_everywhere(
        "## To at1 (t: Pair of Int and Bool) -> Int:
    Return item 1 of t.
## To at2 (t: Pair of Int and Bool) -> Bool:
    Return item 2 of t.
## To tri (t: Triple of Int and Bool and Int) -> Int:
    Return item 1 of t + item 3 of t.
## To mk () -> Pair of Int and Bool:
    Return (42, true).
## Main
    Let p be (7, true).
    Show at1(p).
    Show at2(p).
    Let q be (10, false, 5).
    Show tri(q).
    Let r be mk().
    Show item 1 of r.
    Show item 2 of r.
",
    );
    means_the_same_everywhere(
        "## To describe (p: Pair of Int and Text) -> Text:
    Show p.
    Return item 2 of p.

## Main
Let p be (3, \"x\").
Show describe(p).
",
    );
    means_the_same_everywhere(
        "## To total (p: Pair of Int and Int) -> Int:
    Show p.
    Return item 1 of p + item 2 of p.

## Main
Show total((5, 6)).
",
    );
}

#[test]
fn a_record_named_box_leaves_the_boxes_of_a_recursive_variant_alone() {
    means_the_same_everywhere(
        "## A Box has:
    a label: Text.

## A Tree is either:
    A Leaf.
    A Node with value Int and left Tree and right Tree.

## To total (t: Tree) -> Int:
    Inspect t:
        When Leaf: Return 0.
        When Node (v, l, r): Return v + total(l) + total(r).

## Main
Let b be a new Box with label \"parts\".
Let t be a new Node with value 3 and left (a new Node with value 4 and left (a new Leaf) and right (a new Leaf)) and right (a new Leaf).
Show b's label.
Show total(t).
",
    );
}

#[test]
fn a_value_equals_nothing_only_if_it_is_nothing() {
    means_the_same_everywhere(
        "## Main
Show false is equal to nothing.
Show nothing is equal to false.
Show false is not equal to nothing.
Show nothing is not equal to false.
",
    );
    means_the_same_everywhere(
        "## Main
Show 0 is equal to nothing.
Show nothing is equal to 0.
Show 0 is not equal to nothing.
Show nothing is not equal to 0.
Show nothing is equal to nothing.
Show nothing is not equal to nothing.
",
    );
    means_the_same_everywhere(
        "## Main
Let text be \"\".
Let items be a new Seq of Int.
Show text is equal to nothing.
Show nothing is equal to text.
Show items is equal to nothing.
Show nothing is equal to items.
",
    );
    means_the_same_everywhere(
        "## A P has:
    an x: Int.
    a tag: Text.

## Main
Let a be some 3.
Let b be some 3.
Let c be none.
Show a is equal to b.
Show a is equal to c.
Show c is equal to nothing.
Show a is equal to nothing.
Let p be a new P with x 1 and tag \"k\".
Let q be a new P with x 1 and tag \"k\".
Show p is equal to q.
Show [1, 2] is equal to [1, 2].
Show [\"a\"] is equal to [\"b\"].
",
    );
    means_the_same_everywhere(
        "## Main
Let present be some 3.
Let absent be none.
Let copied be copy of absent.
Show present is equal to absent.
Show absent is equal to present.
Show copied is equal to absent.
Show copied is equal to nothing.
",
    );
    means_the_same_everywhere(
        "## Main
Let v be none.
If v is equal to nothing:
    Show \"absent\".
Otherwise:
    Show \"present\".
",
    );
}

#[test]
fn a_sum_reduced_over_a_list_never_wraps() {
    means_the_same_or_rust_stops_at_the_word(
        "## To total (xs: Seq of Int) -> Int:
    Let mutable sum be 0.
    Repeat for x in xs:
        Set sum to sum + x.
    Return sum.

## Main
Let xs be [9223372036854775807, 1].
Show total(xs).
",
        "9223372036854775808",
    );
    means_the_same_everywhere(
        "## To total (xs: Seq of Int) -> Int:
    Let mutable sum be 0.
    Repeat for x in xs:
        Set sum to sum + x.
    Return sum.

## Main
Let xs be [9223372036854775807, 1, -2].
Show total(xs).
",
    );
    means_the_same_or_rust_stops_at_the_word(
        "## To total (xs: Seq of Int) -> Int:
    Let mutable sum be 0.
    Repeat for x in xs:
        Set sum to sum + x.
    Return sum.

## Main
Let mutable xs be a new Seq of Int.
Let mutable k be 0.
While k is less than 10001:
    Push 922337203685478 to xs.
    Set k to k + 1.
Show \"built\".
Show total(xs).
",
        "9224294374058465478",
    );
    means_the_same_everywhere(
        "## To total (xs: Seq of Int) -> Int:
    Let mutable sum be 0.
    Repeat for x in xs:
        Set sum to sum + x.
    Return sum.

## Main
Let mutable xs be a new Seq of Int.
Push 9223372036854775807 to xs.
Push 1 to xs.
Push -2 to xs.
Let mutable k be 0.
While k is less than 10000:
    Push 0 to xs.
    Set k to k + 1.
Show total(xs).
",
    );
}

/// `src` prints on every tier what the tree-walker prints, but a compiled tier whose `Int` is a machine word may stop
/// instead at a range whose bound is past the word: with the sentence the language gives such a bound, before the
/// range prints anything. It never fails to build.
fn walks_the_range_or_stops_at_the_word(src: &str) {
    let tw = tw_lines(src);
    assert_eq!(tw.error, None, "the tree-walker refused:\n{src}");
    let reference = written(&tw.lines);
    let (sentence, _) = logicaffeine_compile::semantics::collections::integer_place_refusals(
        logicaffeine_compile::semantics::collections::IntegerPlace::Range,
    );
    let mut report = String::new();
    for run in every_tier(src) {
        let same = run.failure.is_none() && run.output == reference;
        let compiled = run.tier.starts_with("AOT Rust") || run.tier == "AST C";
        // The C runtime opens each of its stops with `LOGOS runtime error: `.
        let stopped = compiled
            && run.output.is_empty()
            && run.failure.as_deref().is_some_and(|why| {
                let why = why.trim_end();
                why == sentence || why == format!("LOGOS runtime error: {sentence}")
            });
        if !same && !stopped {
            report.push_str(&format!("\n== {}\nprinted:\n{}failed: {:?}\n", run.tier, run.output, run.failure));
        }
    }
    assert!(report.is_empty(), "the tree-walker prints:\n{reference}for:\n{src}(a compiled tier may stop with: {sentence}){report}");
}

/// A range from an `Int` past the word walks it exactly on the tree-walker and the VM; AOT Rust builds it, and walks
/// it or stops with the range's sentence, as AST C stops (B27: `Repeat for i from big to big + 2` did not build).
#[test]
fn a_range_from_an_int_past_the_word_builds_and_walks_or_stops() {
    walks_the_range_or_stops_at_the_word(
        "## Main\nLet big be 9223372036854775807 + 1.\nRepeat for i from big to big + 2:\n    Show i.\n",
    );
}
