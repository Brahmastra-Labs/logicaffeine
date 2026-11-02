//! Trace gross/live heap and independently checked counts beyond allocation size transitions.
#![cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::mount_interactive_checked;

fn sample(records: &str, state: &str, action: &str, shown: &str) {
    let src = format!("{records}## Component App\nState:\n{state}    Let tick be 0.\nActions:\n    To add:\n        Set tick to tick + 1.\n{action}View:\n    <div><button id=\"a\" on-click={{add}}>a</button><p>{shown}</p></div>\n\n## Main\nMount App.\n");
    let mut page = mount_interactive_checked(&src);
    let button = *page.listening("click").last().unwrap();
    let mut mismatches = Vec::new();
    for turn in 1..=6000 {
        page.dispatch(button, "click", "")
            .unwrap_or_else(|e| panic!("turn {turn}: {e}"));
        if [1, 9, 10, 99, 100, 200, 999, 1000, 2000, 4000, 6000].contains(&turn) {
            let block = page.census();
            let words: Vec<_> = if block == 0 {
                vec![]
            } else {
                (0..5).map(|i| page.word(block + i * 4)).collect()
            };
            eprintln!(
                "turn={turn} gross={} used={} mismatch={block} header={words:?}",
                page.heap_allocated(),
                page.heap_used()
            );
            if block != 0 {
                mismatches.push((turn, block, words));
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "all checkpoints require exact holder counts: {mismatches:?}"
    );
}

#[test]
fn text_replacement_counts_remain_exact_across_size_classes() {
    sample(
        "",
        "    Let label be \"\".\n",
        "        Set label to \"turn \" combined with tick.\n",
        "{label}",
    );
}

#[test]
fn record_list_counts_remain_exact_across_size_classes() {
    sample("## A Note has:\n    a n, which is Int.\n\n", "    Let notes be a new Seq of Note.\n",
        "        Let mutable fresh be a new Seq of Note.\n        Push (a new Note with n tick) to fresh.\n        Set notes to fresh.\n", "{length of notes}");
}
