//! ★ LOCK — `noteTurn` writes one whole journal entry on the tiers that run lowered code. ★
//!
//! A lowered page journals each turn itself, through the reserved builtin
//! `noteTurn(name, key, changes, failure)`: the action's journal name, the instance key it ran
//! against, every field whose display changed as a flattened `[field, before, after, …]`
//! `Seq of Text`, and the failure that rolled it back (`""` when the turn committed). The builtin is
//! the one place a lowered tier's journal entry is made, so what it writes is exactly the
//! reference's `JournalEntry` shape: triples, in order, and `failed` present only for a failure.
//! Each row runs on the tree-walker and on the bytecode VM, the two in-process runners of lowered
//! code — hosts that keep the journal, so `journalDetail()`, which a lowered turn asks before
//! capturing its changes, answers true on both.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome, RunOutcome};
use logicaffeine_compile::semantics::dom;
use logicaffeine_compile::semantics::world::{self, JournalEntry};

type Runner = fn(&str) -> RunOutcome;
const RUNNERS: [(&str, Runner); 2] = [("tree-walker", tw_outcome), ("vm", vm_outcome)];

/// `src` on `run`, from a clean journal: its outcome's error, and the journal it left.
fn journal_of(run: Runner, src: &str) -> (Option<String>, Vec<JournalEntry>) {
    dom::reset();
    world::reset();
    let out = run(src);
    (out.error, world::journal())
}

fn triple(f: &str, b: &str, a: &str) -> (String, String, String) {
    (f.to_string(), b.to_string(), a.to_string())
}

#[test]
fn a_committed_turn_is_journalled_with_its_instance_and_every_change_in_order() {
    let src = "## Main\nLet changes be [\"n\", \"0\", \"1\", \"label\", \"zero\", \"one\"].\nCall noteTurn with \"bump\" and \"/0.0#k\" and changes and \"\".\n";
    for (tier, run) in RUNNERS {
        let (error, journal) = journal_of(run, src);
        assert_eq!(error, None, "{tier}: the program runs");
        assert_eq!(
            journal.len(),
            1,
            "{tier}: one call is one entry: {journal:?}"
        );
        let e = &journal[0];
        assert_eq!(
            (e.action.as_str(), e.instance.as_str()),
            ("bump", "/0.0#k"),
            "{tier}: {e:?}"
        );
        assert_eq!(
            e.changes,
            vec![triple("n", "0", "1"), triple("label", "zero", "one")],
            "{tier}: {e:?}"
        );
        assert_eq!(
            e.failed, None,
            "{tier}: an empty failure is a turn that committed: {e:?}"
        );
    }
}

#[test]
fn a_failed_turn_is_journalled_with_its_failure_and_no_changes() {
    let src = "## Main\nLet none be a new Seq of Text.\nCall noteTurn with \"half\" and \"\" and none and \"index 9 is out of range\".\n";
    for (tier, run) in RUNNERS {
        let (error, journal) = journal_of(run, src);
        assert_eq!(error, None, "{tier}: the program runs");
        assert_eq!(journal.len(), 1, "{tier}: {journal:?}");
        let e = &journal[0];
        assert_eq!(
            (e.action.as_str(), e.instance.as_str()),
            ("half", ""),
            "{tier}: a mounted root's key is empty: {e:?}"
        );
        assert!(e.changes.is_empty(), "{tier}: {e:?}");
        assert_eq!(
            e.failed.as_deref(),
            Some("index 9 is out of range"),
            "{tier}: {e:?}"
        );
    }
}

#[test]
fn turns_are_journalled_in_the_order_they_were_noted() {
    let src = "## Main\nLet none be a new Seq of Text.\nLet one be [\"x\", \"1\", \"2\"].\nCall noteTurn with \"a\" and \"\" and none and \"\".\nCall noteTurn with \"b\" and \"/0.0#1\" and one and \"\".\nCall noteTurn with \"c\" and \"\" and none and \"boom\".\n";
    for (tier, run) in RUNNERS {
        let (error, journal) = journal_of(run, src);
        assert_eq!(error, None, "{tier}: the program runs");
        let seen: Vec<(&str, &str, usize, Option<&str>)> = journal
            .iter()
            .map(|e| {
                (
                    e.action.as_str(),
                    e.instance.as_str(),
                    e.changes.len(),
                    e.failed.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            seen,
            vec![
                ("a", "", 0, None),
                ("b", "/0.0#1", 1, None),
                ("c", "", 0, Some("boom"))
            ],
            "{tier}"
        );
    }
}

#[test]
fn changes_that_are_not_whole_triples_are_refused_by_name() {
    let src = "## Main\nLet half be [\"n\", \"0\"].\nCall noteTurn with \"bump\" and \"\" and half and \"\".\n";
    for (tier, run) in RUNNERS {
        let (error, journal) = journal_of(run, src);
        let error = error.unwrap_or_else(|| {
            panic!("{tier}: a half triple is refused, not journalled: {journal:?}")
        });
        assert!(
            error.contains("noteTurn") && error.contains("triples"),
            "{tier}: the refusal says why: {error}"
        );
        assert!(
            journal.is_empty(),
            "{tier}: nothing is journalled from a refused call: {journal:?}"
        );
    }
}

#[test]
fn the_one_argument_form_is_refused_by_name() {
    let src = "## Main\nCall noteTurn with \"bump\".\n";
    for (tier, run) in RUNNERS {
        let (error, journal) = journal_of(run, src);
        let error = error
            .unwrap_or_else(|| panic!("{tier}: a one-argument noteTurn is refused: {journal:?}"));
        assert!(
            error.contains("noteTurn"),
            "{tier}: the refusal names the builtin: {error}"
        );
        assert!(journal.is_empty(), "{tier}: {journal:?}");
    }
}

#[test]
fn every_in_process_host_keeps_the_journal_detail() {
    let src = "## Main\nLet detail be journalDetail().\nShow detail.\n";
    for (tier, run) in RUNNERS {
        dom::reset();
        let out = run(src);
        assert_eq!(out.error, None, "{tier}: the program runs");
        assert_eq!(
            out.output.trim(),
            "true",
            "{tier}: an in-process host records the journal, so a turn records its changes"
        );
    }
}
