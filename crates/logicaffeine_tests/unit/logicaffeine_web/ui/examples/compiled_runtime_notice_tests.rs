//! A registry example that cannot run in the browser opens to a notice, not to an
//! interpreter run that pretends to listen or fails on a feature it never had.
use super::*;

fn native_only() -> impl Iterator<Item = &'static ExampleSpec> {
    ALL_CODE_EXAMPLES
        .iter()
        .filter(|spec| matches!(spec.expected, Expected::NativeOnlyCompiles))
}

#[test]
fn every_native_only_example_says_it_needs_the_compiled_runtime() {
    assert_eq!(native_only().count(), 2, "the networking server and client");
    for spec in native_only() {
        let notice = compiled_runtime_notice(spec.vfs_path)
            .unwrap_or_else(|| panic!("{} opens with no notice", spec.vfs_path));
        assert!(notice.contains("compiled runtime"), "{}: {notice}", spec.vfs_path);
        assert!(notice.contains("`largo build`"), "{}: {notice}", spec.vfs_path);
        assert!(notice.contains("Compile"), "{} must point at the Rust view: {notice}", spec.vfs_path);
    }
}

#[test]
fn each_notice_names_what_the_example_needs() {
    let cases = [
        ("/examples/code/networking/server.logos", "networking"),
        ("/examples/code/networking/client.logos", "networking"),
    ];
    for (path, need) in cases {
        let notice = compiled_runtime_notice(path).unwrap_or_else(|| panic!("{path} opens with no notice"));
        assert!(notice.contains(need), "{path} should name {need:?}: {notice}");
    }
}

#[test]
fn a_notice_leads_with_what_it_needs() {
    for spec in native_only() {
        let needs = compiled_runtime_needs(spec.vfs_path).expect("a native-only example says what it needs");
        let notice = compiled_runtime_notice(spec.vfs_path).expect("a native-only example has a notice");
        assert!(notice.starts_with(needs), "{}: {notice}", spec.vfs_path);
        assert!(needs.chars().next().is_some_and(char::is_uppercase), "{}: {needs:?} starts a sentence", spec.vfs_path);
    }
}

#[test]
fn an_example_that_runs_in_the_browser_opens_with_no_notice() {
    let quiet: Vec<&str> = ALL_LOGIC_EXAMPLES
        .iter()
        .chain(ALL_CODE_EXAMPLES)
        .chain(ALL_MATH_EXAMPLES)
        .chain(ALL_HARDWARE_SPECS)
        .filter(|spec| !matches!(spec.expected, Expected::NativeOnlyCompiles))
        .filter(|spec| compiled_runtime_notice(spec.vfs_path).is_some())
        .map(|spec| spec.vfs_path)
        .collect();
    assert!(quiet.is_empty(), "these run in the browser yet open with a notice: {quiet:?}");
}

#[test]
fn a_file_outside_the_registry_opens_with_no_notice() {
    for path in [
        "/scratch.logos",
        "/examples/code/networking/relay.logos",
        "examples/code/networking/server.logos",
        "/examples/code/networking/server.logos.bak",
        "",
    ] {
        assert_eq!(compiled_runtime_notice(path), None, "{path:?}");
    }
}

#[test]
fn every_row_of_the_needs_table_is_a_native_only_example() {
    for (at, (path, _)) in COMPILED_RUNTIME_NEEDS.iter().enumerate() {
        assert!(compiled_runtime_needs(path).is_some(), "{path} is not a native-only program in the registry");
        assert!(
            COMPILED_RUNTIME_NEEDS[..at].iter().all(|(earlier, _)| earlier != path),
            "{path} has two rows"
        );
    }
}
