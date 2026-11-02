use lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind, Position, Range};

use logicaffeine_compile::analysis::VarState;
use logicaffeine_language::token::{BlockType, TokenType};

use crate::document::DocumentState;
use crate::workspace::WorkspaceView;

/// Handle hover request.
///
/// Shows type info, signatures, and keyword documentation depending on
/// what's under the cursor.
pub fn hover(doc: &DocumentState, position: Position) -> Option<Hover> {
    let offset = doc.line_index.offset(position);

    // ★ A NAME INSIDE A TEMPLATE IS NOT A TOKEN. The match below dispatches on the token's KIND,
    // and a `View:` body is one `RawBlock` — so `{count}` never reached the identifier arm and
    // hovering it said nothing at all. The reference the parser recorded is the only place that
    // name exists by now, and it already points at the definition, which is exactly what the
    // identifier arm goes on to look up.
    //
    // ★ ONLY WHERE THERE IS NO NAME TOKEN, which means only inside a raw block. Taking this path
    // whenever a reference covered the cursor ALSO took it for ordinary code, and skipped the
    // enrichment below — the ownership state, the diagnostics on the token — so hovering a plain
    // `x` stopped saying whether it had been given away. A narrower gate keeps the two worlds
    // apart: outside a template the token path was always right and still is.
    let inside_raw_block = doc
        .tokens
        .iter()
        .find(|t| offset >= t.span.start && offset < t.span.end)
        .is_some_and(|t| matches!(t.kind, TokenType::RawBlock { .. }));
    if let Some(reference) = doc
        .symbol_index
        .references
        .iter()
        .find(|r| inside_raw_block && offset >= r.span.start && offset < r.span.end)
    {
        if let Some(def) = reference
            .definition_idx
            .and_then(|i| doc.symbol_index.definitions.get(i))
        {
            if let Some(md) = definition_hover(def) {
                return Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: md,
                    }),
                    range: Some(Range {
                        start: doc.line_index.position(reference.span.start),
                        end: doc.line_index.position(reference.span.end),
                    }),
                });
            }
        }
    }

    // Find the token at the cursor position
    let at = doc
        .tokens
        .iter()
        .position(|t| offset >= t.span.start && offset < t.span.end)?;
    let token = &doc.tokens[at];

    // A comparison written in English (`is less than`, `at least`, `is equal to`) is several
    // tokens that mean one thing; the hover covers the phrase and teaches the comparison, the
    // same lesson `<`, `>=` or `==` would.
    if let Some((lesson, span)) = logicaffeine_language::teach::doc_for_phrase(&doc.source, &doc.tokens, at) {
        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: crate::teach_md::keyword_hover_md(lesson),
            }),
            range: Some(Range {
                start: doc.line_index.position(span.start),
                end: doc.line_index.position(span.end),
            }),
        });
    }

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
        TokenType::Identifier
        | TokenType::ProperName(_)
        | TokenType::Adjective(_)
        | TokenType::Noun(_) => {
            let defs = doc.symbol_index.definitions_of(text);
            if let Some(def) = defs.first() {
                definition_hover(def)
            } else if let Some(binding) = let_binding(doc, text) {
                Some(binding)
            } else if let Some(lesson) = logicaffeine_language::teach::doc_for_primitive(text) {
                // `Int`, `Text`, `Seq`: the type's lesson (DAT-106).
                Some(crate::teach_md::keyword_hover_md(lesson))
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
        TokenType::Verb {
            lemma,
            time,
            aspect,
            class,
        } => {
            let defs = doc.symbol_index.definitions_of(text);
            if let Some(def) = defs.first() {
                definition_hover(def)
            } else if let Some(binding) = let_binding(doc, text) {
                Some(binding)
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

        // Type names → the primitive's lesson, else the registry
        _ => {
            if let Some(lesson) = logicaffeine_language::teach::doc_for_primitive(text) {
                Some(crate::teach_md::keyword_hover_md(lesson))
            } else if let Some(sym) = doc.interner.lookup(text) {
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

    // Enrich variable-like tokens with their ownership state. A token's problems are the
    // editor's to show, in its own section of the hover — repeating them here says each twice.
    let content = content.map(|mut c| {
        if matches!(
            &token.kind,
            TokenType::Identifier
                | TokenType::ProperName(_)
                | TokenType::Adjective(_)
                | TokenType::Noun(_)
        ) {
            // Ownership state
            if let Some(state) = doc.ownership_states.get(text) {
                let state_str = match state {
                    VarState::Owned => "Owned",
                    VarState::Moved => "Moved",
                    VarState::MaybeMoved(_) => "Maybe Moved",
                    VarState::Borrowed => "Borrowed",
                };
                c.push_str(&format!("\n\n**Ownership**: {}", state_str));
                match state {
                    VarState::Moved => {
                        c.push_str(
                            "\n\n*This variable has been given away and can no longer be used.*",
                        );
                    }
                    VarState::MaybeMoved(given_in) => {
                        c.push_str(&format!(
                            "\n\n*This variable might have been given away in {given_in}.*"
                        ));
                    }
                    _ => {}
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
/// The `Let` that binds `name`, read from the tokens alone — `Let mut tries` for
/// `Let mutable tries be 0.` A program that does not parse (an exercise with a `???` hole) has
/// no symbol index to find its variables in, yet its `Let` lines still say which words they
/// are, whatever those words would lex as on their own.
fn let_binding(doc: &DocumentState, name: &str) -> Option<String> {
    let text = |i: usize| {
        doc.tokens
            .get(i)
            .and_then(|t| doc.source.get(t.span.start..t.span.end))
            .unwrap_or("")
    };
    (0..doc.tokens.len()).find_map(|i| {
        if !matches!(doc.tokens[i].kind, TokenType::Let) {
            return None;
        }
        let mutable = matches!(doc.tokens.get(i + 1).map(|t| &t.kind), Some(TokenType::Mut))
            || text(i + 1).eq_ignore_ascii_case("mutable")
            || text(i + 1).eq_ignore_ascii_case("mut");
        let bound = if mutable { i + 2 } else { i + 1 };
        (text(bound) == name).then(|| format!("Let {}{name}", if mutable { "mut " } else { "" }))
    })
}

fn definition_hover(def: &crate::index::Definition) -> Option<String> {
    match (&def.detail, &def.doc) {
        (Some(detail), Some(doc)) => Some(format!("{detail}\n\n{doc}")),
        (Some(detail), None) => Some(detail.clone()),
        (None, Some(doc)) => Some(doc.clone()),
        (None, None) => None,
    }
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
                    s.push_str(&format!(
                        "- `{}` with {}\n",
                        variant_name,
                        fields.join(", ")
                    ));
                }
            }
            s
        }
        logicaffeine_language::analysis::TypeDef::Primitive => {
            format!("**primitive** {}", name)
        }
        logicaffeine_language::analysis::TypeDef::Generic { param_count } => {
            format!(
                "**generic** {} (takes {} type parameter(s))",
                name, param_count
            )
        }
        logicaffeine_language::analysis::TypeDef::Alias { target } => {
            format!("**alias** {} = {}", name, interner.resolve(*target))
        }
    }
}

/// "Defined in module `util.lg`" for a cross-file name — a name NOT defined in
/// this document and NOT a stdlib name, but publicly exported by some project
/// module. Returns `None` for local/stdlib names (they have their own hover).
pub fn module_attribution(
    doc: &DocumentState,
    position: Position,
    ws: &dyn WorkspaceView,
) -> Option<String> {
    let offset = doc.line_index.offset(position);
    let token = doc
        .tokens
        .iter()
        .find(|t| offset >= t.span.start && offset < t.span.end)?;
    let text = doc.source.get(token.span.start..token.span.end)?;

    if !doc.symbol_index.definitions_of(text).is_empty() {
        return None;
    }
    if crate::stdlib_docs::stdlib_doc(text).is_some() {
        return None;
    }
    let files: Vec<String> = ws
        .module_exporting(text)
        .iter()
        .filter_map(|e| crate::workspace::url_path(&e.module_url))
        .filter_map(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .map(|s| format!("`{s}`"))
        })
        .collect();
    if files.is_empty() {
        return None;
    }
    Some(format!("*Defined in module {}*", files.join(", ")))
}

/// [`hover`] enriched with cross-file module attribution: when the name under the
/// cursor is defined in another project module, append "Defined in module X".
pub fn hover_with_workspace(
    doc: &DocumentState,
    position: Position,
    ws: &dyn WorkspaceView,
) -> Option<Hover> {
    let base = hover(doc, position);
    let attribution = module_attribution(doc, position, ws);
    match (base, attribution) {
        (Some(mut h), Some(attr)) => {
            if let HoverContents::Markup(mc) = &mut h.contents {
                mc.value.push_str("\n\n---\n\n");
                mc.value.push_str(&attr);
            }
            Some(h)
        }
        (Some(h), None) => Some(h),
        (None, Some(attr)) => Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: attr,
            }),
            range: None,
        }),
        (None, None) => None,
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/hover/tests.rs"]
mod tests;

/// Analyze the source text of a Theorem block to suggest a proof strategy.
///
/// Scans the block's body text for structural keywords (quantifiers, connectives)
/// and returns a Socratic hint guiding the user toward the right tactic.
fn proof_strategy_hint_from_source(source: &str, block_start: usize) -> String {
    let block_text = &source[block_start..];
    let block_end = block_text.find("\n## ").unwrap_or(block_text.len());
    let block_body = &block_text[..block_end].to_lowercase();

    if block_body.contains("for all") || block_body.contains("every") || block_body.contains("all ")
    {
        return "Your theorem involves a universal claim. To prove it, consider an arbitrary element and show the property holds.".to_string();
    }
    if block_body.contains("there exists") || block_body.contains("some ") {
        return "Your theorem involves an existential claim. You need to find a specific witness that satisfies the property.".to_string();
    }
    if block_body.contains(" implies ") || block_body.contains("if ") {
        return "Your theorem is an implication. Try assuming the antecedent and proving the consequent.".to_string();
    }
    if block_body.contains(" and ") {
        return "Your theorem is a conjunction. You need to prove both parts separately."
            .to_string();
    }
    if block_body.contains(" or ") {
        return "Your theorem is a disjunction. You only need to prove one of the alternatives."
            .to_string();
    }
    if block_body.contains("not ") || block_body.contains("no ") {
        return "Your theorem involves negation. Try assuming the positive form and deriving a contradiction.".to_string();
    }
    if block_body.contains("equals") || block_body.contains(" = ") {
        return "Your theorem is an equality. Can you rewrite one side to match the other, or use reflexivity?".to_string();
    }

    "What logical structure does your theorem have? Try breaking it down into simpler parts."
        .to_string()
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
        logicaffeine_language::analysis::FieldType::Tuple(items) => {
            let parts: Vec<String> = items
                .iter()
                .map(|t| format_field_type(t, interner))
                .collect();
            format!("({})", parts.join(", "))
        }
        logicaffeine_language::analysis::FieldType::Record(fields) => {
            let parts: Vec<String> = fields
                .iter()
                .map(|(n, t)| {
                    format!(
                        "{}: {}",
                        interner.resolve(*n),
                        format_field_type(t, interner)
                    )
                })
                .collect();
            format!("({})", parts.join(", "))
        }
    }
}
