//! The export as LOGOS reads it, for a checker that is to decide the proof the LOGOS kernel decided: each native
//! decision's leaf replaced by its re-derivation, every other line byte for byte as exported.
use super::*;
use crate::dag::{arena, tc::Engine, Verdict as CoreVerdict};

const NATIVE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../logicaffeine_tests/tests/fixtures/lean_conformance/cases_lean414/native_decide/T08-014.viaNative.ndjson"
));

#[test]
fn an_export_without_a_native_decision_is_read_as_exported() {
    let plain = "{\"meta\":{}}\n{\"in\":1,\"str\":{\"pre\":0,\"str\":\"Nat\"}}\n";
    let natives = NativeDecisions::read(plain);
    assert!(natives.is_empty());
    match natives.rederived_export(plain) {
        std::borrow::Cow::Borrowed(read) => assert_eq!(read, plain),
        std::borrow::Cow::Owned(_) => panic!("an export without a leaf is not copied"),
    }
}

#[test]
fn only_each_leaf_line_is_replaced_by_its_rederivation() {
    let natives = NativeDecisions::read(NATIVE);
    assert!(!natives.is_empty());
    let read = natives.rederived_export(NATIVE);
    let exported: Vec<&str> = NATIVE.split_inclusive('\n').collect();
    let lines: Vec<&str> = read.split_inclusive('\n').collect();
    assert_eq!(exported.len(), lines.len());
    let mut replaced = 0;
    for (was, is) in exported.iter().zip(&lines) {
        if was == is {
            continue;
        }
        replaced += 1;
        let record: Value = serde_json::from_str(was.trim_end()).unwrap();
        assert_eq!(is.trim_end_matches('\n'), natives.rederive(record).to_string());
        assert_eq!(was.ends_with('\n'), is.ends_with('\n'));
    }
    assert_eq!(replaced, natives.decisions().count());
}

#[test]
fn the_kernel_core_accepts_the_export_as_logos_reads_it_and_not_as_exported() {
    let natives = NativeDecisions::read(NATIVE);
    let read = natives.rederived_export(NATIVE);
    match arena::verdict(&read, Engine::Sem) {
        CoreVerdict::Accept => {}
        CoreVerdict::Reject(e) | CoreVerdict::Decline(e) => {
            panic!("the core decides the re-derived proof: {e}")
        }
    }
    match arena::verdict(NATIVE, Engine::Sem) {
        CoreVerdict::Reject(_) => {}
        CoreVerdict::Accept => panic!("as exported, the proof rests on Lean's compiled step, which the core does not run"),
        CoreVerdict::Decline(e) => panic!("as exported, the core refuses Lean's compiled step, not declines it: {e}"),
    }
}
