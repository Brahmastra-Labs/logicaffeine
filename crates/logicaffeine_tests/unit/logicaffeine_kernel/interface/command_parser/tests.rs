use super::*;

#[test]
fn test_parse_definition_with_type() {
    let cmd = parse_command("Definition one : Nat := Succ Zero.").unwrap();
    if let Command::Definition { name, ty, body, .. } = cmd {
        assert_eq!(name, "one");
        assert!(ty.is_some());
        assert!(matches!(body, Term::App(..)));
    } else {
        panic!("Expected Definition");
    }
}

#[test]
fn test_parse_definition_without_type() {
    let cmd = parse_command("Definition two := Succ (Succ Zero).").unwrap();
    if let Command::Definition { name, ty, .. } = cmd {
        assert_eq!(name, "two");
        assert!(ty.is_none());
    } else {
        panic!("Expected Definition");
    }
}

#[test]
fn test_parse_check() {
    let cmd = parse_command("Check Zero.").unwrap();
    assert!(matches!(cmd, Command::Check(_)));
}

#[test]
fn test_parse_eval() {
    let cmd = parse_command("Eval (Succ Zero).").unwrap();
    assert!(matches!(cmd, Command::Eval(_)));
}

#[test]
fn test_parse_inductive() {
    let cmd = parse_command("Inductive Bool := True : Bool | False : Bool.").unwrap();
    if let Command::Inductive {
        name, constructors, ..
    } = cmd
    {
        assert_eq!(name, "Bool");
        assert_eq!(constructors.len(), 2);
        assert_eq!(constructors[0].0, "True");
        assert_eq!(constructors[1].0, "False");
    } else {
        panic!("Expected Inductive");
    }
}
