//! ★ LOCK — appending to a Text the program holds alone is amortised O(1) on the tree-walker. ★
//!
//! `Set out to out + piece.` in a loop is how an ordinary program builds a string. Built by copying
//! the whole of `out` into a fresh string on every append, N appends cost O(N²): 80 000 one-letter
//! appends took 2.2 s on the tree-walker against 30 ms on the VM, which appends in place. The
//! tree-walker appends in place too whenever nothing else holds the string, and copies first when
//! something does, so no other binding ever sees the change.
//!
//! ★ THE YARDSTICK IS A BYTE COUNT, NOT A CLOCK. This binary's allocator counts the bytes the
//! calling thread allocates while the program runs. Doubling the number of appends doubles that
//! count when each append is amortised O(1), and quadruples it when each one copies the string, so
//! the growth ratio separates the two with no timing noise, whatever the box's load.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::interpreter::Interpreter;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static BYTES: Cell<u64> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        System.alloc(layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(new_size);
        System.realloc(ptr, layout, new_size)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
}

fn count(bytes: usize) {
    let _ = COUNTING.try_with(|on| {
        if on.get() {
            let _ = BYTES.try_with(|b| b.set(b.get() + bytes as u64));
        }
    });
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The bytes the tree-walker allocates running `src` (its run only, not its parse), and its output.
fn bytes_to_run(src: &str) -> (u64, Vec<String>) {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, policies) = parsed.unwrap_or_else(|e| panic!("parses:\n{src}\n=> {e}"));
        let mut interp = Interpreter::new(interner)
            .with_type_registry(types)
            .with_policies(policies);
        BYTES.with(|b| b.set(0));
        COUNTING.with(|on| on.set(true));
        let ran = interp.run_sync(stmts);
        COUNTING.with(|on| on.set(false));
        ran.unwrap_or_else(|e| panic!("runs:\n{src}\n=> {e}"));
        (BYTES.with(Cell::get), interp.output.clone())
    })
}

fn builder(appends: usize, append: &str) -> String {
    format!(
        "## Main\nLet mutable out be \"\".\nRepeat for i from 1 to {appends}:\n    {append}\nShow length of out.\n"
    )
}

/// The byte count's growth when the number of appends doubles.
fn growth(append: &str, per_append: usize) -> f64 {
    const N: usize = 20_000;
    let (small, small_out) = bytes_to_run(&builder(N, append));
    let (large, large_out) = bytes_to_run(&builder(2 * N, append));
    assert_eq!(
        small_out,
        [(N * per_append).to_string()],
        "the program built the whole string"
    );
    assert_eq!(large_out, [(2 * N * per_append).to_string()]);
    large as f64 / small as f64
}

#[test]
fn a_single_append_in_a_loop_allocates_linearly() {
    let ratio = growth("Set out to out + \"x\".", 1);
    assert!(
        ratio < 2.6,
        "doubling the appends grew the allocated bytes {ratio:.2}× — each append copies the string"
    );
}

#[test]
fn a_chain_of_appends_in_one_sentence_allocates_linearly() {
    let ratio = growth("Set out to out + \"ab\" + \"c\".", 3);
    assert!(
        ratio < 2.6,
        "doubling the appends grew the allocated bytes {ratio:.2}× — each append copies the string"
    );
}

#[test]
fn a_combined_with_append_allocates_linearly() {
    let ratio = growth("Set out to out combined with \"x\".", 1);
    assert!(
        ratio < 2.6,
        "doubling the appends grew the allocated bytes {ratio:.2}× — each append copies the string"
    );
}

/// Every in-place append means what the copying one means, on the tree-walker and the VM alike.
fn agree(src: &str, expected: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker runs it:\n{src}");
    assert_eq!(tw.output.trim(), expected, "tree-walker:\n{src}");
    let vm = vm_outcome(src);
    assert_eq!(
        (vm.output.trim(), vm.error.as_deref()),
        (expected, None),
        "the VM agrees:\n{src}"
    );
}

#[test]
fn another_binding_of_the_same_text_never_sees_an_append() {
    agree(
        "## Main\nLet a be \"x\".\nLet mutable b be a.\nSet b to b + \"y\".\nShow a.\nShow b.\n",
        "x\nxy",
    );
}

#[test]
fn a_text_held_in_a_list_never_sees_an_append_to_its_binding() {
    agree(
        "## Main\nLet mutable s be \"x\".\nLet xs be [s].\nSet s to s + \"y\".\nShow item 1 of xs.\nShow s.\n",
        "x\nxy",
    );
}

#[test]
fn an_append_that_reads_its_own_target_reads_the_value_before_it() {
    agree(
        "## Main\nLet mutable s be \"ab\".\nSet s to s + s + s.\nShow s.\n",
        "ababab",
    );
}

#[test]
fn an_append_whose_operand_rewrites_the_target_keeps_the_value_it_read_first() {
    agree(
        "## To grow -> Text:\n    Set s to \"zzz\".\n    Return \"!\".\n\n## Main\nLet mutable s be \"a\".\nSet s to s + grow().\nShow s.\n",
        "a!",
    );
}

#[test]
fn a_non_text_operand_is_appended_as_it_shows() {
    agree(
        "## Main\nLet mutable s be \"n=\".\nSet s to s + 42 + \" \" + 1.5 + true.\nShow s.\n",
        "n=42 1.5true",
    );
}

#[test]
fn a_number_target_still_adds() {
    agree(
        "## Main\nLet mutable n be 1.\nSet n to n + 2 + 3.\nShow n.\n",
        "6",
    );
}
