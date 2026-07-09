//! The rustc flycheck substrate: mapped codegen ties every generated Rust
//! line back to the LOGOS statement that produced it, variable origins carry
//! ownership roles, and the diagnostic bridge translates real rustc JSON
//! onto USER-source spans — "English with a borrow checker".

use logicaffeine_compile::compile::{rustc_check, rustc_check_artifacts, CheckArtifacts};
use logicaffeine_compile::diagnostic::{
    translate_diagnostics_all, RustcCode, RustcDiagnostic, RustcSpan,
};
use logicaffeine_compile::sourcemap::OwnershipRole;

// NOTE: the recipient must be a real sink — rustc itself caught an earlier
// fixture that gave to an Int (`Give a to b.` generates a call `b(a)`), which
// the parser and local checkers all accepted. The flycheck exists for exactly
// that class of finding.
const GIVE_PROGRAM: &str = "\
## To consume (v: Int):
    Show v.

## Main
Let a be 5.
Let c be 1.
Give a to consume.
Show c.
";

fn artifacts(source: &str) -> CheckArtifacts {
    rustc_check_artifacts(source).expect("program must compile to artifacts")
}

/// The 1-based rust line whose mapped LOGOS span text starts with `prefix`.
fn rust_line_mapped_to(art: &CheckArtifacts, source: &str, prefix: &str) -> u32 {
    art.source_map
        .line_span_entries()
        .into_iter()
        .find(|(_, span)| source[span.start..span.end].starts_with(prefix))
        .map(|(line, _)| line)
        .unwrap_or_else(|| {
            panic!(
                "no generated line maps to a statement starting {prefix:?}; map: {:?}",
                art.source_map.line_span_entries()
            )
        })
}

#[test]
fn mapped_lines_point_at_their_logos_statements() {
    let art = artifacts(GIVE_PROGRAM);

    // Every mapped span lies inside the USER source (never the prelude).
    for (line, span) in art.source_map.line_span_entries() {
        assert!(
            span.end <= GIVE_PROGRAM.len() && span.start < span.end,
            "line {line} maps outside the user source: {span:?}"
        );
    }

    // Each statement's generated code maps back to that statement — the
    // function definition included (its whole body maps to the `## To` span).
    for prefix in ["## To consume", "Let a", "Let c", "Give a", "Show c"] {
        rust_line_mapped_to(&art, GIVE_PROGRAM, prefix);
    }

    // The binding line for `a` maps to `Let a be 5.` specifically.
    let let_a_line = rust_line_mapped_to(&art, GIVE_PROGRAM, "Let a");
    let rust_line = art
        .rust_code
        .lines()
        .nth(let_a_line as usize - 1)
        .expect("mapped line exists in the generated code");
    assert!(
        rust_line.contains('a'),
        "the line mapped to `Let a` should mention a: {rust_line:?}"
    );
}

#[test]
fn var_origins_carry_move_roles() {
    let art = artifacts(GIVE_PROGRAM);

    let a = art
        .source_map
        .get_var_origin("a")
        .expect("a is tracked");
    assert_eq!(a.role, OwnershipRole::GiveObject, "a is what got given away");
    assert!(
        GIVE_PROGRAM[a.span.start..a.span.end].starts_with("Give a"),
        "a's origin span is the Give statement: {:?}",
        &GIVE_PROGRAM[a.span.start..a.span.end]
    );

    let recipient = art
        .source_map
        .get_var_origin("consume")
        .expect("the sink is tracked");
    assert_eq!(recipient.role, OwnershipRole::GiveRecipient);

    let c = art.source_map.get_var_origin("c").expect("c is tracked");
    assert_eq!(c.role, OwnershipRole::ShowObject);
}

#[test]
fn zone_locals_are_tracked_as_zone_locals() {
    let source = "\
## Main
Inside a zone called \"Scratch\":
    Let t be 1.
    Show t.
Show 2.
";
    let art = artifacts(source);
    let t = art.source_map.get_var_origin("t").expect("t is tracked");
    assert_eq!(t.role, OwnershipRole::ZoneLocal, "zone-allocated Lets are ZoneLocal");
}

#[test]
fn bridge_translates_e0382_onto_the_give_statement() {
    let art = artifacts(GIVE_PROGRAM);
    let give_line = rust_line_mapped_to(&art, GIVE_PROGRAM, "Give a");

    // A real-shape rustc diagnostic pointing at the generated Give line.
    let diag = RustcDiagnostic {
        message: "use of moved value: `a`".to_string(),
        code: Some(RustcCode { code: "E0382".to_string() }),
        level: "error".to_string(),
        spans: vec![RustcSpan {
            file_name: "src/main.rs".to_string(),
            line_start: give_line,
            line_end: give_line,
            column_start: 5,
            column_end: 6,
            is_primary: true,
            label: None,
            text: vec![],
        }],
        children: vec![],
    };

    let errors = translate_diagnostics_all(&[diag], &art.source_map, &art.interner);
    assert_eq!(errors.len(), 1, "the E0382 must translate");
    let error = &errors[0];
    assert!(
        error.title.contains("giving it away"),
        "speaks English: {}",
        error.title
    );
    let span = error.logos_span.expect("a REAL sourcemap yields a LOGOS span");
    assert!(
        GIVE_PROGRAM[span.start..span.end].contains("Give a"),
        "the span lands on the Give statement, got: {:?}",
        &GIVE_PROGRAM[span.start..span.end]
    );
}

#[test]
fn spans_stay_in_user_source_even_when_the_prelude_fires() {
    // `flush` drags in the stdlib prelude, which is PREPENDED to the source;
    // mapped spans must still index the USER'S buffer, never the prelude.
    let source = "\
## Main
Let x be 5.
Show x.
Call flush.
";
    let art = artifacts(source);
    assert!(
        !art.source_map.line_span_entries().is_empty(),
        "user statements must map"
    );
    for (line, span) in art.source_map.line_span_entries() {
        assert!(
            span.end <= source.len(),
            "line {line} maps into the prepended prelude: {span:?}"
        );
    }
}

/// The full loop against real cargo: a clean program checks clean.
/// (Spawns `cargo check`; warm second run proves cache-dir reuse.)
#[test]
fn rustc_check_reports_nothing_for_a_clean_program() {
    let dir = std::env::temp_dir().join("logos_flycheck_test_clean");
    let _ = std::fs::remove_dir_all(&dir);

    let findings = rustc_check(GIVE_PROGRAM, &dir).expect("cargo check must run");
    assert!(
        findings.is_empty(),
        "a clean program has no rustc findings: {findings:#?}"
    );

    // Second run over the same cache dir must also work (incremental reuse).
    let findings = rustc_check(GIVE_PROGRAM, &dir).expect("warm cargo check must run");
    assert!(findings.is_empty());
}
