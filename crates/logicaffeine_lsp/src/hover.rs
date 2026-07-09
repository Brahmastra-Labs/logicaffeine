use tower_lsp::lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind, Position, Range};

use logicaffeine_compile::analysis::VarState;
use logicaffeine_language::token::{BlockType, TokenType};

use crate::document::DocumentState;

/// Handle hover request.
///
/// Shows type info, signatures, and keyword documentation depending on
/// what's under the cursor.
pub fn hover(doc: &DocumentState, position: Position) -> Option<Hover> {
    let offset = doc.line_index.offset(position);

    // Find the token at the cursor position
    let token = doc.tokens.iter().find(|t| {
        offset >= t.span.start && offset < t.span.end
    })?;

    let text = doc.source.get(token.span.start..token.span.end)?;

    let content = match &token.kind {
        // Block headers → the shared lesson table (total over BlockType),
        // plus the socratic proof-strategy hint on theorems.
        TokenType::BlockHeader { block_type } => {
            let lesson = logicaffeine_language::teach::doc_for_block(block_type);
            let mut result = crate::teach_md::block_hover_md(lesson);

            if *block_type == BlockType::Theorem {
                let hint = proof_strategy_hint_from_source(&doc.source, token.span.start);
                result.push_str(&format!("\n\n---\n\n**Proof Strategy**: {}", hint));
            }

            Some(result)
        }

        // Taught keywords → the shared lesson table (one brain for hover,
        // completion docs, and the REPL's :explain).
        kind if logicaffeine_language::teach::doc_for(kind).is_some() => {
            logicaffeine_language::teach::doc_for(kind).map(crate::teach_md::keyword_hover_md)
        }

        // Identifiers, proper names, adjectives, nouns → look up definition
        // info first (detail + literate doc), then the stdlib registry.
        TokenType::Identifier | TokenType::ProperName(_)
        | TokenType::Adjective(_) | TokenType::Noun(_) => {
            let defs = doc.symbol_index.definitions_of(text);
            if let Some(def) = defs.first() {
                definition_hover(def)
            } else if let Some(entry) = crate::stdlib_docs::stdlib_doc(text) {
                Some(crate::stdlib_docs::hover_md(text, entry))
            } else {
                // Fall back to token-specific info
                match &token.kind {
                    TokenType::Noun(sym) => {
                        Some(format!("**Noun**: {}", doc.interner.resolve(*sym)))
                    }
                    TokenType::Adjective(sym) => {
                        Some(format!("**Adjective**: {}", doc.interner.resolve(*sym)))
                    }
                    _ => None,
                }
            }
        }

        // Verbs → a defined function first (English-word function names lex
        // as verbs), then the stdlib registry, then the verb-class display.
        TokenType::Verb { lemma, time, aspect, class } => {
            let defs = doc.symbol_index.definitions_of(text);
            if let Some(def) = defs.first() {
                definition_hover(def)
            } else if let Some(entry) = crate::stdlib_docs::stdlib_doc(text) {
                Some(crate::stdlib_docs::hover_md(text, entry))
            } else {
                let lemma_str = doc.interner.resolve(*lemma);
                Some(format!(
                    "**Verb**: {} ({}, {}, {})",
                    lemma_str,
                    verb_class_display(class),
                    time_display(time),
                    aspect_display(aspect),
                ))
            }
        }

        // Type names → look up in registry
        _ => {
            // Check if this is a type name
            if let Some(sym) = doc.interner.lookup(text) {
                if let Some(typedef) = doc.type_registry.get(sym) {
                    Some(format_typedef(text, typedef, &doc.interner))
                } else {
                    None
                }
            } else {
                None
            }
        }
    };

    let token_range = Range {
        start: doc.line_index.position(token.span.start),
        end: doc.line_index.position(token.span.end),
    };

    // Enrich with ownership state and diagnostics for variable-like tokens
    let content = content.map(|mut c| {
        if matches!(
            &token.kind,
            TokenType::Identifier | TokenType::ProperName(_)
            | TokenType::Adjective(_) | TokenType::Noun(_)
        ) {
            // Ownership state
            if let Some(state) = doc.ownership_states.get(text) {
                let state_str = match state {
                    VarState::Owned => "Owned",
                    VarState::Moved => "Moved",
                    VarState::MaybeMoved => "Maybe Moved",
                    VarState::Borrowed => "Borrowed",
                };
                c.push_str(&format!("\n\n**Ownership**: {}", state_str));
                match state {
                    VarState::Moved => {
                        c.push_str("\n\n*This variable has been given away and can no longer be used.*");
                    }
                    VarState::MaybeMoved => {
                        c.push_str("\n\n*This variable might have been given away in a conditional branch.*");
                    }
                    _ => {}
                }
            }

            // Diagnostics affecting this token
            let affecting: Vec<_> = doc.diagnostics.iter()
                .filter(|d| ranges_overlap(&token_range, &d.range))
                .collect();
            if !affecting.is_empty() {
                c.push_str("\n\n---\n");
                for diag in affecting.iter().take(3) {
                    let first_line = diag.message.lines().next().unwrap_or(&diag.message);
                    c.push_str(&format!("\n**Diagnostic**: {}", first_line));
                }
            }
        }
        c
    });

    content.map(|c| Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: c,
        }),
        range: Some(token_range),
    })
}

fn verb_class_display(class: &logicaffeine_language::lexicon::VerbClass) -> &'static str {
    match class {
        logicaffeine_language::lexicon::VerbClass::State => "state",
        logicaffeine_language::lexicon::VerbClass::Activity => "activity",
        logicaffeine_language::lexicon::VerbClass::Accomplishment => "accomplishment",
        logicaffeine_language::lexicon::VerbClass::Achievement => "achievement",
        logicaffeine_language::lexicon::VerbClass::Semelfactive => "semelfactive",
    }
}

fn time_display(time: &logicaffeine_language::lexicon::Time) -> &'static str {
    match time {
        logicaffeine_language::lexicon::Time::Past => "past",
        logicaffeine_language::lexicon::Time::Present => "present",
        logicaffeine_language::lexicon::Time::Future => "future",
        logicaffeine_language::lexicon::Time::None => "none",
    }
}

fn aspect_display(aspect: &logicaffeine_language::lexicon::Aspect) -> &'static str {
    match aspect {
        logicaffeine_language::lexicon::Aspect::Simple => "simple",
        logicaffeine_language::lexicon::Aspect::Progressive => "progressive",
        logicaffeine_language::lexicon::Aspect::Perfect => "perfect",
    }
}

/// A definition's hover: its signature detail plus its literate `## Note`
/// documentation, when either exists.
fn definition_hover(def: &crate::index::Definition) -> Option<String> {
    match (&def.detail, &def.doc) {
        (Some(detail), Some(doc)) => Some(format!("{detail}\n\n{doc}")),
        (Some(detail), None) => Some(detail.clone()),
        (None, Some(doc)) => Some(doc.clone()),
        (None, None) => None,
    }
}

fn ranges_overlap(a: &Range, b: &Range) -> bool {
    !(a.end.line < b.start.line
        || (a.end.line == b.start.line && a.end.character < b.start.character)
        || b.end.line < a.start.line
        || (b.end.line == a.start.line && b.end.character < a.start.character))
}

fn format_typedef(
    name: &str,
    typedef: &logicaffeine_language::analysis::TypeDef,
    interner: &logicaffeine_base::Interner,
) -> String {
    match typedef {
        logicaffeine_language::analysis::TypeDef::Struct { fields, .. } => {
            let mut s = format!("**struct** {}\n\n", name);
            for field in fields {
                let field_name = interner.resolve(field.name);
                let field_type = format_field_type(&field.ty, interner);
                s.push_str(&format!("- `{}`: {}\n", field_name, field_type));
            }
            s
        }
        logicaffeine_language::analysis::TypeDef::Enum { variants, .. } => {
            let mut s = format!("**enum** {}\n\n", name);
            for variant in variants {
                let variant_name = interner.resolve(variant.name);
                if variant.fields.is_empty() {
                    s.push_str(&format!("- `{}`\n", variant_name));
                } else {
                    let fields: Vec<String> = variant
                        .fields
                        .iter()
                        .map(|f| {
                            format!(
                                "{}: {}",
                                interner.resolve(f.name),
                                format_field_type(&f.ty, interner)
                            )
                        })
                        .collect();
                    s.push_str(&format!("- `{}` with {}\n", variant_name, fields.join(", ")));
                }
            }
            s
        }
        logicaffeine_language::analysis::TypeDef::Primitive => {
            format!("**primitive** {}", name)
        }
        logicaffeine_language::analysis::TypeDef::Generic { param_count } => {
            format!("**generic** {} (takes {} type parameter(s))", name, param_count)
        }
        logicaffeine_language::analysis::TypeDef::Alias { target } => {
            format!("**alias** {} = {}", name, interner.resolve(*target))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentState;

    fn make_doc(source: &str) -> DocumentState {
        DocumentState::new(source.to_string(), 1)
    }

    #[test]
    fn hover_on_keyword_shows_docs() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        // Position on "Let" (line 1, character 4)
        let pos = Position { line: 1, character: 4 };
        let result = hover(&doc, pos);
        assert!(result.is_some(), "Expected hover info for 'Let'");
        let h = result.unwrap();
        match &h.contents {
            HoverContents::Markup(m) => {
                assert!(m.value.contains("Let"), "Hover should mention 'Let': {}", m.value);
                assert!(m.value.contains("variable"), "Hover should describe variable declaration: {}", m.value);
            }
            _ => panic!("Expected markup content"),
        }
    }

    #[test]
    fn hover_on_variable_shows_definition() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
        let pos = Position { line: 2, character: 9 };
        let result = hover(&doc, pos);
        assert!(result.is_some(), "Expected hover info for variable 'x'");
        let h = result.unwrap();
        match &h.contents {
            HoverContents::Markup(m) => {
                assert!(m.value.contains("Let") || m.value.contains("x"),
                    "Variable hover should contain 'Let' or 'x': {}", m.value);
            }
            _ => panic!("Expected markup content"),
        }
    }

    #[test]
    fn hover_on_block_header() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        let pos = Position { line: 0, character: 0 };
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
        let pos = Position { line: 0, character: 50 };
        let result = hover(&doc, pos);
        assert!(result.is_none(), "Hover on whitespace/OOB should return None");
    }

    #[test]
    fn hover_returns_correct_range() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
        let pos = Position { line: 2, character: 9 };
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
        assert_eq!(verb_class_display(&logicaffeine_language::lexicon::VerbClass::Activity), "activity");
        assert_eq!(verb_class_display(&logicaffeine_language::lexicon::VerbClass::State), "state");
        assert_eq!(verb_class_display(&logicaffeine_language::lexicon::VerbClass::Accomplishment), "accomplishment");
        assert_eq!(verb_class_display(&logicaffeine_language::lexicon::VerbClass::Achievement), "achievement");
        assert_eq!(verb_class_display(&logicaffeine_language::lexicon::VerbClass::Semelfactive), "semelfactive");
        assert_eq!(time_display(&logicaffeine_language::lexicon::Time::Past), "past");
        assert_eq!(time_display(&logicaffeine_language::lexicon::Time::Present), "present");
        assert_eq!(time_display(&logicaffeine_language::lexicon::Time::Future), "future");
        assert_eq!(time_display(&logicaffeine_language::lexicon::Time::None), "none");
        assert_eq!(aspect_display(&logicaffeine_language::lexicon::Aspect::Simple), "simple");
        assert_eq!(aspect_display(&logicaffeine_language::lexicon::Aspect::Progressive), "progressive");
        assert_eq!(aspect_display(&logicaffeine_language::lexicon::Aspect::Perfect), "perfect");
    }

    #[test]
    fn hover_set_keyword_docs() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Set x to 10.\n");
        // Position on "Set" keyword (line 2, character 4)
        let pos = Position { line: 2, character: 4 };
        let result = hover(&doc, pos);
        assert!(result.is_some(), "Expected hover for 'Set' keyword");
        let h = result.unwrap();
        match &h.contents {
            HoverContents::Markup(m) => {
                assert!(m.value.contains("Updates") || m.value.contains("Set"),
                    "Set hover should contain 'Updates': {}", m.value);
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
        let pos = Position { line: 7, character: 10 };
        let result = hover(&doc, pos).expect("hover on documented call");
        let HoverContents::Markup(m) = &result.contents else { panic!("expected markup") };
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
        let pos = Position { line: 1, character: 9 };
        let result = hover(&doc, pos).expect("hover on a stdlib name");
        let HoverContents::Markup(m) = &result.contents else { panic!("expected markup") };
        assert!(m.value.contains("standard library"), "{}", m.value);
        assert!(m.value.contains("MD5"), "the literate Note teaches: {}", m.value);
    }

    #[test]
    fn hover_keyword_carries_the_full_lesson() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n");
        // Position on "Give" (line 3, character 4).
        let pos = Position { line: 3, character: 4 };
        let result = hover(&doc, pos).expect("hover on 'Give'");
        let HoverContents::Markup(m) = &result.contents else { panic!("expected markup") };
        let lesson = logicaffeine_language::teach::doc_for(&TokenType::Give)
            .expect("Give is a taught keyword");
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
        let pos = Position { line: 2, character: 8 };
        let result = hover(&doc, pos).expect("hover on 'Break' — newly taught");
        let HoverContents::Markup(m) = &result.contents else { panic!("expected markup") };
        assert!(
            m.value.contains("innermost"),
            "Break hover must teach the innermost-loop rule: {}",
            m.value
        );
    }

    #[test]
    fn hover_block_header_carries_the_lesson_question() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        let pos = Position { line: 0, character: 0 };
        let result = hover(&doc, pos).expect("hover on ## Main");
        let HoverContents::Markup(m) = &result.contents else { panic!("expected markup") };
        let lesson = logicaffeine_language::teach::doc_for_block(
            &logicaffeine_language::token::BlockType::Main,
        );
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
        let pos = Position { line: 0, character: 0 };
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
        let doc = make_doc("## Theorem: if it rains then ground is wet\n    If it rains then the ground is wet.\n");
        let pos = Position { line: 0, character: 0 };
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
        let pos = Position { line: 2, character: 9 };
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
        let doc = make_doc("## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n    Show x.\n");
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
}

/// Analyze the source text of a Theorem block to suggest a proof strategy.
///
/// Scans the block's body text for structural keywords (quantifiers, connectives)
/// and returns a Socratic hint guiding the user toward the right tactic.
fn proof_strategy_hint_from_source(source: &str, block_start: usize) -> String {
    let block_text = &source[block_start..];
    let block_end = block_text.find("\n## ").unwrap_or(block_text.len());
    let block_body = &block_text[..block_end].to_lowercase();

    if block_body.contains("for all") || block_body.contains("every") || block_body.contains("all ") {
        return "Your theorem involves a universal claim. To prove it, consider an arbitrary element and show the property holds.".to_string();
    }
    if block_body.contains("there exists") || block_body.contains("some ") {
        return "Your theorem involves an existential claim. You need to find a specific witness that satisfies the property.".to_string();
    }
    if block_body.contains(" implies ") || block_body.contains("if ") {
        return "Your theorem is an implication. Try assuming the antecedent and proving the consequent.".to_string();
    }
    if block_body.contains(" and ") {
        return "Your theorem is a conjunction. You need to prove both parts separately.".to_string();
    }
    if block_body.contains(" or ") {
        return "Your theorem is a disjunction. You only need to prove one of the alternatives.".to_string();
    }
    if block_body.contains("not ") || block_body.contains("no ") {
        return "Your theorem involves negation. Try assuming the positive form and deriving a contradiction.".to_string();
    }
    if block_body.contains("equals") || block_body.contains(" = ") {
        return "Your theorem is an equality. Can you rewrite one side to match the other, or use reflexivity?".to_string();
    }

    "What logical structure does your theorem have? Try breaking it down into simpler parts.".to_string()
}

pub fn format_field_type(
    ty: &logicaffeine_language::analysis::FieldType,
    interner: &logicaffeine_base::Interner,
) -> String {
    match ty {
        logicaffeine_language::analysis::FieldType::Primitive(sym) => {
            interner.resolve(*sym).to_string()
        }
        logicaffeine_language::analysis::FieldType::Named(sym) => {
            interner.resolve(*sym).to_string()
        }
        logicaffeine_language::analysis::FieldType::Generic { base, params } => {
            let base_name = interner.resolve(*base);
            let param_strs: Vec<String> = params
                .iter()
                .map(|p| format_field_type(p, interner))
                .collect();
            format!("{} of {}", base_name, param_strs.join(", "))
        }
        logicaffeine_language::analysis::FieldType::TypeParam(sym) => {
            interner.resolve(*sym).to_string()
        }
    }
}
