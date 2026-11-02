use super::*;

#[test]
fn cli_error_defaults_to_failure_exit_code() {
    let e = CliError::new("boom");
    assert_eq!(e.exit_code, EXIT_FAILURE);
    assert!(e.hint.is_none());
    assert_eq!(e.to_string(), "boom");
}

#[test]
fn cli_error_carries_hint_and_custom_code() {
    let e = CliError::with_hint("boom", "try --fix").exit_code(EXIT_USAGE);
    assert_eq!(e.hint.as_deref(), Some("try --fix"));
    assert_eq!(e.exit_code, EXIT_USAGE);
}

#[test]
fn render_error_uses_cli_error_exit_code() {
    let boxed: Box<dyn std::error::Error> = Box::new(CliError::new("x").exit_code(EXIT_USAGE));
    assert_eq!(render_error(boxed.as_ref()), EXIT_USAGE);
}

#[test]
fn render_error_defaults_other_errors_to_failure() {
    let boxed: Box<dyn std::error::Error> = "plain".into();
    assert_eq!(render_error(boxed.as_ref()), EXIT_FAILURE);
}
