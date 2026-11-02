//! ★ LOCK — `Swap a and b.` exchanges two variables on every tier, whatever they hold and wherever they live. ★
//!
//! A rolling two-row table (`prev`/`curr` in a dynamic program) is the reason the statement exists outside
//! `Reversibly:` blocks: swapping the rows is O(1) where copying one is O(n). Each program here runs on the
//! tree-walker (the reference), the VM, the optimized program `largo run --interpret` executes (on the VM alone and
//! with the native tier installed), the Rust backend, the C backend, and — in the `wasm-jit` pass — the WebAssembly
//! backend, and every one prints the same bytes.
//!
//! The shapes cover what a swap can be handed: two scalars, two texts, two lists of different lengths, two
//! records, at `## Main` and inside a function, in a branch and in a loop, and a list that another binding
//! still holds, which must keep its value when the swapped names are changed afterwards.
//!
//! ★ A SMALL FIXED LIST STAYS FAST WHEN IT IS SWAPPED. A list built by a fixed run of pushes and read only at
//! constant positions becomes scalars on the run path and a `[T; N]` array in the Rust backend. A swap of two such
//! lists with the same element type and length keeps both there — it exchanges their elements (or the arrays)
//! instead — and the optimizations are checked to still fire, so correctness is never bought by losing them.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test swap_every_tier_lock`
//! (and with `--features wasm-jit` for the WebAssembly tier).

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "compile_common/mod.rs"]
mod compile_common;

use common::{assert_c_output, assert_exact_output};
use logicaffeine_compile::compile::{
    compile_to_rust, optimizations_fired, optimizations_fired_run, tw_outcome, vm_outcome,
    vm_outcome_tiered,
};
use logicaffeine_compile::optimization::Tier;
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

/// `src` prints exactly `expected` on every tier.
fn every_tier(src: &str, expected: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker refused:\n{src}");
    assert_eq!(
        tw.output.trim_end(),
        expected,
        "the tree-walker's answer:\n{src}"
    );

    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM refused:\n{src}");
    assert_eq!(vm.output.trim_end(), expected, "the VM's answer:\n{src}");

    let run = vm_outcome_tiered(src, &[], Tier::T3, None);
    assert_eq!(run.error, None, "the optimized run path refused:\n{src}");
    assert_eq!(
        run.output.trim_end(),
        expected,
        "the optimized run path's answer:\n{src}"
    );

    let tier = ForgeTier::new();
    let jit = vm_outcome_tiered(src, &[], Tier::T3, Some(&tier as &dyn NativeTier));
    assert_eq!(
        jit.error, None,
        "the optimized run path with the native tier refused:\n{src}"
    );
    assert_eq!(
        jit.output.trim_end(),
        expected,
        "the optimized run path with the native tier's answer:\n{src}"
    );

    assert_exact_output(src, expected);
    assert_c_output(src, expected);

    #[cfg(feature = "wasm-jit")]
    match compile_common::wasm_host::run_show(src) {
        Ok(got) => assert_eq!(
            got.trim_end(),
            expected,
            "the WebAssembly module's answer:\n{src}"
        ),
        Err(why) => panic!("the WebAssembly module could not run it ({why}):\n{src}"),
    }
}

#[test]
fn two_integers_at_main_trade_values() {
    every_tier(
        "## Main
Let mutable a be 1.
Let mutable b be 2.
Swap a and b.
Show a.
Show b.
",
        "2\n1",
    );
}

#[test]
fn two_texts_at_main_trade_values() {
    every_tier(
        "## Main
Let mutable a be \"left\".
Let mutable b be \"right\".
Swap a and b.
Show a.
Show b.
",
        "right\nleft",
    );
}

#[test]
fn two_lists_at_main_trade_values() {
    every_tier(
        "## Main
Let mutable a be a new Seq of Int.
Push 1 to a.
Let mutable b be a new Seq of Int.
Push 2 to b.
Swap a and b.
Show item 1 of a.
Show item 1 of b.
",
        "2\n1",
    );
}

#[test]
fn two_lists_of_different_lengths_trade_their_whole_contents() {
    every_tier(
        "## Main
Let mutable a be a new Seq of Int.
Push 1 to a.
Push 2 to a.
Push 3 to a.
Let mutable b be a new Seq of Int.
Push 9 to b.
Swap a and b.
Show length of a.
Show length of b.
Show a.
Show b.
",
        "1\n3\n[9]\n[1, 2, 3]",
    );
}

#[test]
fn two_lists_of_text_at_main_trade_values() {
    every_tier(
        "## Main
Let mutable a be a new Seq of Text.
Push \"x\" to a.
Let mutable b be a new Seq of Text.
Push \"y\" to b.
Push \"z\" to b.
Swap a and b.
Show a.
Show b.
",
        "[y, z]\n[x]",
    );
}

#[test]
fn two_records_at_main_trade_values() {
    every_tier(
        "## A Point has:
    An x: Int.
    A y: Int.

## Main
Let mutable p be a new Point with x 1 and y 2.
Let mutable q be a new Point with x 3 and y 4.
Swap p and q.
Show p's x.
Show q's y.
",
        "3\n2",
    );
}

#[test]
fn two_lists_inside_a_function_trade_values() {
    every_tier(
        "## To first (n: Int) -> Int:
    Let mutable a be a new Seq of Int.
    Push n to a.
    Let mutable b be a new Seq of Int.
    Push n + 1 to b.
    Swap a and b.
    Return item 1 of a.

## Main
Show first(1).
",
        "2",
    );
}

#[test]
fn a_swap_in_a_branch_exchanges_the_outer_lists() {
    every_tier(
        "## Main
Let mutable a be a new Seq of Int.
Push 1 to a.
Let mutable b be a new Seq of Int.
Push 2 to b.
Let flag be 1.
If flag equals 1:
    Swap a and b.
Show item 1 of a.
Show item 1 of b.
",
        "2\n1",
    );
}

#[test]
fn an_odd_number_of_swaps_in_a_loop_leaves_the_lists_exchanged() {
    every_tier(
        "## Main
Let mutable a be a new Seq of Int.
Push 1 to a.
Let mutable b be a new Seq of Int.
Push 2 to b.
Let mutable i be 0.
While i is less than 3:
    Swap a and b.
    Set i to i + 1.
Show item 1 of a.
Show item 1 of b.
",
        "2\n1",
    );
}

#[test]
fn a_swap_of_a_name_with_itself_changes_nothing() {
    every_tier(
        "## Main
Let mutable a be a new Seq of Int.
Push 7 to a.
Swap a and a.
Show a.
",
        "[7]",
    );
}

#[test]
fn a_list_another_binding_holds_keeps_its_value_after_the_swap() {
    every_tier(
        "## Main
Let mutable a be a new Seq of Int.
Push 1 to a.
Let mutable b be a new Seq of Int.
Push 2 to b.
Let kept be a.
Swap a and b.
Push 5 to b.
Set item 1 of a to 8.
Show kept.
Show a.
Show b.
",
        "[1]\n[8]\n[1, 5]",
    );
}

#[test]
fn a_rolling_two_row_table_writes_the_current_row_only() {
    every_tier(
        "## Main
Let mutable prev be a new Seq of Int.
Let mutable curr be a new Seq of Int.
Repeat for k from 1 to 4:
    Push 0 to prev.
    Push 0 to curr.
Repeat for row from 1 to 5:
    Repeat for k from 1 to 4:
        Set item k of curr to item k of prev + row * k.
    Swap prev and curr.
Show prev.
Show curr.
",
        "[15, 30, 45, 60]\n[10, 20, 30, 40]",
    );
}

#[test]
fn a_fixed_list_read_inside_a_record_constructor_keeps_its_value() {
    every_tier(
        "## A Point has:
    An x: Int.

## Main
Let mutable a be a new Seq of Int.
Push 5 to a.
Let p be a new Point with x (item 1 of a).
Show p's x.
",
        "5",
    );
}

const FIXED_PAIR: &str = "## Main
Let mutable a be a new Seq of Int.
Push 1 to a.
Push 2 to a.
Push 3 to a.
Let mutable b be a new Seq of Int.
Push 4 to b.
Push 5 to b.
Push 6 to b.
Swap a and b.
Set item 2 of a to item 2 of a * 10.
Show item 1 of a + item 3 of b.
Show item 2 of a.
Show item 2 of b.
";

#[test]
fn two_fixed_lists_of_one_length_trade_values() {
    every_tier(FIXED_PAIR, "7\n50\n2");
}

#[test]
fn two_fixed_lists_of_one_length_stay_fixed_when_swapped() {
    assert!(
        optimizations_fired_run(FIXED_PAIR).contains(&"scalarize"),
        "the run path must still scalarize two same-length fixed lists it swaps:\n{FIXED_PAIR}"
    );
    assert!(
        optimizations_fired(FIXED_PAIR).contains(&"scalarize"),
        "the Rust backend must still keep two same-length fixed lists it swaps as arrays:\n{FIXED_PAIR}"
    );
    let rust = compile_to_rust(FIXED_PAIR).expect("compiles to Rust");
    assert!(
        rust.contains("let mut a: [i64; 3]") && rust.contains("let mut b: [i64; 3]"),
        "both lists are `[i64; 3]` arrays:\n{rust}"
    );
}

#[test]
fn a_fixed_list_swapped_with_one_whose_length_is_read_trades_values() {
    every_tier(
        "## Main
Let mutable a be a new Seq of Int.
Push 1 to a.
Push 2 to a.
Push 3 to a.
Let mutable b be a new Seq of Int.
Push 4 to b.
Push 5 to b.
Push 6 to b.
Swap a and b.
Show item 1 of a.
Show length of b.
",
        "4\n3",
    );
}

#[test]
fn two_fixed_lists_of_one_length_inside_a_function_trade_values() {
    every_tier(
        "## To mixed (n: Int) -> Int:
    Let mutable a be a new Seq of Int.
    Push n to a.
    Push n + 1 to a.
    Let mutable b be a new Seq of Int.
    Push n * 10 to b.
    Push n * 20 to b.
    Swap a and b.
    Return item 1 of a + item 2 of b.

## Main
Show mixed(3).
",
        "34",
    );
}

#[test]
fn a_one_character_text_and_a_longer_one_trade_values() {
    every_tier(
        "## Main
Let mutable a be \"x\".
Let mutable b be \"hello\".
Swap a and b.
Show a.
Show b.
",
        "hello\nx",
    );
}

#[test]
fn two_integer_maps_trade_values() {
    every_tier(
        "## Main
Let mutable m be a new Map of Int to Int.
Set item 1 of m to 10.
Let mutable n be a new Map of Int to Int.
Set item 2 of n to 20.
Set item 3 of n to 30.
Swap m and n.
Show item 2 of m + item 3 of m.
Show item 1 of n.
",
        "50\n10",
    );
}

#[test]
fn a_local_list_and_one_a_record_also_holds_trade_values() {
    every_tier(
        "## A Box has:
    An items: Seq of Int.

## Main
Let mutable a be a new Seq of Int.
Push 1 to a.
Let mutable b be a new Seq of Int.
Push 2 to b.
Let bx be a new Box with items b.
Swap a and b.
Push 3 to a.
Show a.
Show b.
Show bx's items.
",
        "[2, 3]\n[1]\n[2]",
    );
}

#[test]
fn two_lists_built_by_an_affine_loop_trade_values() {
    every_tier(
        "## Main
Let mutable a be a new Seq of Int.
Repeat for i from 0 to 9:
    Push i * 5 to a.
Let mutable b be a new Seq of Int.
Repeat for i from 0 to 9:
    Push i * 3 to b.
Swap a and b.
Let mutable total be 0.
Repeat for k from 1 to 10:
    Set total to total + item k of a * 100 + item k of b.
Show total.
Show length of a.
",
        "13725\n10",
    );
}

#[test]
fn two_constant_tables_in_a_function_trade_values() {
    every_tier(
        "## To pick (k: Int) -> Int:
    Let mutable t be a new Seq of Int.
    Push 7 to t.
    Push 8 to t.
    Let mutable u be a new Seq of Int.
    Push 1 to u.
    Push 2 to u.
    Swap t and u.
    Return item k of t * 10 + item k of u.

## Main
Show pick(1).
Show pick(2).
",
        "17\n28",
    );
}

#[test]
fn a_constant_table_and_a_longer_list_in_a_function_trade_values() {
    every_tier(
        "## To pick (k: Int) -> Int:
    Let mutable t be a new Seq of Int.
    Push 7 to t.
    Push 8 to t.
    Let mutable u be a new Seq of Int.
    Push 1 to u.
    Push 2 to u.
    Push 3 to u.
    Swap t and u.
    Return item k of t * 10 + length of t + length of u.

## Main
Show pick(3).
",
        "35",
    );
}

#[test]
fn two_interleaved_columns_trade_values() {
    every_tier(
        "## Main
Let mutable xs be a new Seq of Float.
Let mutable ys be a new Seq of Float.
Push 1.0 to xs.
Push 2.0 to ys.
Push 3.0 to xs.
Push 4.0 to ys.
Push 5.0 to xs.
Push 6.0 to ys.
Swap xs and ys.
Let mutable total be 0.0.
Repeat for k from 1 to 3:
    Set total to total + item k of xs * 10.0 + item k of ys.
Show total.
",
        "129",
    );
}

#[test]
fn a_scratch_buffer_filled_by_a_loop_and_a_list_trade_values() {
    every_tier(
        "## To mix (n: Int) -> Int:
    Let mutable w be a new Seq of Int.
    Repeat for j from 1 to 4:
        Push j * n to w.
    Let mutable v be a new Seq of Int.
    Push 9 to v.
    Swap w and v.
    Return item 1 of w * 1000 + item 4 of v + length of w * 10.

## Main
Show mix(3).
",
        "9022",
    );
}

#[test]
fn an_integer_that_outgrew_a_machine_word_and_a_small_one_trade_values() {
    every_tier(
        "## Main
Let mutable a be 3.
Let mutable b be 5.
Repeat for i from 1 to 7:
    Set a to a * a.
Swap a and b.
Show a.
Show b.
",
        "5\n11790184577738583171520872861412518665678211592275841109096961",
    );
}

#[test]
fn a_map_a_function_reads_and_a_local_map_trade_values() {
    every_tier(
        "## To first (m: Map of Int to Int) -> Int:
    Return item 1 of m.

## Main
Let mutable m be a new Map of Int to Int.
Set item 1 of m to 10.
Let mutable n be a new Map of Int to Int.
Set item 1 of n to 20.
Show first(n).
Swap m and n.
Show item 1 of m.
Show first(n).
",
        "20\n20\n10",
    );
}

#[test]
fn two_parameters_trade_values_inside_their_function() {
    every_tier(
        "## To weigh (xs: Seq of Int, ys: Seq of Int) -> Int:
    Swap xs and ys.
    Return item 1 of xs * 10 + item 1 of ys.

## Main
Let a be [1, 2].
Let b be [3, 4].
Show weigh(a, b).
Show item 1 of a.
",
        "31\n1",
    );
}

#[test]
fn a_level_by_level_walk_swaps_its_frontier_and_next_level() {
    every_tier(
        "## Main
Let mutable frontier be a new Seq of Int.
Push 1 to frontier.
Let mutable next be a new Seq of Int.
Let mutable level be 0.
Let mutable total be 0.
While level is less than 5:
    Set next to a new Seq of Int.
    Repeat for v in frontier:
        Set total to total + v.
        Push v * 2 to next.
        Push v * 2 + 1 to next.
    Swap frontier and next.
    Set level to level + 1.
Show total.
Show length of frontier.
",
        "496\n32",
    );
}

/// ★ THE DIVISOR A SWAP CHANGES. The Rust backend replaces `/ n` and `% n` in a loop bounded by `n` with a magic
/// multiply computed once from `n` (`LogosDivU64`), which is sound only while `n` never changes. A `Swap` changes it,
/// and a check that did not count a swap as a write kept dividing by the old value: 500025 instead of 499005, with
/// no error anywhere.
#[test]
fn a_divisor_exchanged_inside_a_loop_divides_by_its_current_value() {
    every_tier(
        "## Main
Let mutable n be 1000.
Let mutable m be 997.
Let mutable total be 0.
Let mutable c be 0.
While c is less than n:
    Set total to total + (c * 7) % n + (c * 7) / n.
    Swap n and m.
    Set c to c + 1.
Show total.
",
        &bounded_divisor_swap_answer(),
    );
}

/// The exchanged loop-bound divisor's answer, computed here in plain Rust.
fn bounded_divisor_swap_answer() -> String {
    let (mut n, mut m, mut total, mut c) = (1000i64, 997i64, 0i64, 0i64);
    while c < n {
        total += (c * 7) % n + (c * 7) / n;
        std::mem::swap(&mut n, &mut m);
        c += 1;
    }
    total.to_string()
}

#[test]
fn two_divisors_exchanged_every_iteration_divide_by_the_current_one() {
    every_tier(
        "## Main
Let mutable d be 7.
Let mutable e be 3.
Let mutable total be 0.
Let mutable i be 0.
While i is less than 1000:
    Set total to total + i / d + i % d.
    Swap d and e.
    Set i to i + 1.
Show total.
",
        &divisor_swap_answer(),
    );
}

/// The exchanged-divisor loop's answer, computed here in plain Rust.
fn divisor_swap_answer() -> String {
    let (mut d, mut e, mut total) = (7i64, 3i64, 0i64);
    for i in 0..1000i64 {
        total += i / d + i % d;
        std::mem::swap(&mut d, &mut e);
    }
    total.to_string()
}

#[test]
fn a_parameter_and_a_local_list_trade_values() {
    every_tier(
        "## To weigh (xs: Seq of Int) -> Int:
    Let mutable local be a new Seq of Int.
    Push 9 to local.
    Push 8 to local.
    Push 7 to local.
    Swap xs and local.
    Return item 1 of xs * 100 + length of xs * 10 + item 1 of local.

## Main
Let a be [1, 2].
Show weigh(a).
Show a.
",
        "931\n[1, 2]",
    );
}

/// Run `src` on the optimized run path with a fresh native tier; answer its output and how many loop regions reached
/// native code.
fn native_regions(src: &str) -> (String, u64) {
    let tier = ForgeTier::new();
    let run = vm_outcome_tiered(src, &[], Tier::T3, Some(&tier as &dyn NativeTier));
    assert_eq!(run.error, None, "the native tier refused:\n{src}");
    let (_, regions) = tier.region_counts();
    (run.output.trim_end().to_string(), regions as u64)
}

/// ★ A LOOP THAT SWAPS STAYS NATIVE. A swap of two scalars is three register moves; a loop containing one used to be
/// refused by the native tier outright and ran on the bytecode interpreter, twenty times slower than the same loop
/// written with a temporary.
#[test]
fn a_loop_that_swaps_two_integers_runs_as_native_code() {
    let src = "## Main
Let mutable a be 1.
Let mutable b be 2.
Let mutable total be 0.
Let mutable i be 0.
While i is less than 5000:
    Set total to (total * 31 + a) % 1000000007.
    Swap a and b.
    Set i to i + 1.
Show total.
";
    let expected = {
        let (mut a, mut b, mut total) = (1i64, 2i64, 0i64);
        for _ in 0..5000 {
            total = (total * 31 + a) % 1_000_000_007;
            std::mem::swap(&mut a, &mut b);
        }
        total.to_string()
    };
    every_tier(src, &expected);
    let (out, regions) = native_regions(src);
    assert_eq!(out, expected);
    #[cfg(target_arch = "x86_64")]
    assert!(
        regions >= 1,
        "the swapping loop must compile to native code"
    );
}

#[test]
fn a_loop_that_swaps_two_reals_runs_as_native_code() {
    let src = "## Main
Let mutable a be 1.5.
Let mutable b be 0.25.
Let mutable total be 0.0.
Let mutable i be 0.
While i is less than 4000:
    Set total to total + a * 2.0.
    Swap a and b.
    Set i to i + 1.
Show total.
";
    let (out, regions) = native_regions(src);
    every_tier(src, &out);
    assert_eq!(out, "7000");
    #[cfg(target_arch = "x86_64")]
    assert!(
        regions >= 1,
        "the swapping loop must compile to native code"
    );
}

#[test]
fn a_hot_rolling_table_agrees_on_every_tier() {
    every_tier(
        "## Main
Let mutable prev be a new Seq of Int.
Let mutable curr be a new Seq of Int.
Repeat for k from 1 to 64:
    Push k to prev.
    Push 0 to curr.
Let mutable row be 1.
While row is at most 300:
    Let mutable k be 1.
    While k is at most 64:
        Set item k of curr to (item k of prev * 31 + row + k) % 1000003.
        Set k to k + 1.
    Swap prev and curr.
    Set row to row + 1.
Let mutable h be 0.
Repeat for k from 1 to 64:
    Set h to (h * 31 + item k of prev) % 268435456.
Show h.
",
        &hot_rolling_table_answer(),
    );
}

/// The hot rolling table's answer, computed here in plain Rust so the lock does not lean on any tier it checks.
fn hot_rolling_table_answer() -> String {
    let mut prev: Vec<i64> = (1..=64).collect();
    let mut curr = vec![0i64; 64];
    for row in 1..=300i64 {
        for k in 1..=64i64 {
            let i = (k - 1) as usize;
            curr[i] = (prev[i] * 31 + row + k) % 1_000_003;
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    let mut h = 0i64;
    for v in &prev {
        h = (h * 31 + v) % 268_435_456;
    }
    h.to_string()
}
