use super::*;
use crate::document::DocumentState;
use crate::workspace::ModuleExport;
use lsp_types::Url;

fn make_doc(source: &str) -> DocumentState {
    DocumentState::new(source.to_string(), 1)
}

struct FakeWs {
    exports: Vec<(&'static str, &'static str)>,
}
impl WorkspaceView for FakeWs {
    fn module_exporting(&self, name: &str) -> Vec<ModuleExport> {
        self.exports
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, file)| ModuleExport {
                module_url: Url::parse(&format!("file:///proj/{file}")).unwrap(),
                module_name: "M".to_string(),
            })
            .collect()
    }
    fn imports_of(&self, _uri: &Url) -> Vec<String> {
        Vec::new()
    }
    fn module_names(&self) -> Vec<(String, Url)> {
        Vec::new()
    }
}

/// Editors show a word's problems in their own section of the hover (VS Code's Problems,
/// the Studio's squiggle hover), so the language server's hover leaves them out rather
/// than saying each one twice.
#[test]
fn a_hover_leaves_problems_to_the_editor() {
    let doc = make_doc("## Main\nLet x be 5.\n");
    assert!(
        !doc.diagnostics.is_empty(),
        "an unused binding has a diagnostic to leave out"
    );
    let off = doc.source.find("x be").unwrap();
    let shown = hover(&doc, doc.line_index.position(off)).expect("x has a hover");
    let HoverContents::Markup(markup) = shown.contents else {
        panic!("hover is markdown")
    };
    assert!(
        !markup.value.contains("Diagnostic"),
        "the hover repeats the problem: {}",
        markup.value
    );
    assert!(
        !markup.value.contains("never used"),
        "the hover repeats the problem: {}",
        markup.value
    );
}

#[test]
fn hover_names_the_defining_module() {
    // `double` is not defined locally and not stdlib; the workspace says it
    // lives in `util.lg`.
    let doc = make_doc("# App\n\n## Main\nShow double(21).\n");
    let ws = FakeWs {
        exports: vec![("double", "util.lg")],
    };
    let off = doc.source.find("double").unwrap() + 1;
    let pos = doc.line_index.position(off);
    let attr = module_attribution(&doc, pos, &ws).expect("cross-file name is attributed");
    assert!(
        attr.contains("util.lg"),
        "attribution names the file: {attr}"
    );
}

#[test]
fn locally_defined_name_has_no_module_attribution() {
    // `helper` is defined in this very file → no cross-file attribution.
    let doc = make_doc(
        "# App\n\n## To helper (n: Int) -> Int:\n    Return n.\n\n## Main\nShow helper(1).\n",
    );
    let ws = FakeWs {
        exports: vec![("helper", "other.lg")],
    };
    let off = doc.source.rfind("helper").unwrap() + 1;
    let pos = doc.line_index.position(off);
    assert!(
        module_attribution(&doc, pos, &ws).is_none(),
        "a locally-defined name is not attributed elsewhere"
    );
}

#[test]
fn hover_on_keyword_shows_docs() {
    let doc = make_doc("## Main\n    Let x be 5.\n");
    // Position on "Let" (line 1, character 4)
    let pos = Position {
        line: 1,
        character: 4,
    };
    let result = hover(&doc, pos);
    assert!(result.is_some(), "Expected hover info for 'Let'");
    let h = result.unwrap();
    match &h.contents {
        HoverContents::Markup(m) => {
            assert!(
                m.value.contains("Let"),
                "Hover should mention 'Let': {}",
                m.value
            );
            assert!(
                m.value.contains("variable"),
                "Hover should describe variable declaration: {}",
                m.value
            );
        }
        _ => panic!("Expected markup content"),
    }
}

#[test]
fn hover_on_variable_shows_definition() {
    let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
    let pos = Position {
        line: 2,
        character: 9,
    };
    let result = hover(&doc, pos);
    assert!(result.is_some(), "Expected hover info for variable 'x'");
    let h = result.unwrap();
    match &h.contents {
        HoverContents::Markup(m) => {
            assert!(
                m.value.contains("Let") || m.value.contains("x"),
                "Variable hover should contain 'Let' or 'x': {}",
                m.value
            );
        }
        _ => panic!("Expected markup content"),
    }
}

#[test]
fn hover_on_block_header() {
    let doc = make_doc("## Main\n    Let x be 5.\n");
    let pos = Position {
        line: 0,
        character: 0,
    };
    let result = hover(&doc, pos);
    assert!(result.is_some(), "Expected hover for block header");
    let h = result.unwrap();
    match &h.contents {
        HoverContents::Markup(m) => {
            assert!(
                m.value.contains("Block Header") || m.value.contains("entry point"),
                "Expected block header info: {}",
                m.value
            );
        }
        _ => panic!("Expected markup content"),
    }
}

#[test]
fn hover_whitespace_returns_none() {
    let doc = make_doc("## Main\n    Let x be 5.\n");
    let pos = Position {
        line: 0,
        character: 50,
    };
    let result = hover(&doc, pos);
    assert!(
        result.is_none(),
        "Hover on whitespace/OOB should return None"
    );
}

#[test]
fn hover_returns_correct_range() {
    let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
    let pos = Position {
        line: 2,
        character: 9,
    };
    let result = hover(&doc, pos);
    assert!(result.is_some());
    let h = result.unwrap();
    assert!(h.range.is_some(), "Hover should include a range");
    let range = h.range.unwrap();
    assert_eq!(range.start.line, 2, "Hover range should be on line 2");
}

#[test]
fn hover_verb_shows_lowercase_class() {
    // Verb hover should show lowercase class/time/aspect, not Debug format
    let _doc = make_doc("## Main\n    Let x be 5.\n");
    assert_eq!(
        verb_class_display(&logicaffeine_language::lexicon::VerbClass::Activity),
        "activity"
    );
    assert_eq!(
        verb_class_display(&logicaffeine_language::lexicon::VerbClass::State),
        "state"
    );
    assert_eq!(
        verb_class_display(&logicaffeine_language::lexicon::VerbClass::Accomplishment),
        "accomplishment"
    );
    assert_eq!(
        verb_class_display(&logicaffeine_language::lexicon::VerbClass::Achievement),
        "achievement"
    );
    assert_eq!(
        verb_class_display(&logicaffeine_language::lexicon::VerbClass::Semelfactive),
        "semelfactive"
    );
    assert_eq!(
        time_display(&logicaffeine_language::lexicon::Time::Past),
        "past"
    );
    assert_eq!(
        time_display(&logicaffeine_language::lexicon::Time::Present),
        "present"
    );
    assert_eq!(
        time_display(&logicaffeine_language::lexicon::Time::Future),
        "future"
    );
    assert_eq!(
        time_display(&logicaffeine_language::lexicon::Time::None),
        "none"
    );
    assert_eq!(
        aspect_display(&logicaffeine_language::lexicon::Aspect::Simple),
        "simple"
    );
    assert_eq!(
        aspect_display(&logicaffeine_language::lexicon::Aspect::Progressive),
        "progressive"
    );
    assert_eq!(
        aspect_display(&logicaffeine_language::lexicon::Aspect::Perfect),
        "perfect"
    );
}

fn hover_text(doc: &DocumentState, line: u32, character: u32) -> String {
    match hover(doc, Position { line, character }).map(|h| h.contents) {
        Some(HoverContents::Markup(m)) => m.value,
        other => panic!("no markdown hover at {line}:{character}: {other:?}"),
    }
}

/// A program with a hole (`???`) does not parse, so there is no symbol index to find a name
/// in — yet a word the text binds with `Let` is that variable, whatever it would lex as.
#[test]
fn a_let_bound_word_is_its_variable_even_in_a_program_with_holes() {
    let doc = make_doc(
        "## Main\nLet mutable tries be 0.\nWhile true:\n    Set tries to ???.\nShow tries.\n",
    );
    for (line, character) in [(1, 12), (3, 8), (4, 5)] {
        let md = hover_text(&doc, line, character);
        assert!(
            md.contains("Let mut tries"),
            "hovering tries at {line}:{character} does not describe the variable: {md}"
        );
        assert!(
            !md.contains("**Verb**"),
            "hovering the variable tries at {line}:{character} reads it as a verb: {md}"
        );
    }
}

#[test]
fn a_let_bound_word_hovers_as_its_definition() {
    let doc = make_doc("## Main\nLet mutable tries be 0.\nSet tries to tries + 1.\nShow tries.\n");
    let md = hover_text(&doc, 3, 5);
    assert!(
        md.contains("Let mut tries"),
        "hovering tries does not describe the variable: {md}"
    );
}

#[test]
fn hover_set_keyword_docs() {
    let doc = make_doc("## Main\n    Let x be 5.\n    Set x to 10.\n");
    // Position on "Set" keyword (line 2, character 4)
    let pos = Position {
        line: 2,
        character: 4,
    };
    let result = hover(&doc, pos);
    assert!(result.is_some(), "Expected hover for 'Set' keyword");
    let h = result.unwrap();
    match &h.contents {
        HoverContents::Markup(m) => {
            assert!(
                m.value.contains("Updates") || m.value.contains("Set"),
                "Set hover should contain 'Updates': {}",
                m.value
            );
        }
        _ => panic!("Expected markup content"),
    }
}

#[test]
fn hover_shows_the_notes_prose_for_a_documented_function() {
    let doc = make_doc(
        "## Note\nDoubles a number.\n\n## To double (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nLet x be double(3).\n",
    );
    // Hover over "double" at the call site (line 7).
    let pos = Position {
        line: 7,
        character: 10,
    };
    let result = hover(&doc, pos).expect("hover on documented call");
    let HoverContents::Markup(m) = &result.contents else {
        panic!("expected markup")
    };
    assert!(
        m.value.contains("double"),
        "the signature detail renders: {}",
        m.value
    );
    assert!(
        m.value.contains("Doubles a number."),
        "the ## Note prose renders: {}",
        m.value
    );
}

#[test]
fn hover_teaches_stdlib_names_from_their_literate_docs() {
    let doc = make_doc("## Main\nLet d be md5([1]).\n");
    // Hover over "md5" (line 1, character 9).
    let pos = Position {
        line: 1,
        character: 9,
    };
    let result = hover(&doc, pos).expect("hover on a stdlib name");
    let HoverContents::Markup(m) = &result.contents else {
        panic!("expected markup")
    };
    assert!(m.value.contains("standard library"), "{}", m.value);
    assert!(
        m.value.contains("MD5"),
        "the literate Note teaches: {}",
        m.value
    );
}

#[test]
fn hover_keyword_carries_the_full_lesson() {
    let doc = make_doc("## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n");
    // Position on "Give" (line 3, character 4).
    let pos = Position {
        line: 3,
        character: 4,
    };
    let result = hover(&doc, pos).expect("hover on 'Give'");
    let HoverContents::Markup(m) = &result.contents else {
        panic!("expected markup")
    };
    let lesson =
        logicaffeine_language::teach::doc_for(&TokenType::Give).expect("Give is a taught keyword");
    assert!(
        m.value.contains(lesson.what),
        "Give hover must carry the lesson's `what`: {}",
        m.value
    );
    assert!(
        m.value.contains(lesson.question_or_tip),
        "Give hover must carry the socratic question: {}",
        m.value
    );
    assert!(
        m.value.contains("LOGOS_QUICKGUIDE.md#"),
        "Give hover must link the quickguide anchor: {}",
        m.value
    );
}

#[test]
fn hover_covers_newly_taught_keywords() {
    let doc = make_doc("## Main\n    While true:\n        Break.\n");
    // Position on "Break" (line 2, character 8).
    let pos = Position {
        line: 2,
        character: 8,
    };
    let result = hover(&doc, pos).expect("hover on 'Break' — newly taught");
    let HoverContents::Markup(m) = &result.contents else {
        panic!("expected markup")
    };
    assert!(
        m.value.contains("innermost"),
        "Break hover must teach the innermost-loop rule: {}",
        m.value
    );
}

#[test]
fn hover_block_header_carries_the_lesson_question() {
    let doc = make_doc("## Main\n    Let x be 5.\n");
    let pos = Position {
        line: 0,
        character: 0,
    };
    let result = hover(&doc, pos).expect("hover on ## Main");
    let HoverContents::Markup(m) = &result.contents else {
        panic!("expected markup")
    };
    let lesson =
        logicaffeine_language::teach::doc_for_block(&logicaffeine_language::token::BlockType::Main);
    assert!(
        m.value.contains("Block Header"),
        "block hover keeps the Block Header banner: {}",
        m.value
    );
    assert!(
        m.value.contains(lesson.question_or_tip),
        "block hover must carry the lesson's question or tip: {}",
        m.value
    );
}

#[test]
fn hover_theorem_shows_proof_hint() {
    let doc = make_doc("## Theorem: all humans are mortal\n    All humans are mortal.\n");
    // Hover on the block header "##" (line 0, character 0)
    let pos = Position {
        line: 0,
        character: 0,
    };
    let result = hover(&doc, pos);
    assert!(result.is_some(), "Expected hover for Theorem block header");
    let h = result.unwrap();
    match &h.contents {
        HoverContents::Markup(m) => {
            assert!(
                m.value.contains("Proof Strategy"),
                "Theorem hover should include proof strategy hint: {}",
                m.value
            );
            assert!(
                m.value.contains("universal"),
                "Theorem with 'all' should suggest universal proof strategy: {}",
                m.value
            );
        }
        _ => panic!("Expected markup content"),
    }
}

#[test]
fn hover_theorem_implication_shows_strategy() {
    let doc = make_doc(
        "## Theorem: if it rains then ground is wet\n    If it rains then the ground is wet.\n",
    );
    let pos = Position {
        line: 0,
        character: 0,
    };
    let result = hover(&doc, pos);
    assert!(result.is_some(), "Expected hover for Theorem block");
    let h = result.unwrap();
    match &h.contents {
        HoverContents::Markup(m) => {
            assert!(
                m.value.contains("implication"),
                "Theorem with 'if' should suggest implication strategy: {}",
                m.value
            );
        }
        _ => panic!("Expected markup content"),
    }
}

#[test]
fn hover_owned_variable_shows_owned() {
    let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
    // Hover on "x" in "Show x." (line 2, character 9)
    let pos = Position {
        line: 2,
        character: 9,
    };
    let result = hover(&doc, pos);
    assert!(result.is_some(), "Expected hover for 'x'");
    let h = result.unwrap();
    match &h.contents {
        HoverContents::Markup(m) => {
            assert!(
                m.value.contains("Ownership") && m.value.contains("Owned")
                    || m.value.contains("Borrowed"),
                "Hover should show ownership state: {}",
                m.value
            );
        }
        _ => panic!("Expected markup content"),
    }
}

#[test]
fn hover_moved_variable_shows_ownership_state() {
    // After Give, x is Moved. The parser may emit use-after-move, but
    // the ownership state should still be Moved on the checker.
    let doc =
        make_doc("## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n    Show x.\n");
    // Check that x has Moved state in ownership_states
    if let Some(state) = doc.ownership_states.get("x") {
        assert!(
            matches!(state, VarState::Moved),
            "x should be Moved after Give, got: {:?}",
            state
        );
    }
}

#[test]
fn hover_borrowed_variable_shows_borrowed() {
    let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
    // After Show, x should be Borrowed (or still Owned)
    if let Some(state) = doc.ownership_states.get("x") {
        assert!(
            matches!(state, VarState::Borrowed | VarState::Owned),
            "x should be Borrowed or Owned after Show, got: {:?}",
            state
        );
    }
}

/// The lesson a hover shows, and the source text its range covers.
fn lesson_and_range(source: &str, word: &str) -> (String, String) {
    let doc = make_doc(source);
    let off = source.find(word).unwrap_or_else(|| panic!("{word:?} is not in the source"));
    let shown = hover(&doc, doc.line_index.position(off)).unwrap_or_else(|| panic!("no hover on {word:?}"));
    let HoverContents::Markup(markup) = shown.contents else { panic!("hover is markdown") };
    let range = shown.range.expect("the hover has a range");
    let start = doc.line_index.offset(range.start);
    let end = doc.line_index.offset(range.end);
    (markup.value, source[start..end].to_string())
}

/// A comparison written in English hovers as the comparison it means, over the whole phrase, with
/// the symbol it equals: `is less than` is `<` (the user, 2026-11-01).
#[test]
fn english_comparisons_hover_as_their_symbols() {
    let cases: &[(&str, &str, &str, &str, &str)] = &[
        ("## Main\nLet n be 1.\nIf n is less than 2:\n    Show n.\n", "less", "Less than", "`<`", "less than"),
        ("## Main\nLet n be 1.\nIf n is less than 2:\n    Show n.\n", "than", "Less than", "`<`", "less than"),
        ("## Main\nLet n be 1.\nIf n is greater than 0:\n    Show n.\n", "greater", "Greater than", "`>`", "greater than"),
        ("## Main\nLet n be 1.\nIf n is at least 1:\n    Show n.\n", "at least", "At least", "`>=`", "at least"),
        ("## Main\nLet n be 1.\nIf n is at most 1:\n    Show n.\n", "most", "At most", "`<=`", "at most"),
        ("## Main\nLet n be 1.\nIf n is less than or equal to 1:\n    Show n.\n", "equal", "At most", "`<=`", "less than or equal to"),
        ("## Main\nLet n be 1.\nIf n is equal to 1:\n    Show n.\n", "equal", "Equal", "`==`", "is equal to"),
        ("## Main\nLet n be 1.\nIf n equals 1:\n    Show n.\n", "equals", "Equal", "`==`", "equals"),
        ("## Main\nLet n be 1.\nIf n is not equal to 1:\n    Show n.\n", "equal", "Not equal", "`!=`", "not equal to"),
    ];
    for (source, word, lesson, symbol, phrase) in cases {
        let (markdown, covered) = lesson_and_range(source, word);
        assert!(markdown.starts_with(&format!("**{lesson}**")), "hovering {word:?} in {source:?} teaches:\n{markdown}");
        assert!(markdown.contains(symbol), "the lesson for {word:?} does not name {symbol}:\n{markdown}");
        assert_eq!(&covered, phrase, "the hover on {word:?} covers the wrong text");
    }
}

/// The symbols teach the same lessons as the words.
#[test]
fn comparison_symbols_hover_as_the_same_lessons() {
    for (symbol, lesson) in [("<", "Less than"), (">", "Greater than"), ("<=", "At most"), (">=", "At least"), ("==", "Equal"), ("!=", "Not equal")] {
        let source = format!("## Main\nLet n be 1.\nIf n {symbol} 1:\n    Show n.\n");
        let (markdown, covered) = lesson_and_range(&source, symbol);
        assert!(markdown.starts_with(&format!("**{lesson}**")), "hovering {symbol} teaches:\n{markdown}");
        assert_eq!(covered, symbol);
    }
}

/// `is` on its own is not a comparison, and the phrase reader leaves it alone.
#[test]
fn a_bare_is_does_not_hover_as_a_comparison() {
    let source = "## Main\nLet n be 1.\nIf n is 1:\n    Show n.\n";
    let doc = make_doc(source);
    let off = source.find("is 1").unwrap();
    if let Some(shown) = hover(&doc, doc.line_index.position(off)) {
        let HoverContents::Markup(markup) = shown.contents else { panic!("hover is markdown") };
        assert!(!markup.value.contains("Written `"), "a bare `is` teaches a comparison:\n{}", markup.value);
    }
}

/// Arithmetic and the connectives teach too, by symbol or by word, and `times` is an operator only
/// between two operands (DAT-106).
#[test]
fn arithmetic_and_connectives_hover_as_their_lessons() {
    let cases: &[(&str, &str, &str)] = &[
        ("## Main\nLet n be 1.\nLet b be n + 2.\nShow b.\n", "+", "Plus"),
        ("## Main\nLet n be 1.\nLet b be n plus 2.\nShow b.\n", "plus", "Plus"),
        ("## Main\nLet n be 4.\nLet b be n minus 2.\nShow b.\n", "minus", "Minus"),
        ("## Main\nLet n be 4.\nLet b be n times 2.\nShow b.\n", "times", "Times"),
        ("## Main\nLet n be 4.\nLet b be n % 2.\nShow b.\n", "%", "Remainder"),
        ("## Main\nLet n be 14.\nIf n is divisible by 7:\n    Show n.\n", "divisible", "Remainder"),
        ("## Main\nLet n be 1.\nIf n is greater than 0 and n is less than 10:\n    Show n.\n", "and", "And"),
        ("## Main\nLet done be false.\nIf not done:\n    Show 1.\n", "not", "Not"),
    ];
    for (source, word, lesson) in cases {
        let (markdown, _) = lesson_and_range(source, word);
        assert!(markdown.starts_with(&format!("**{lesson}**")), "hovering {word:?} in {source:?} teaches:\n{markdown}");
    }
    let source = "## Main\nRepeat 3 times:\n    Show 1.\n";
    let doc = make_doc(source);
    if let Some(shown) = hover(&doc, doc.line_index.position(source.find("times").unwrap())) {
        let HoverContents::Markup(markup) = shown.contents else { panic!("hover is markdown") };
        assert!(!markup.value.starts_with("**Times**"), "`Repeat 3 times` hovers as multiplication");
    }
}

/// A primitive type name hovers as the type's lesson (DAT-106: the LSP catches our types).
#[test]
fn a_primitive_type_hovers_as_its_lesson() {
    for (ty, word) in [("Int", "Int"), ("Text", "Text"), ("Seq of Int", "Seq")] {
        let source = format!("## Main\nLet x: {ty} be ???.\nShow x.\n");
        let doc = make_doc(&source);
        let off = source.find(&format!(": {word}")).unwrap() + 2;
        let shown = hover(&doc, doc.line_index.position(off)).unwrap_or_else(|| panic!("no hover on the type {word}"));
        let HoverContents::Markup(markup) = shown.contents else { panic!("hover is markdown") };
        assert!(markup.value.starts_with(&format!("**{word}**")), "hovering {word} teaches:\n{}", markup.value);
    }
}
