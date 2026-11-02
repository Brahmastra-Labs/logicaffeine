use super::*;

#[test]
fn header_imports_are_read_through_comments_and_stop_at_the_first_command() {
    let source = "/- a module\n  comment -/\n-- line\nprelude\nimport Init.Core\n\nimport  Demo.Defs -- trailing\nimport «Odd Name».X\nnamespace Demo\nimport NotAnImport\n";
    assert_eq!(
        header_imports(source),
        vec![
            ("Init.Core".to_string(), 5),
            ("Demo.Defs".to_string(), 7),
            ("«Odd Name».X".to_string(), 8)
        ]
    );
    assert!(header_imports("theorem t : True := trivial\nimport X\n").is_empty());
    assert_eq!(
        header_imports("import Missing.Thing\n"),
        vec![("Missing.Thing".to_string(), 1)]
    );
    // A module doc of non-ASCII prose (the corpus cases open with `/-! … · … -/`).
    assert_eq!(
        header_imports("/-! Tier 6 · Propositions — `Eq.rec`\n-/\nimport Init.Core\n"),
        vec![("Init.Core".to_string(), 3)]
    );
}

#[test]
fn print_axioms_output_is_parsed_including_wrapped_lists() {
    assert_eq!(
        parse_print_axioms("'Demo.x' does not depend on any axioms\n"),
        Some(vec![])
    );
    assert_eq!(
        parse_print_axioms(
            "'Toy.em'' depends on axioms: [propext, Classical.choice, Quot.sound]\n"
        ),
        Some(vec![
            "propext".into(),
            "Classical.choice".into(),
            "Quot.sound".into()
        ])
    );
    assert_eq!(
        parse_print_axioms(
            "'x' depends on axioms: [propext,\n Classical.choice,\n «weird, name»]\n"
        ),
        Some(vec![
            "propext".into(),
            "Classical.choice".into(),
            "«weird, name»".into()
        ])
    );
    assert_eq!(parse_print_axioms("error: unknown constant"), None);
}

#[test]
fn lean_errors_keep_their_location_and_continuation() {
    let output = "Broken.lean:3:8: warning: declaration uses 'sorry'\nBroken.lean:5:17: error: unexpected token ':='; expected term\n  more context\nBroken.lean:7:0: error: second\n";
    assert_eq!(
        lean_errors(output),
        vec![
            (
                5,
                "Broken.lean:5:17: error: unexpected token ':='; expected term\n  more context"
                    .to_string()
            ),
            (7, "Broken.lean:7:0: error: second".to_string()),
        ]
    );
}
