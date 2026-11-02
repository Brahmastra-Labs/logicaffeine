//! ★ LOCK — nothing a program drops stays on the wasm heap. ★
//!
//! A page that runs N turns and then ten times as many ends with the heap it had after the first N:
//! every value a turn drops (a list replaced, a record replaced, a map rebuilt, a map value or a
//! record field overwritten, a text re-rendered) goes back to the allocator at the turn's
//! safepoint, and the allocator hands the same blocks to the next turn. The measure is the gross
//! heap, `__logos_heap_allocated` (every byte the allocator has ever claimed), on the module a user
//! is shipped (`compile_to_wasm`): a leak of any size per turn moves it, where a rate (bytes per
//! turn, early against late) lets a constant leak through. Each page also runs as the check-mode
//! module (`compile_to_wasm_checked`), whose census recomputes every counted value's holder count
//! from the page's own globals and must find it exact.
//!
//! A program that runs to completion holds the same line: a loop that builds and drops a value per
//! iteration ends with the bump pointer (`__heap_ptr`) where the same loop over a tenth of the
//! iterations ends it.
//!
//! One page per kind of value, so each kind is held on its own.

#![cfg(feature = "wasm-jit")]

#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::{
    mount_interactive, mount_interactive_checked, run_outcome, InteractivePage,
};

/// Turns before the first reading: past every first-time growth (a free list's first block of each
/// size, a registry's first slot).
const N: usize = 200;

/// A page with one button `add`, its state `state`, the action body `act` (run once per click) and
/// the view text `shown`.
fn page(records: &str, state: &str, act: &str, shown: &str) -> String {
    format!(
        "{records}## Component App\nState:\n{state}    Let tick be 0.\nActions:\n    To add:\n        Set tick to tick + 1.\n{act}View:\n    <div><button id=\"a\" on-click={{add}}>a</button><p>{shown}</p></div>\n\n## Main\nMount App.\n"
    )
}

fn click(page: &mut InteractivePage, button: i64, turn: usize) {
    page.dispatch(button, "click", "")
        .unwrap_or_else(|e| panic!("turn {turn} traps: {e}"));
}

/// The gross heap after `N` turns and after `10 N`, on the shipped module; then the census after
/// `10 N` turns on the check-mode module.
fn holds_flat(name: &str, src: &str) {
    let mut shipped = mount_interactive(src);
    let button = *shipped
        .listening("click")
        .last()
        .unwrap_or_else(|| panic!("{name}: the page listens for a click"));
    for turn in 0..N {
        click(&mut shipped, button, turn);
    }
    let at_n = shipped.heap_allocated();
    for turn in N..10 * N {
        click(&mut shipped, button, turn);
    }
    let at_10n = shipped.heap_allocated();
    assert_eq!(
        at_10n,
        at_n,
        "{name}: the heap after {} turns is {at_10n} bytes against {at_n} after {N} — {} bytes a turn kept\n{src}",
        10 * N,
        f64::from(at_10n.saturating_sub(at_n)) / (9 * N) as f64,
    );
    let mut checked = mount_interactive_checked(src);
    let button = *checked
        .listening("click")
        .last()
        .expect("the check-mode page listens for a click");
    for turn in 0..10 * N {
        click(&mut checked, button, turn);
    }
    assert_eq!(
        checked.census(),
        0,
        "{name}: every counted value's holder count is exact after {} turns",
        10 * N
    );
}

#[test]
fn a_counter_page_keeps_nothing_it_renders() {
    holds_flat("counter", &page("", "", "", "{tick}"));
}

#[test]
fn a_text_replaced_every_turn_is_freed() {
    holds_flat(
        "text",
        &page(
            "",
            "    Let label be \"\".\n",
            "        Set label to \"turn \" combined with tick.\n",
            "{label}",
        ),
    );
}

#[test]
fn a_list_literal_replaced_every_turn_is_freed() {
    holds_flat(
        "list literal",
        &page(
            "",
            "    Let items be [0].\n",
            // Cycle payload widths: page-state text gains a size class at 999/1000.
            "        Set tick to tick % 100.\n        Set items to [tick, tick + 1, tick + 2].\n",
            "{length of items}",
        ),
    );
}

#[test]
fn a_list_built_from_an_empty_list_is_freed() {
    holds_flat(
        "built list",
        &page(
            "",
            "    Let items be a new Seq of Int.\n",
            "        Let mutable fresh be a new Seq of Int.\n        Push tick to fresh.\n        Push tick + 1 to fresh.\n        Set items to fresh.\n",
            "{length of items}",
        ),
    );
}

#[test]
fn a_record_replaced_every_turn_is_freed() {
    holds_flat(
        "record",
        &page(
            "## A Note has:\n    a n, which is Int.\n\n",
            "    Let current be a new Note with n 0.\n",
            "        Set current to a new Note with n tick.\n",
            "{current's n}",
        ),
    );
}

#[test]
fn a_list_of_records_replaced_every_turn_is_freed() {
    holds_flat(
        "records",
        &page(
            "## A Note has:\n    a n, which is Int.\n\n",
            "    Let notes be a new Seq of Note.\n",
            "        Set tick to tick % 100.\n        Let mutable fresh be a new Seq of Note.\n        Push (a new Note with n tick) to fresh.\n        Set notes to fresh.\n",
            "{length of notes}",
        ),
    );
}

#[test]
fn a_map_rebuilt_every_turn_is_freed() {
    holds_flat(
        "map",
        &page(
            "",
            "    Let counts be a new Map of Text to Int.\n",
            "        Let mutable fresh be a new Map of Text to Int.\n        Set item \"turn\" of fresh to tick.\n        Set counts to fresh.\n",
            "{length of counts}",
        ),
    );
}

#[test]
fn a_map_value_overwritten_every_turn_is_freed() {
    holds_flat(
        "map value",
        &page(
            "",
            "    Let rows be a new Map of Text to (Seq of Int).\n",
            "        Set item \"row\" of rows to [tick, tick + 1].\n",
            "{length of rows}",
        ),
    );
}

#[test]
fn a_record_field_overwritten_every_turn_is_freed() {
    holds_flat(
        "record field",
        &page(
            "## A Box has:\n    a tape, which is Seq of Int.\n\n",
            "    Let box be a new Box with tape [0].\n",
            "        Set box's tape to [tick, tick + 1].\n",
            "{length of box's tape}",
        ),
    );
}

/// The bump pointer after a program whose loop runs `iterations` times.
fn heap_after(name: &str, body: &str, iterations: usize) -> u32 {
    let src = format!(
        "## Main\nLet mutable total be 0.\nRepeat for i from 1 to {iterations}:\n{body}Show total.\n"
    );
    let run = run_outcome(&src).unwrap_or_else(|e| panic!("{name}: {e}\n{src}"));
    assert_eq!(
        run.failure, None,
        "{name}: the program runs to its end\n{src}"
    );
    run.heap_ptr
        .unwrap_or_else(|| panic!("{name}: a program that builds values has a heap\n{src}"))
}

fn loop_holds_flat(name: &str, body: &str) {
    let (short, long) = (
        heap_after(name, body, 1_000),
        heap_after(name, body, 10_000),
    );
    assert_eq!(
        long,
        short,
        "{name}: ten times the iterations end with the heap at {long} against {short} — {} bytes an iteration kept",
        f64::from(long.saturating_sub(short)) / 9_000.0,
    );
}

#[test]
fn a_loop_dropping_a_list_each_iteration_frees_it() {
    loop_holds_flat(
        "loop list",
        "    Let xs be [i, i + 1].\n    Set total to total + item 1 of xs.\n",
    );
}

#[test]
fn a_loop_dropping_a_text_each_iteration_frees_it() {
    loop_holds_flat(
        "loop text",
        "    Let s be \"n\" combined with i.\n    Set total to total + length of s.\n",
    );
}

#[test]
fn a_loop_dropping_a_map_each_iteration_frees_it() {
    loop_holds_flat(
        "loop map",
        "    Let mutable m be a new Map of Int to Int.\n    Set item i of m to i.\n    Set total to total + length of m.\n",
    );
}
