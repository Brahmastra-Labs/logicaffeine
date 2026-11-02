use super::*;

#[test]
fn non_meta_lines_pass_through() {
    assert!(parse_meta("Show 1.").is_none());
    assert!(parse_meta("Every cat sleeps.").is_none());
}

#[test]
fn quit_aliases() {
    assert_eq!(parse_meta(":quit"), Some(Ok(MetaCommand::Quit)));
    assert_eq!(parse_meta(":q"), Some(Ok(MetaCommand::Quit)));
}

#[test]
fn format_parses_each_variant() {
    assert_eq!(
        parse_meta(":format latex"),
        Some(Ok(MetaCommand::Format(LogicFormat::Latex)))
    );
    assert_eq!(
        parse_meta(":format unicode"),
        Some(Ok(MetaCommand::Format(LogicFormat::Unicode)))
    );
    assert!(matches!(parse_meta(":format klingon"), Some(Err(_))));
    assert!(matches!(parse_meta(":format"), Some(Err(_))));
}

#[test]
fn mode_switching_forms() {
    assert_eq!(
        parse_meta(":logic"),
        Some(Ok(MetaCommand::ModeSet(Mode::Logic)))
    );
    assert_eq!(
        parse_meta(":mode imperative"),
        Some(Ok(MetaCommand::ModeSet(Mode::Imperative)))
    );
    assert_eq!(parse_meta(":mode"), Some(Ok(MetaCommand::ModeShow)));
}

#[test]
fn unknown_command_points_at_help() {
    match parse_meta(":frobnicate") {
        Some(Err(msg)) => assert!(msg.contains(":help"), "{msg}"),
        other => panic!("expected an error, got {other:?}"),
    }
}

#[test]
fn load_and_save_take_paths() {
    assert_eq!(
        parse_meta(":save out.lg"),
        Some(Ok(MetaCommand::Save(PathBuf::from("out.lg"))))
    );
    assert!(matches!(parse_meta(":load"), Some(Err(_))));
}

#[test]
fn load_and_save_accept_paths_with_spaces() {
    assert_eq!(
        parse_meta(":save /tmp/my session.lg"),
        Some(Ok(MetaCommand::Save(PathBuf::from("/tmp/my session.lg"))))
    );
    assert_eq!(
        parse_meta(":load notes about logic.lg"),
        Some(Ok(MetaCommand::Load(PathBuf::from("notes about logic.lg"))))
    );
}

#[test]
fn discourse_needs_on_or_off() {
    assert_eq!(
        parse_meta(":discourse on"),
        Some(Ok(MetaCommand::Discourse(true)))
    );
    assert_eq!(
        parse_meta(":discourse off"),
        Some(Ok(MetaCommand::Discourse(false)))
    );
    assert!(matches!(parse_meta(":discourse maybe"), Some(Err(_))));
}

#[test]
fn too_many_arguments_is_an_error() {
    assert!(matches!(parse_meta(":format latex extra"), Some(Err(_))));
}

#[test]
fn explain_takes_one_word() {
    assert_eq!(
        parse_meta(":explain give"),
        Some(Ok(MetaCommand::Explain("give".to_string())))
    );
    assert!(matches!(parse_meta(":explain"), Some(Err(_))));
    assert!(matches!(parse_meta(":explain give show"), Some(Err(_))));
}
