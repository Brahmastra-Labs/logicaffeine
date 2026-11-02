//! A deliberately unsafe design is a demonstration: it says so, and points to its fix.
use super::*;

fn planted() -> impl Iterator<Item = (&'static ExampleSpec, &'static str)> {
    ALL_HARDWARE_SPECS
        .iter()
        .filter(|spec| matches!(spec.expected, Expected::RefutesWithCounterexample | Expected::SpillsRequired))
        .map(|spec| {
            let fixed = planted_bug_fix(spec.vfs_path).unwrap_or_else(|| panic!("{} names no fix", spec.vfs_path));
            (spec, fixed)
        })
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[test]
fn every_planted_bug_names_a_fixed_twin_that_holds() {
    assert_eq!(planted().count(), 5, "four refuted designs and one spilling allocation");
    for (spec, fixed) in planted() {
        let twin = ALL_HARDWARE_SPECS
            .iter()
            .find(|s| s.vfs_path == fixed)
            .unwrap_or_else(|| panic!("{} names {fixed}, which the registry does not ship", spec.vfs_path));
        let holds = match (&spec.expected, &twin.expected) {
            (Expected::RefutesWithCounterexample, Expected::RtlProven) => true,
            (Expected::SpillsRequired, Expected::RegisterAllocFits) => true,
            _ => false,
        };
        assert!(holds, "{}'s twin {fixed} does not hold", spec.vfs_path);
    }
}

#[test]
fn a_planted_bug_says_it_is_planted_and_names_its_fix() {
    for (spec, fixed) in planted() {
        let notice = planted_bug_notice(spec.vfs_path).unwrap_or_else(|| panic!("{} opens with no notice", spec.vfs_path));
        assert!(notice.contains("planted"), "{}: {notice}", spec.vfs_path);
        assert!(notice.contains(file_name(fixed)), "{} should name {}: {notice}", spec.vfs_path, file_name(fixed));
    }
}

#[test]
fn an_example_without_a_planted_bug_has_no_notice() {
    let loud: Vec<&str> = ALL_LOGIC_EXAMPLES
        .iter()
        .chain(ALL_CODE_EXAMPLES)
        .chain(ALL_MATH_EXAMPLES)
        .chain(ALL_HARDWARE_SPECS)
        .filter(|spec| !matches!(spec.expected, Expected::RefutesWithCounterexample | Expected::SpillsRequired))
        .filter(|spec| planted_bug_notice(spec.vfs_path).is_some())
        .map(|spec| spec.vfs_path)
        .collect();
    assert!(loud.is_empty(), "{loud:?}");
    assert_eq!(planted_bug_notice("/examples/hardware/queue-jam.v.bak"), None);
}

#[test]
fn every_row_of_the_fix_table_is_a_planted_bug() {
    for (at, (path, _)) in PLANTED_BUG_FIXES.iter().enumerate() {
        assert!(planted_bug_fix(path).is_some(), "{path} is not a planted bug in the registry");
        assert!(PLANTED_BUG_FIXES[..at].iter().all(|(earlier, _)| earlier != path), "{path} has two rows");
    }
}
