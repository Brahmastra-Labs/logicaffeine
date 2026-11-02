//! Regression: policy **capabilities** (`A User can <verb> the <Object> if …`) must be
//! checkable in the interpreter, not just the compiled path.
//!
//! The capability action is a verb ("edit"). The `Check` site resolves the verb by its
//! *lemma* (`parser`), while the discovery pass registered it by its *lexeme*
//! (`consume_noun_or_proper` returns the lexeme for a `Verb` token). The two symbols never
//! matched, so EVERY capability check died with
//! `No capability 'Edit' defined for type 'User'` — even though codegen (which lowercases
//! the action on both sides) handled it fine. Predicate checks ("is admin") were unaffected
//! because both sides use the same noun/adjective symbol.
//!
//! These pin the behaviour end-to-end: a capability must be (a) found, (b) granted when the
//! condition holds (via either disjunct), and (c) DENIED when it does not — proving the
//! condition is actually evaluated, not rubber-stamped.

use logicaffeine_compile::interpret_for_ui_sync;

const POLICY: &str = r#"## Definition
A User has:
    a name: Text.
    a role: Text.

A Document has:
    an owner: Text.

## Policy
A User is admin if the user's role equals "admin".
A User can edit the Document if:
    The user is admin, OR
    The user's name equals the document's owner.

## Main
"#;

fn run(main_body: &str) -> (Vec<String>, Option<String>) {
    let src = format!("{POLICY}{main_body}");
    let r = interpret_for_ui_sync(&src);
    (r.lines, r.error)
}

#[test]
fn capability_granted_via_admin_disjunct() {
    let (lines, err) = run(
        r#"Let alice be a new User with name "Alice" and role "admin".
Let doc be a new Document with owner "Bob".
Check that alice can edit doc.
Show "edit permitted"."#,
    );
    assert_eq!(err, None, "admin should be granted edit; got error");
    assert_eq!(lines, vec!["edit permitted".to_string()]);
}

#[test]
fn capability_granted_via_owner_disjunct() {
    // Not admin, but owns the document → second disjunct grants it.
    let (lines, err) = run(
        r#"Let alice be a new User with name "Alice" and role "editor".
Let doc be a new Document with owner "Alice".
Check that alice can edit doc.
Show "edit permitted"."#,
    );
    assert_eq!(err, None, "owner should be granted edit; got error");
    assert_eq!(lines, vec!["edit permitted".to_string()]);
}

#[test]
fn capability_denied_when_neither_disjunct_holds() {
    // Neither admin nor owner → the Check must FAIL (proving the condition is evaluated).
    let (_lines, err) = run(
        r#"Let mallory be a new User with name "Mallory" and role "guest".
Let doc be a new Document with owner "Alice".
Check that mallory can edit doc.
Show "edit permitted"."#,
    );
    let err = err.expect("a guest who is neither admin nor owner must be DENIED");
    assert!(
        err.contains("Security Check Failed"),
        "expected a security-check failure, got: {err}"
    );
}

#[test]
fn single_line_capability_also_checkable() {
    let src = r#"## Definition
A User has:
    a role: Text.

A Document has:
    an owner: Text.

## Policy
A User is admin if the user's role equals "admin".
A User can edit the Document if the user is admin.

## Main
Let u be a new User with role "admin".
Let d be a new Document with owner "x".
Check that u can edit d.
Show "ok"."#;
    let r = interpret_for_ui_sync(src);
    assert_eq!(r.error, None, "single-line capability check should pass");
    assert_eq!(r.lines, vec!["ok".to_string()]);
}
