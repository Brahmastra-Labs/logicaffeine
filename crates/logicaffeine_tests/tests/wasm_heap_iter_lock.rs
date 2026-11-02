//! `Repeat` on the WebAssembly tier: its frames, what they hold, and how they end.
//!
//! A loop's iteration state — the snapshot it walks and its cursor — lives in a heap block from
//! `IterPrepare` to `IterPop`. Three properties hold it to the reference tiers:
//!
//! - a `Return` from inside a loop unwinds that function's loops, exactly as the VM truncates its
//!   iterator stack at a return, so the CALLER's loop keeps its own frame;
//! - a loop gives its block back when it ends — by `IterPop` or by a `Return` that leaves it — so
//!   running a loop a thousand times costs the heap no more than running it once;
//! - `For each c in <Text>` walks the text's characters (UTF-8 aware), one-character Text each.

#![cfg(all(feature = "wasm-jit", not(target_arch = "wasm32")))]

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

#[path = "compile_common/mod.rs"]
mod common;

/// The tree-walker, the VM and the WebAssembly module all run `src` and print `expected`.
fn all_three(src: &str, expected: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "tree-walker refused:\n{src}");
    assert_eq!(tw.output.trim_end(), expected, "tree-walker answer:\n{src}");
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "VM refused:\n{src}");
    assert_eq!(vm.output.trim_end(), expected, "VM answer:\n{src}");
    match common::wasm_host::run_show(src) {
        Ok(got) => assert_eq!(got.trim_end(), expected, "wasm answer:\n{src}"),
        Err(why) => panic!("wasm could not run it ({why}):\n{src}"),
    }
}

#[test]
fn returning_from_inside_a_loop_keeps_the_callers_loop() {
    all_three(
        "## To firstOver (xs: Seq of Int) and (k: Int) -> Int:\n\
         \x20   Repeat for x in xs:\n\
         \x20       If x is greater than k:\n\
         \x20           Return x.\n\
         \x20   Return 0.\n\
         \n\
         ## Main\n\
         Let outer be [10, 20, 30].\n\
         Repeat for y in outer:\n\
         \x20   Show y + firstOver([1, 5, 9], 2).\n",
        "15\n25\n35",
    );
}

#[test]
fn returning_from_inside_two_nested_loops_unwinds_both() {
    all_three(
        "## To firstPair (xs: Seq of Int) and (ys: Seq of Int) -> Int:\n\
         \x20   Repeat for x in xs:\n\
         \x20       Repeat for y in ys:\n\
         \x20           If x + y is equal to 7:\n\
         \x20               Return x * 10 + y.\n\
         \x20   Return 0.\n\
         \n\
         ## Main\n\
         Repeat for k in [1, 2, 3]:\n\
         \x20   Show k.\n\
         \x20   Show firstPair([1, 2, 3], [4, 5, 6]).\n\
         Show \"done\".\n",
        "1\n16\n2\n16\n3\n16\ndone",
    );
}

#[test]
fn a_nothing_returning_function_leaves_its_loop_too() {
    all_three(
        "## To report (xs: Seq of Int):\n\
         \x20   Repeat for x in xs:\n\
         \x20       If x is equal to 2:\n\
         \x20           Show \"two\".\n\
         \x20           Return.\n\
         \n\
         ## Main\n\
         Repeat for k in [7, 8]:\n\
         \x20   report([1, 2, 3]).\n\
         \x20   Show k.\n",
        "two\n7\ntwo\n8",
    );
}

/// The heap a program has claimed when it ends, running `total` — a function that walks a 32-element
/// list — `calls` times.
fn heap_after_calls(calls: usize, early_return: bool) -> u32 {
    let body = if early_return {
        "\x20   Let mutable t be 0.\n\
         \x20   Repeat for x in xs:\n\
         \x20       If x is greater than 30:\n\
         \x20           Return t.\n\
         \x20       Set t to t + x.\n\
         \x20   Return t.\n"
    } else {
        "\x20   Let mutable t be 0.\n\
         \x20   Repeat for x in xs:\n\
         \x20       Set t to t + x.\n\
         \x20   Return t.\n"
    };
    let items: Vec<String> = (1..=32).map(|i| i.to_string()).collect();
    let src = format!(
        "## To total (xs: Seq of Int) -> Int:\n{body}\n\
         ## Main\n\
         Let xs be [{}].\n\
         Let mutable s be 0.\n\
         Let mutable i be 0.\n\
         While i is less than {calls}:\n\
         \x20   Set s to s + total(xs).\n\
         \x20   Set i to i + 1.\n\
         Show s.\n",
        items.join(", ")
    );
    let run =
        common::wasm_host::run_outcome(&src).unwrap_or_else(|e| panic!("it must run: {e}\n{src}"));
    assert_eq!(run.failure, None, "it must not fail:\n{src}");
    run.heap_ptr
        .expect("a module that builds a list has a heap")
}

#[test]
fn a_loop_returns_its_snapshot() {
    let (few, many) = (heap_after_calls(10, false), heap_after_calls(1000, false));
    assert!(
        many.saturating_sub(few) < 64,
        "990 more runs of a 32-element loop claimed {} more bytes of heap ({few} → {many}): the \
         loop's snapshot is not given back when the loop ends",
        many.saturating_sub(few)
    );
}

#[test]
fn a_loop_left_by_return_returns_its_snapshot() {
    let (few, many) = (heap_after_calls(10, true), heap_after_calls(1000, true));
    assert!(
        many.saturating_sub(few) < 64,
        "990 more runs of a loop left by `Return` claimed {} more bytes of heap ({few} → {many}): \
         the frames a return unwinds are not given back",
        many.saturating_sub(few)
    );
}

#[test]
fn a_text_iterates_its_characters() {
    all_three(
        "## To spell (t: Text) -> Text:\n\
         \x20   Let mutable out be \"\".\n\
         \x20   Let mutable n be 0.\n\
         \x20   For each c in t:\n\
         \x20       Set n to n + 1.\n\
         \x20       Set out to out + c + \"-\".\n\
         \x20   Return out + format(n).\n\
         \n\
         ## Main\n\
         Show spell(\"héllo\").\n\
         Show spell(\"\").\n\
         Show spell(\"a€😀\").\n",
        "h-é-l-l-o-5\n0\na-€-😀-3",
    );
}

#[test]
fn the_stdlib_text_walkers_run_on_wasm() {
    // `forms.lg`'s `looksLikeEmail` (behind `checkEmail`) and `ui.lg`'s `truncate`/`initials` are all
    // `For each c in <Text>` loops, so a page using any of them needs the walk on this tier.
    all_three(
        "## Main\n\
         Show looksLikeEmail(\"ada@example.com\").\n\
         Show looksLikeEmail(\"nope\").\n\
         Show truncate(\"Hello world\", 5).\n\
         Show initials(\"Ada Lovelace\").\n",
        "true\nfalse\nHell…\nAL",
    );
}

#[test]
fn the_session_cookie_scanners_run_on_wasm() {
    // `session.lg`'s `textUpTo`/`textFrom`/`lastSeparator` walk a cookie's characters to split it at
    // its signature separator — the same `For each c in <Text>` shape, gating `## Session` CSRF.
    all_three(
        "## Main\n\
         Show textUpTo(\"hello|MAC\", 5).\n\
         Show textFrom(\"hello|MAC\", 5).\n\
         Show lastSeparator(\"a|b|MAC\").\n\
         Show lastSeparator(\"none here\").\n",
        "hello\n|MAC\n4\n0",
    );
}
