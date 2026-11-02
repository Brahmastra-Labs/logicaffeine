use logicaffeine_kernel::elaborate::ParamKind;
use logicaffeine_kernel::interface::Repl;

#[test]
fn inferred_definition_preserves_interleaved_lambda_binders() {
    let mut repl = Repl::new();
    repl.execute("Definition inferred := fun n : Nat => fun {A : Type} => fun a : A => a.")
        .unwrap();
    assert_eq!(
        repl.context().binder_kinds("inferred"),
        Some(
            &[
                ParamKind::Explicit,
                ParamKind::Implicit,
                ParamKind::Explicit
            ][..]
        )
    );
    assert_eq!(
        repl.execute("Eval inferred Zero (Succ Zero).").unwrap(),
        "(Succ Zero)"
    );
}

#[test]
fn inferred_definition_preserves_strict_implicit_binders() {
    let mut repl = Repl::new();
    repl.execute("Definition inferred := fun {{A : Type}} => fun a : A => a.")
        .unwrap();
    assert_eq!(
        repl.context().binder_kinds("inferred"),
        Some(&[ParamKind::StrictImplicit, ParamKind::Explicit][..])
    );
    assert!(repl.execute("Check inferred.").is_ok());
    assert_eq!(repl.execute("Eval inferred Zero.").unwrap(), "Zero");
}

#[test]
fn redeclaring_an_explicit_function_removes_old_implicit_metadata() {
    let mut repl = Repl::new();
    repl.execute("Definition identity : {A : Type} -> A -> A := fun A : Type => fun a : A => a.")
        .unwrap();
    repl.execute("Definition identity : Nat -> Nat := fun n : Nat => n.")
        .unwrap();
    assert_eq!(repl.context().binder_kinds("identity"), None);
    assert_eq!(repl.context().implicit_args("identity"), 0);
    assert_eq!(repl.execute("Eval identity Zero.").unwrap(), "Zero");
}
