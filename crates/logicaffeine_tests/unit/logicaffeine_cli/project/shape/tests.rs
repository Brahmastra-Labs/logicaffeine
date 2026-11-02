use super::*;

fn serves(src: &str) -> bool {
    program_serves(src).unwrap_or(false)
}

#[test]
fn a_top_level_serve_statement_is_found() {
    let src = "# S\n\n\
             ## To handle (request: Request) -> Response:\n    Return plain(\"hi\").\n\n\
             ## Main\nServe on port 8080 with handle.\n";
    assert!(serves(src), "the classic web program carries Serve");
}

#[test]
fn a_serve_nested_in_control_flow_is_found() {
    let src = "# S\n\n\
             ## To handle (request: Request) -> Response:\n    Return plain(\"hi\").\n\n\
             ## Main\nLet dev be true.\nIf dev:\n    Serve on port 8080 with handle.\n";
    assert!(serves(src), "Serve inside an If still classifies");
}

#[test]
fn a_serve_inside_a_function_is_found() {
    let src = "# S\n\n\
             ## To handle (request: Request) -> Response:\n    Return plain(\"hi\").\n\n\
             ## To boot ():\n    Serve on port 8080 with handle.\n\n\
             ## Main\nCall boot.\n";
    assert!(
        serves(src),
        "Serve inside a called function still classifies"
    );
}

#[test]
fn a_route_program_serves() {
    let src =
        "# S\n\n## Route GET / -> Text:\n    Return \"hi\".\n\n## Main\n\nServe on port 8080.\n";
    assert!(
        serves(src),
        "`Serve on port N.` over `## Route`s is a backend"
    );
}

#[test]
fn dom_and_plain_programs_have_no_serve() {
    assert!(!serves("## Main\nRender:\n    h1: \"hi\"\n"), "a DOM app");
    assert!(
        !serves("# M\n\n## Main\n\nShow \"Hello\".\n"),
        "a plain program"
    );
}

#[test]
fn the_word_serve_in_a_string_does_not_misroute() {
    let src = "# M\n\n## Main\n\nShow \"Serve on port 8080 with handle.\".\n";
    assert!(!serves(src), "an AST fact, not a substring guess");
}

#[test]
fn an_unparseable_program_has_no_answer() {
    assert_eq!(
        program_serves("## Main\nqqq zzz.\n"),
        None,
        "a parse failure is not a `false`"
    );
}

fn lexes_serve(src: &str) -> bool {
    source_lexes_a_serve_statement(src)
}

#[test]
fn the_lexer_fallback_finds_a_top_level_serve_statement() {
    let src = "# S\n\n\
             ## To handle (request: Request) -> Response:\n    Return plain(\"hi\").\n\n\
             ## Main\nServe on port 8080 with handle.\n";
    assert!(lexes_serve(src));
}

#[test]
fn the_lexer_fallback_finds_a_serve_the_parser_never_reaches() {
    // A genuine syntax error EARLIER in the file stops the parser before it would ever see
    // the `Serve` below — the lexical fallback must still find it: a lexer classifies words,
    // it does not validate sentences.
    let src = "## Main\nqqq zzz.\nServe on port 8080.\n";
    assert!(lexes_serve(src));
}

#[test]
fn the_lexer_fallback_does_not_misroute_on_a_string() {
    let src = "# M\n\n## Main\n\nShow \"Serve on port 8080 with handle.\".\n";
    assert!(
        !lexes_serve(src),
        "a quoted string is one token, never a bare `Serve`"
    );
}

#[test]
fn the_lexer_fallback_does_not_misroute_on_a_note() {
    // `## Note` is documentation, not code — the parser never turns its prose into
    // statements at all, so a bare `Serve` there must not read as one either.
    let src = "## Note\nServe is a word this program never calls as a statement.\n\n\
                   ## Main\nShow \"hi\".\n";
    assert!(
        !lexes_serve(src),
        "a `## Note` block is prose, not a bare `Serve` statement"
    );
}

#[test]
fn the_lexer_fallback_resumes_after_a_note_ends() {
    // The Note-exclusion span must close at the NEXT header — a real `Serve` after the Note
    // must still be found.
    let src = "## Note\nServe is mentioned here, not called.\n\n\
                   ## Main\nServe on port 8080.\n";
    assert!(
        lexes_serve(src),
        "a real Serve AFTER the Note must still be found"
    );
}

#[test]
fn the_lexer_fallback_says_no_for_a_plain_program() {
    assert!(!lexes_serve("# M\n\n## Main\n\nShow \"Hello\".\n"));
}

#[test]
fn shape_or_best_effort_matches_shape_when_the_entry_parses() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path(), "[package]\nname = \"p\"\n", &["src/main.lg"]);
    let p = Project::load(dir.path()).unwrap();
    assert_eq!(p.shape().unwrap(), p.shape_or_best_effort());
}

#[test]
fn shape_or_best_effort_never_fails_on_a_broken_entry() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Largo.toml"), "[package]\nname = \"p\"\n").unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.lg"), "## Main\nqqq zzz.\n").unwrap();
    let p = Project::load(dir.path()).unwrap();
    assert!(p.shape().is_err(), "the strict form still refuses");
    assert_eq!(
        p.shape_or_best_effort(),
        ProjectShape::Program,
        "no site, no lexable Serve — a program"
    );
}

#[test]
fn shape_or_best_effort_finds_serve_through_a_syntax_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("Largo.toml"), "[package]\nname = \"p\"\n").unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/main.lg"),
        "## To handle (request: Request) -> Response:\n    qqq zzz.\n\n\
             ## Main\nServe on port 8080 with handle.\n",
    )
    .unwrap();
    let p = Project::load(dir.path()).unwrap();
    assert!(
        p.shape().is_err(),
        "the strict form still refuses — the body does not parse"
    );
    assert_eq!(
        p.shape_or_best_effort(),
        ProjectShape::Backend,
        "the lexical `Serve` still routes it as a backend"
    );
}

#[test]
fn two_facts_decide_four_shapes() {
    assert_eq!(ProjectShape::from_facts(true, false), ProjectShape::Site);
    assert_eq!(
        ProjectShape::from_facts(true, true),
        ProjectShape::FullStack
    );
    assert_eq!(ProjectShape::from_facts(false, true), ProjectShape::Backend);
    assert_eq!(
        ProjectShape::from_facts(false, false),
        ProjectShape::Program
    );
}

fn project(dir: &Path, manifest: &str, files: &[&str]) {
    std::fs::write(dir.join("Largo.toml"), manifest).unwrap();
    for f in files {
        let p = dir.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "## Main\nShow 1.\n").unwrap();
    }
}

#[test]
fn a_site_may_leave_out_the_default_entry() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path(), "[package]\nname = \"s\"\n", &["pages/app.lg"]);
    let p = Project::load(dir.path()).expect("a site with no entry loads");
    assert_eq!(p.entry, None);
    assert_eq!(p.shape().unwrap(), ProjectShape::Site);
    let err = p
        .require_entry()
        .expect_err("a command that runs a program is refused");
    assert!(err.message.contains("is a site"), "{}", err.message);
}

#[test]
fn a_declared_entry_must_exist_even_beside_pages() {
    let dir = tempfile::tempdir().unwrap();
    project(
        dir.path(),
        "[package]\nname = \"s\"\nentry = \"src/server.lg\"\n",
        &["pages/app.lg"],
    );
    let err = Project::load(dir.path()).expect_err("a misspelled backend is not a site");
    assert!(
        err.message.contains("entry point not found"),
        "{}",
        err.message
    );
}

#[test]
fn a_program_without_its_entry_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path(), "[package]\nname = \"p\"\n", &[]);
    let err = Project::load(dir.path()).expect_err("no site, no entry: nothing to be");
    assert!(
        err.message.contains("entry point not found"),
        "{}",
        err.message
    );
}

#[test]
fn an_entry_escaping_the_project_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    for bad in ["../outside.lg", "src/../../x.lg", "/etc/hostname"] {
        project(
            dir.path(),
            &format!("[package]\nname = \"p\"\nentry = \"{bad}\"\n"),
            &[],
        );
        let err = Project::load(dir.path()).expect_err(&format!("{bad} must be rejected"));
        assert!(err.message.contains("escapes"), "{bad}: {}", err.message);
    }
}
