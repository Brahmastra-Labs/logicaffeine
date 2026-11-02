//! Map reclamation must preserve values, copied containers, and their exact holder counts.
#![cfg(feature = "wasm-jit")]
#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::mount_interactive_checked;

fn flat(state: &str, action: &str, shown: &str, expected: &str) {
    let mut observed = "\"\"".to_string();
    for (i, part) in shown.split(['{', '}']).enumerate() {
        if part.is_empty() {
            continue;
        }
        if i % 2 == 1 {
            observed.push_str(&format!(" combined with ({part})"));
        } else {
            observed.push_str(&format!(" combined with {part:?}"));
        }
    }
    observed.push_str(" combined with \":\" combined with stamp");
    let src = format!("## Component App\nState:\n{state}    Let observed be \"\".\n    Let stamp be 0.\nActions:\n    To step:\n{action}        Set stamp to 1 - stamp.\n        Set observed to {observed}.\nView:\n    <div><button id=\"a\" on-click={{step}}>a</button><p>{{observed}}</p></div>\n\n## Main\nMount App.\n");
    let mut page = mount_interactive_checked(&src);
    let button = *page.listening("click").last().expect("button");
    let mut warm = 0;
    for turn in 1..=1000 {
        page.dispatch(button, "click", "")
            .unwrap_or_else(|e| panic!("turn {turn}: {e}\n{src}"));
        let journal = page.journal();
        let entry = journal.last().expect("completed action");
        assert_eq!(entry.name, "step");
        assert_eq!(entry.failure, "", "turn {turn}: {entry:?}");
        let change = entry
            .changes
            .chunks_exact(3)
            .find(|c| c[0] == "observed")
            .unwrap_or_else(|| panic!("turn {turn}: missing observed value: {entry:?}\n{src}"));
        assert_eq!(
            change[2],
            format!("{expected}:{}", turn % 2),
            "turn {turn}: observed map value\n{src}"
        );
        assert_eq!(
            page.census(),
            0,
            "turn {turn}: every reachable holder is counted exactly\n{src}"
        );
        if turn == 100 {
            warm = page.heap_allocated();
        }
    }
    assert_eq!(
        page.heap_allocated(),
        warm,
        "bounded map workload keeps no bytes per turn\n{src}"
    );
}

#[test]
fn empty_maps_replaced_each_turn_are_freed() {
    flat(
        "    Let rows be a new Map of Int to Int.\n",
        "        Set rows to a new Map of Int to Int.\n",
        "{length of rows}",
        "0",
    );
}

#[test]
fn maps_release_scalar_entries() {
    flat("    Let rows be a new Map of Int to Int.\n", "        Let mutable fresh be a new Map of Int to Int.\n        Set item 4 of fresh to 7.\n        Set rows to fresh.\n", "{item 4 of rows}", "7");
}

#[test]
fn maps_release_text_keys_and_text_values() {
    flat("    Let rows be a new Map of Text to Text.\n", "        Let mutable fresh be a new Map of Text to Text.\n        Let key be \"k\" combined with 4.\n        Let value be \"v\" combined with 7.\n        Set item key of fresh to value.\n        Set rows to fresh.\n", "{item \"k4\" of rows}", "v7");
}

#[test]
fn copied_maps_keep_their_own_sequence_values() {
    flat("    Let rows be a new Map of Int to (Seq of Int).\n    Let saved be a new Map of Int to (Seq of Int).\n", "        Set item 1 of rows to [4, 5].\n        Set saved to rows.\n        Set item 1 of rows to [9].\n", "{length of item 1 of saved}:{length of item 1 of rows}", "2:1");
}

#[test]
fn removing_a_map_entry_releases_both_owned_handles() {
    flat("    Let rows be a new Map of Text to (Seq of Int).\n", "        Let key be \"k\" combined with 4.\n        Set item key of rows to [4, 5].\n        Remove key from rows.\n", "{length of rows}", "0");
}

#[test]
fn shared_child_edges_are_counted_once_per_container() {
    flat("    Let rows be a new Map of Int to (Seq of Int).\n    Let saved be a new Map of Int to (Seq of Int).\n", "        Let xs be [4, 5].\n        Set item 1 of rows to xs.\n        Set item 2 of rows to xs.\n        Set saved to rows.\n", "{length of item 1 of saved}:{length of item 2 of rows}", "2:2");
}

#[test]
fn removing_a_text_set_member_releases_its_owned_handle() {
    flat("    Let members be a new Set of Text.\n", "        Let member be \"v\" combined with 7.\n        Add member to members.\n        Remove member from members.\n", "{length of members}", "0");
}

#[test]
fn removing_from_a_copy_preserves_the_saved_map_value() {
    flat("    Let rows be a new Map of Text to (Seq of Int).\n    Let saved be a new Map of Text to (Seq of Int).\n",
         "        Let key be \"k\" combined with 4.\n        Set item key of rows to [4, 5].\n        Set saved to rows.\n        Remove key from rows.\n",
         "{length of item \"k4\" of saved}:{length of rows}", "2:0");
}

#[test]
fn removing_scalar_entries_never_treats_payload_bits_as_handles() {
    flat("    Let rows be a new Map of Int to Int.\n",
         "        Set item 4294967295 of rows to 9223372036854775807.\n        Remove 4294967295 from rows.\n", "{length of rows}", "0");
}

#[test]
fn removing_a_missing_key_preserves_the_existing_owned_value() {
    flat("    Let rows be a new Map of Text to (Seq of Int).\n",
         "        Let key be \"k\" combined with 4.\n        Set item key of rows to [4, 5].\n        Remove \"absent\" from rows.\n",
         "{length of item \"k4\" of rows}", "2");
}
