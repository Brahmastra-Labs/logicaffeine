use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionItemTag, CompletionResponse, Position, Url,
};

use logicaffeine_compile::analysis::VarState;
use logicaffeine_language::token::{RawKind, TokenType};

use crate::document::DocumentState;
use crate::index::DefinitionKind;
use crate::workspace::WorkspaceView;

/// Handle completion request.
///
/// Provides context-aware completions based on the preceding tokens.
pub fn completions(doc: &DocumentState, position: Position) -> Option<CompletionResponse> {
    let offset = doc.line_index.offset(position);

    // The preceding token sets the context. NONE (empty document, or the cursor
    // before the first token) is not a dead end — it gets identifiers, keywords, and (below)
    // stdlib names alike, so a fresh file still completes.
    let prev_token = doc.tokens.iter().rev().find(|t| t.span.end <= offset);

    let mut items = Vec::new();

    // ★ A TEMPLATE HOLE HAS ITS OWN, SMALL VOCABULARY — and it used to be handed the whole stdlib.
    //
    // Completion dispatches on the PREVIOUS TOKEN, and a `View:` body is one `RawBlock`, so a
    // cursor inside a template fell through to the general fallback: every function the prelude
    // declares, about a thousand of them. Typing `{co` offered `cConj`, `cCosh`, `commonRunLen`
    // and never `count`. **Useless suggestions are not a neutral default** — they bury the two
    // names that were wanted, and an author who is shown a thousand irrelevant entries once stops
    // reading the list at all.
    //
    // What a hole can hold is knowable: this page's state, props and actions, the components it
    // can place, and the template tags. That is a few dozen entries, all of them plausible.
    //
    // A `Style:` body is CSS, and its vocabulary is knowable too: a scoped stylesheet matches
    // only what its own component renders.
    match raw_section_at(doc, offset) {
        Some(RawKind::View | RawKind::Head) => {
            add_template_completions(doc, &mut items);
            return Some(CompletionResponse::Array(items));
        }
        Some(RawKind::Style) => {
            add_style_completions(doc, offset, &mut items);
            return Some(CompletionResponse::Array(items));
        }
        None => {}
    }

    match prev_token.map(|t| &t.kind) {
        // After period, at start of line, or before the first token → statement keywords. A
        // token-less cursor must not depend on `add_identifier_completions` coming back empty to
        // reach this: a compiler-registered built-in type (`HttpError`, `File`) can populate
        // `items` even for an otherwise-empty document, and a statement start still needs its
        // keywords regardless of whatever else is offered alongside them.
        None | Some(TokenType::Period | TokenType::Indent | TokenType::Newline) => {
            add_statement_keywords(&mut items);
            if prev_token.is_none() {
                add_identifier_completions(doc, &mut items);
            }
        }

        // After "Let x be" or similar → expression completions
        Some(TokenType::Be) => {
            add_expression_completions(doc, &mut items);
        }

        // After ":" → type completions
        Some(TokenType::Colon) => {
            add_type_completions(doc, &mut items);
        }

        // After possessive ('s) → field completions
        Some(TokenType::Possessive) => {
            // Look back to find the object before possessive
            add_field_completions(doc, offset, &mut items);
        }

        // After "Inspect x:" → variant completions
        Some(TokenType::Inspect) => {
            add_variant_completions(doc, &mut items);
        }

        // Default → variables and functions in scope
        _ => {
            add_identifier_completions(doc, &mut items);
        }
    }

    if items.is_empty() {
        // Fallback: always offer identifiers + keywords
        add_identifier_completions(doc, &mut items);
        add_statement_keywords(&mut items);
    }

    // Stdlib prelude names ride along in every context, after the locals —
    // additive, so they can never displace the context-specific items above.
    add_stdlib_completions(doc, &mut items);

    Some(CompletionResponse::Array(items))
}

/// [`completions`] enriched with cross-module knowledge from the workspace: after
/// `Import `, the available module NAMES; in code, the public SYMBOLS of the
/// modules this file imports (each labelled `from <Module>`). Additive — never
/// displaces the local + stdlib completions.
pub fn completions_with_workspace(
    doc: &DocumentState,
    position: Position,
    ws: &dyn WorkspaceView,
    uri: &Url,
) -> Option<CompletionResponse> {
    let mut items = match completions(doc, position) {
        Some(CompletionResponse::Array(v)) => v,
        Some(CompletionResponse::List(list)) => list.items,
        None => Vec::new(),
    };

    let offset = doc.line_index.offset(position);
    // A stylesheet names selectors, never code, so no module's symbols belong in it.
    if raw_section_at(doc, offset) == Some(RawKind::Style) {
        return Some(CompletionResponse::Array(items));
    }
    let line_start = doc.source[..offset.min(doc.source.len())]
        .rfind('\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let line_prefix = &doc.source[line_start..offset.min(doc.source.len())];

    if line_prefix.trim_start().starts_with("Import") {
        // Import-statement context → complete module names not already on the line.
        for (alias, _url) in ws.module_names() {
            if line_prefix.contains(&alias) {
                continue;
            }
            items.push(CompletionItem {
                label: alias,
                kind: Some(CompletionItemKind::MODULE),
                ..Default::default()
            });
        }
    } else {
        // Code context → exactly the names this file's imports bring into scope
        // (a selective import narrows to its selected names, not the whole module).
        for (label, alias) in ws.imported_symbols(uri) {
            items.push(CompletionItem {
                label,
                kind: Some(CompletionItemKind::FUNCTION),
                detail: Some(format!("from {alias}")),
                ..Default::default()
            });
        }
    }

    Some(CompletionResponse::Array(items))
}

fn add_statement_keywords(items: &mut Vec<CompletionItem>) {
    // Snippet skeletons are editing mechanics and live here; the teaching
    // text (detail + documentation) comes from the shared lesson table, so
    // completion, hover, and the REPL always say the same thing.
    let keywords = [
        (TokenType::Let, "Let ${1:name} be ${2:value}."),
        (TokenType::Set, "Set ${1:name} to ${2:value}."),
        (TokenType::If, "If ${1:condition}:\n    ${2:body}"),
        (TokenType::While, "While ${1:condition}:\n    ${2:body}"),
        (
            TokenType::Repeat,
            "Repeat for ${1:item} in ${2:collection}:\n    ${3:body}",
        ),
        (TokenType::Return, "Return ${1:value}."),
        (TokenType::Show, "Show ${1:value}."),
        (TokenType::Give, "Give ${1:value} to ${2:target}."),
        (TokenType::Push, "Push ${1:value} to ${2:list}."),
        (TokenType::Call, "Call ${1:function} with ${2:args}."),
        (
            TokenType::Inspect,
            "Inspect ${1:value}:\n    ${2:pattern}:\n        ${3:body}",
        ),
    ];

    for (kind, snippet) in keywords {
        let lesson = logicaffeine_language::teach::doc_for(&kind)
            .expect("every statement-keyword completion is taught");
        items.push(CompletionItem {
            label: lesson.name.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some(lesson.what.to_string()),
            documentation: Some(crate::teach_md::completion_docs(lesson)),
            insert_text: Some(snippet.to_string()),
            insert_text_format: Some(lsp_types::InsertTextFormat::SNIPPET),
            ..Default::default()
        });
    }
}

/// The raw section (`View:`, `Style:`, `Head:` body — one `RawBlock` token to the lexer) the
/// cursor is inside, if any.
fn raw_section_at(doc: &DocumentState, offset: usize) -> Option<RawKind> {
    doc.tokens.iter().find_map(|t| match t.kind {
        TokenType::RawBlock { kind, .. } if offset > t.span.start && offset <= t.span.end => {
            Some(kind)
        }
        _ => None,
    })
}

/// What a scoped stylesheet can match: the element names, classes (static `class="…"` words and
/// `class:name` toggles) and ids the `View:` of the SAME `##` block renders, plus `:global(…)` —
/// the way out of the scope.
fn add_style_completions(doc: &DocumentState, offset: usize, items: &mut Vec<CompletionItem>) {
    use logicaffeine_language::lgx::html::{self, AttrPart, AttrValue, HtmlToken};
    use std::collections::BTreeSet;

    // The `##` block holding the cursor: from the header before it to the header after it.
    let block_start = doc
        .tokens
        .iter()
        .filter(|t| matches!(t.kind, TokenType::BlockHeader { .. }) && t.span.start <= offset)
        .map(|t| t.span.start)
        .last()
        .unwrap_or(0);
    let block_end = doc
        .tokens
        .iter()
        .find(|t| matches!(t.kind, TokenType::BlockHeader { .. }) && t.span.start > offset)
        .map_or(doc.source.len(), |t| t.span.start);

    let mut tags = BTreeSet::new();
    let mut classes = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let views = doc.tokens.iter().filter(|t| {
        matches!(
            t.kind,
            TokenType::RawBlock {
                kind: RawKind::View,
                ..
            }
        ) && t.span.start >= block_start
            && t.span.end <= block_end
    });
    for view in views {
        let Some(body) = doc.source.get(view.span.start..view.span.end) else {
            continue;
        };
        let Ok(tokens) = html::tokenize(body) else {
            continue;
        };
        for token in tokens {
            let HtmlToken::StartTag { name, attrs, .. } = token else {
                continue;
            };
            if !html::is_component_name(&name) {
                tags.insert(name.to_ascii_lowercase());
            }
            for attr in attrs {
                if let Some(toggled) = attr.name.strip_prefix("class:") {
                    classes.insert(toggled.to_string());
                    continue;
                }
                let words = match &attr.value {
                    Some(AttrValue::Static(v)) => vec![v.clone()],
                    // A word touching a `{…}` is only part of a class name; the whole words of the
                    // text between holes are real ones.
                    Some(AttrValue::Parts(parts)) => parts
                        .iter()
                        .enumerate()
                        .filter_map(|(i, p)| match p {
                            AttrPart::Text(t) => {
                                let mut words: Vec<&str> = t.split_whitespace().collect();
                                if i > 0 && !t.starts_with(char::is_whitespace) && !words.is_empty()
                                {
                                    words.remove(0);
                                }
                                if i + 1 < parts.len() && !t.ends_with(char::is_whitespace) {
                                    words.pop();
                                }
                                Some(words.join(" "))
                            }
                            AttrPart::Brace { .. } => None,
                        })
                        .collect(),
                    None => Vec::new(),
                };
                let target = match attr.name.as_str() {
                    "class" => &mut classes,
                    "id" => &mut ids,
                    _ => continue,
                };
                for w in words.iter().flat_map(|v| v.split_whitespace()) {
                    target.insert(w.to_string());
                }
            }
        }
    }

    let mut push = |label: String, kind: CompletionItemKind, detail: &str| {
        items.push(CompletionItem {
            label,
            kind: Some(kind),
            detail: Some(detail.to_string()),
            ..Default::default()
        });
    };
    for c in classes {
        push(
            format!(".{c}"),
            CompletionItemKind::CLASS,
            "a class this component's view renders",
        );
    }
    for id in ids {
        push(
            format!("#{id}"),
            CompletionItemKind::REFERENCE,
            "an id this component's view renders",
        );
    }
    for t in tags {
        push(
            t,
            CompletionItemKind::KEYWORD,
            "an element this component's view renders",
        );
    }
    items.push(CompletionItem {
        label: ":global(…)".to_string(),
        kind: Some(CompletionItemKind::SNIPPET),
        detail: Some("match outside this component — the one way out of the scope".to_string()),
        insert_text: Some(":global(${1:selector})".to_string()),
        insert_text_format: Some(lsp_types::InsertTextFormat::SNIPPET),
        ..Default::default()
    });
}

/// What a template hole can name: the page's own vocabulary, and the tags.
fn add_template_completions(doc: &DocumentState, items: &mut Vec<CompletionItem>) {
    for def in &doc.symbol_index.definitions {
        let kind = match def.kind {
            DefinitionKind::StateField => CompletionItemKind::PROPERTY,
            DefinitionKind::Action => CompletionItemKind::METHOD,
            DefinitionKind::Component => CompletionItemKind::CLASS,
            _ => continue,
        };
        items.push(CompletionItem {
            label: def.name.clone(),
            kind: Some(kind),
            detail: def.detail.clone(),
            ..Default::default()
        });
    }
    // The tags, from the editor's one tag table (`crate::template::TAGS`), which is held against
    // the template parser's own tag words.
    for (label, detail) in crate::template::TAGS.iter().copied() {
        items.push(CompletionItem {
            label: label.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some(detail.to_string()),
            ..Default::default()
        });
    }
}

fn add_expression_completions(doc: &DocumentState, items: &mut Vec<CompletionItem>) {
    // Add variables in scope
    add_identifier_completions(doc, items);

    // Add "a new Type" constructor
    for (name, typedef) in doc.type_registry.iter_types() {
        let type_name = doc.interner.resolve(*name);
        if matches!(
            typedef,
            logicaffeine_language::analysis::TypeDef::Struct { .. }
        ) {
            items.push(CompletionItem {
                label: format!("a new {}", type_name),
                kind: Some(CompletionItemKind::CONSTRUCTOR),
                detail: Some(format!("Create a new {} instance", type_name)),
                ..Default::default()
            });
        }
    }
}

fn add_type_completions(doc: &DocumentState, items: &mut Vec<CompletionItem>) {
    // Primitive types — each teaches from the shared lesson table.
    let primitives = [
        "Int", "Nat", "Text", "Bool", "Float", "Unit", "Char", "Byte",
    ];
    for prim in primitives {
        let lesson = logicaffeine_language::teach::doc_for_primitive(prim)
            .expect("every offered primitive is taught");
        items.push(CompletionItem {
            label: prim.to_string(),
            kind: Some(CompletionItemKind::TYPE_PARAMETER),
            detail: Some(lesson.what.to_string()),
            documentation: Some(crate::teach_md::completion_docs(lesson)),
            ..Default::default()
        });
    }

    // Generic types — same table.
    let generics = ["List", "Seq", "Map", "Set", "Option", "Result"];
    for gen in generics {
        let lesson = logicaffeine_language::teach::doc_for_primitive(gen)
            .expect("every offered generic is taught");
        items.push(CompletionItem {
            label: gen.to_string(),
            kind: Some(CompletionItemKind::CLASS),
            detail: Some(lesson.what.to_string()),
            documentation: Some(crate::teach_md::completion_docs(lesson)),
            ..Default::default()
        });
    }

    // User-defined types from registry
    for (name, _typedef) in doc.type_registry.iter_types() {
        let type_name = doc.interner.resolve(*name);
        items.push(CompletionItem {
            label: type_name.to_string(),
            kind: Some(CompletionItemKind::CLASS),
            detail: Some("User-defined type".to_string()),
            ..Default::default()
        });
    }
}

fn add_field_completions(doc: &DocumentState, offset: usize, items: &mut Vec<CompletionItem>) {
    // Try to resolve the type of the object before the possessive
    if let Some(type_name) = resolve_possessive_type(doc, offset) {
        // Only offer fields from the resolved type
        for (name, typedef) in doc.type_registry.iter_types() {
            let tname = doc.interner.resolve(*name);
            if tname != type_name {
                continue;
            }
            if let logicaffeine_language::analysis::TypeDef::Struct { fields, .. } = typedef {
                for field in fields {
                    let field_name = doc.interner.resolve(field.name);
                    items.push(CompletionItem {
                        label: field_name.to_string(),
                        kind: Some(CompletionItemKind::FIELD),
                        detail: Some(format!(
                            "{}: {}",
                            field_name,
                            crate::hover::format_field_type(&field.ty, &doc.interner)
                        )),
                        ..Default::default()
                    });
                }
            }
        }
    }

    // Fall back: if no fields were added, show all fields from all structs
    if items
        .iter()
        .all(|i| i.kind != Some(CompletionItemKind::FIELD))
    {
        for (_name, typedef) in doc.type_registry.iter_types() {
            if let logicaffeine_language::analysis::TypeDef::Struct { fields, .. } = typedef {
                for field in fields {
                    let field_name = doc.interner.resolve(field.name);
                    items.push(CompletionItem {
                        label: field_name.to_string(),
                        kind: Some(CompletionItemKind::FIELD),
                        detail: Some(format!("{}", field_name)),
                        ..Default::default()
                    });
                }
            }
        }
    }
}

/// Look back from the possessive token to find the object's type name.
fn resolve_possessive_type(doc: &DocumentState, offset: usize) -> Option<String> {
    // Find the token right before the possessive
    let prev_token = doc
        .tokens
        .iter()
        .rev()
        .find(|t| t.span.end <= offset && !matches!(t.kind, TokenType::Possessive))?;
    let var_name = doc.source.get(prev_token.span.start..prev_token.span.end)?;

    // Look up the variable's definition to find its type
    let defs = doc.symbol_index.definitions_of(var_name);
    let def = defs.first()?;

    // Extract type from detail string like "Let x: Point" or "Let x: Point: inferred"
    let detail = def.detail.as_ref()?;
    // Match pattern "Let name: TypeName" or "name: TypeName"
    let after_colon = detail.rsplit_once(": ")?;
    let type_name = after_colon.1.trim();
    // Strip trailing ": inferred" or other suffixes
    let type_name = type_name.split(':').next().unwrap_or(type_name).trim();
    if type_name.is_empty() || type_name == "inferred" {
        return None;
    }
    Some(type_name.to_string())
}

fn add_variant_completions(doc: &DocumentState, items: &mut Vec<CompletionItem>) {
    // Fall back to showing all variants from all enums (type-aware filtering
    // would require resolving the Inspect target, which we can do in the future)
    for (_name, typedef) in doc.type_registry.iter_types() {
        if let logicaffeine_language::analysis::TypeDef::Enum { variants, .. } = typedef {
            for variant in variants {
                let variant_name = doc.interner.resolve(variant.name);
                items.push(CompletionItem {
                    label: variant_name.to_string(),
                    kind: Some(CompletionItemKind::ENUM_MEMBER),
                    ..Default::default()
                });
            }
        }
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/completion/tests.rs"]
mod tests;

fn add_identifier_completions(doc: &DocumentState, items: &mut Vec<CompletionItem>) {
    for def in &doc.symbol_index.definitions {
        let kind = match def.kind {
            DefinitionKind::Variable => CompletionItemKind::VARIABLE,
            DefinitionKind::Function => CompletionItemKind::FUNCTION,
            DefinitionKind::Struct => CompletionItemKind::CLASS,
            DefinitionKind::Enum => CompletionItemKind::ENUM,
            DefinitionKind::Field => CompletionItemKind::FIELD,
            DefinitionKind::Parameter => CompletionItemKind::VARIABLE,
            DefinitionKind::Block => CompletionItemKind::MODULE,
            DefinitionKind::Variant => CompletionItemKind::ENUM_MEMBER,
            DefinitionKind::Theorem => CompletionItemKind::CLASS,
            DefinitionKind::Component => CompletionItemKind::CLASS,
            DefinitionKind::Action => CompletionItemKind::METHOD,
            DefinitionKind::StateField => CompletionItemKind::PROPERTY,
        };

        // Check ownership state for moved variables
        let is_moved = doc.ownership_states.get(&def.name).map_or(false, |state| {
            matches!(state, VarState::Moved | VarState::MaybeMoved(_))
        });

        let mut detail = def.detail.clone();
        let mut tags = None;
        let mut deprecated = None;
        if is_moved {
            deprecated = Some(true);
            tags = Some(vec![CompletionItemTag::DEPRECATED]);
            detail = Some(format!(
                "{} (moved)",
                detail.as_deref().unwrap_or(&def.name)
            ));
        }

        items.push(CompletionItem {
            label: def.name.clone(),
            kind: Some(kind),
            detail,
            documentation: def.doc.clone().map(markdown_docs),
            deprecated,
            tags,
            ..Default::default()
        });
    }
}

/// Stdlib prelude names, documented from their literate `## Note`s.
/// Declarer wins: a local definition of the same name suppresses the
/// stdlib entry, mirroring the loader's auto-import rule.
fn add_stdlib_completions(doc: &DocumentState, items: &mut Vec<CompletionItem>) {
    for (name, entry) in crate::stdlib_docs::all() {
        if doc.symbol_index.name_to_defs.contains_key(name) {
            continue;
        }
        items.push(CompletionItem {
            label: name.to_string(),
            kind: Some(if entry.is_type {
                CompletionItemKind::CLASS
            } else {
                CompletionItemKind::FUNCTION
            }),
            detail: Some(entry.signature.trim_start_matches("## ").to_string()),
            documentation: Some(lsp_types::Documentation::MarkupContent(
                lsp_types::MarkupContent {
                    kind: lsp_types::MarkupKind::Markdown,
                    value: crate::stdlib_docs::hover_md(name, entry),
                },
            )),
            ..Default::default()
        });
    }
}

/// Plain prose as markdown completion documentation.
fn markdown_docs(value: String) -> lsp_types::Documentation {
    lsp_types::Documentation::MarkupContent(lsp_types::MarkupContent {
        kind: lsp_types::MarkupKind::Markdown,
        value,
    })
}
