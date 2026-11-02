use super::*;

#[test]
fn bare_name_is_wildcard() {
    let (name, dep) = parse_spec("foo", None, None).unwrap();
    assert_eq!(name, "foo");
    assert_eq!(dep, DepValue::Simple("*".into()));
}

#[test]
fn at_version_splits() {
    let (name, dep) = parse_spec("foo@1.2", None, None).unwrap();
    assert_eq!(name, "foo");
    assert_eq!(dep, DepValue::Simple("1.2".into()));
}

#[test]
fn logos_uri_keeps_full_value() {
    let (name, dep) = parse_spec("logos:std", None, None).unwrap();
    assert_eq!(name, "std");
    assert_eq!(dep, DepValue::Simple("logos:std".into()));
}

#[test]
fn path_option_wins() {
    let (name, dep) = parse_spec("math", Some("./math".into()), None).unwrap();
    assert_eq!(name, "math");
    assert_eq!(dep, DepValue::Path("./math".into()));
}

#[test]
fn git_option_wins() {
    let (name, dep) = parse_spec("r", None, Some("https://x/r.git".into())).unwrap();
    assert_eq!(name, "r");
    assert_eq!(dep, DepValue::Git("https://x/r.git".into()));
}

#[test]
fn version_spec_with_path_is_usage_error() {
    let err = parse_spec("foo@1.2", Some("./x".into()), None).unwrap_err();
    assert_eq!(err.exit_code, ui::EXIT_USAGE);
}

#[test]
fn empty_version_is_malformed() {
    assert!(parse_spec("foo@", None, None).is_err());
    assert!(parse_spec("@1.0", None, None).is_err());
}

#[test]
fn logos_uri_with_version_is_malformed() {
    let err = parse_spec("logos:std@1.0", None, None).unwrap_err();
    assert_eq!(err.exit_code, ui::EXIT_USAGE);
}

#[test]
fn double_at_is_malformed() {
    assert!(parse_spec("foo@1.0@2.0", None, None).is_err());
}
