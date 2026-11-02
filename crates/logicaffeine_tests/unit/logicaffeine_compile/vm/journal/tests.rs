use super::*;
use crate::vm::value::Value;

#[test]
fn fresh_tape_is_empty_and_zero_garbage() {
    let tape = HistoryTape::new();
    assert!(tape.is_empty());
    assert_eq!(tape.len(), 0);
    assert_eq!(tape.erased_bytes(), 0, "an empty tape has erased nothing");
}

#[test]
fn control_and_derivable_entries_are_zero_erased_bytes() {
    // A `Show` line and a branch direction record CONTROL, not erased data — the abelian
    // zero-erasure claim must not be diluted by them.
    let mut tape = HistoryTape::new();
    tape.push(JournalEntry::Branch { taken: true });
    tape.push(JournalEntry::ShowLine);
    assert_eq!(tape.len(), 2);
    assert_eq!(
        tape.erased_bytes(),
        0,
        "control/derivable entries erase no data"
    );
}

#[test]
fn value_overwrite_records_erased_bytes() {
    let mut tape = HistoryTape::new();
    tape.push(JournalEntry::RegOverwrite {
        slot: 3,
        old: Value::int(7),
    });
    tape.push(JournalEntry::GlobalOverwrite {
        idx: 0,
        old: Some(Value::int(9)),
    });
    // A first-definition (old == None) erased nothing; only the displaced value costs.
    tape.push(JournalEntry::GlobalOverwrite { idx: 1, old: None });
    assert_eq!(tape.len(), 3);
    assert_eq!(tape.erased_bytes(), 2 * std::mem::size_of::<Value>());
}

#[test]
fn tape_pops_in_strict_reverse_order() {
    // Reverse execution depends on LIFO discipline: the last op's delta is undone first.
    let mut tape = HistoryTape::new();
    tape.push(JournalEntry::Branch { taken: false });
    tape.push(JournalEntry::RegOverwrite {
        slot: 1,
        old: Value::int(42),
    });
    match tape.pop() {
        Some(JournalEntry::RegOverwrite { slot, .. }) => assert_eq!(slot, 1),
        other => panic!("expected the last-pushed RegOverwrite, got {other:?}"),
    }
    match tape.pop() {
        Some(JournalEntry::Branch { taken }) => assert!(!taken),
        other => panic!("expected the Branch, got {other:?}"),
    }
    assert!(tape.pop().is_none());
    assert!(tape.is_empty());
}
