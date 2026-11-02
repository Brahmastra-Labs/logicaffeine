//! `largo explain` — offline lookup of diagnostic codes and constructs.
//!
//! One verb, two namespaces, resolved in order: `logos-diagnostic/1` error
//! codes from the canonical catalog (`largo explain zero-index`), then
//! language constructs from the teach lessons (`largo explain Let`). Every
//! diagnostic an agent reads in `check --json` or MCP output can be looked
//! up here without a network or a project.

use anstyle::AnsiColor;
use logicaffeine_language::diagnostic::{catalog, CatalogEntry};
use logicaffeine_language::suggest::find_similar;
use logicaffeine_language::teach::{doc_for_primitive, docs_for_word, ConstructDoc};

use crate::ui::CliError;

/// Handle `largo explain <topic> [--json]`.
pub(crate) fn cmd_explain(topic: &str, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    // Namespace 1: diagnostic codes (a shared code like `type-mismatch`
    // matches every variant that carries it).
    let code_entries: Vec<CatalogEntry> = catalog()
        .into_iter()
        .filter(|entry| entry.code == topic)
        .collect();
    if !code_entries.is_empty() {
        if json {
            println!("{}", render_code_json(topic, &code_entries));
        } else {
            render_code_human(&code_entries);
        }
        return Ok(());
    }

    // Namespace 2: constructs and primitive types from the teach lessons.
    let mut lessons: Vec<&'static ConstructDoc> = docs_for_word(topic);
    if lessons.is_empty() {
        if let Some(doc) = doc_for_primitive(topic) {
            lessons.push(doc);
        }
    }
    if !lessons.is_empty() {
        if json {
            println!("{}", render_lessons_json(topic, &lessons));
        } else {
            for doc in &lessons {
                render_lesson_human(doc);
            }
        }
        return Ok(());
    }

    // Neither namespace knows the topic: suggest the nearest real one.
    let entries = catalog();
    let mut candidates: Vec<&str> = entries.iter().map(|entry| entry.code).collect();
    candidates.extend(
        logicaffeine_language::teach::ALL_DOCS
            .iter()
            .map(|doc| doc.name),
    );
    let hint = match find_similar(topic, &candidates, 2) {
        Some(nearest) => format!("did you mean '{nearest}'?"),
        None => "try an error code from `largo check --json` (e.g. 'zero-index') or a \
                 construct name (e.g. 'Let')"
            .to_string(),
    };
    Err(CliError::with_hint(format!("nothing is called '{topic}'"), hint).into())
}

fn render_code_human(entries: &[CatalogEntry]) {
    let bold = AnsiColor::Cyan.on_default().bold();
    let first = &entries[0];
    anstream::println!("{bold}{}{bold:#} ({})", first.code, first.severity);
    if let Some(quickfix) = first.quickfix {
        anstream::println!("fix: {quickfix}");
    }
    if let Some(docs) = &first.docs {
        anstream::println!("guide: {docs}");
    }
    for entry in entries {
        anstream::println!("\nexample ({}):\n  {}", entry.variant, entry.headline);
        anstream::println!("\n{}", entry.explanation);
    }
}

fn render_code_json(topic: &str, entries: &[CatalogEntry]) -> String {
    let items: Vec<serde_json::Value> = entries
        .iter()
        .map(|entry| {
            serde_json::json!({
                "code": entry.code,
                "severity": entry.severity,
                "variant": entry.variant,
                "headline": entry.headline,
                "explanation": entry.explanation,
                "quickfix": entry.quickfix,
                "docs": entry.docs,
            })
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "schema": "logos-explain/1",
        "topic": topic,
        "kind": "error-code",
        "entries": items,
    }))
    .expect("explain serializes")
}

fn render_lesson_human(doc: &ConstructDoc) {
    let bold = AnsiColor::Cyan.on_default().bold();
    anstream::println!("{bold}{}{bold:#} — {}", doc.name, doc.what);
    anstream::println!("\nExample:\n");
    for line in doc.example.lines() {
        anstream::println!("    {line}");
    }
    anstream::println!("\n{}", doc.question_or_tip);
}

fn render_lessons_json(topic: &str, lessons: &[&'static ConstructDoc]) -> String {
    let items: Vec<serde_json::Value> = lessons
        .iter()
        .map(|doc| {
            serde_json::json!({
                "name": doc.name,
                "what": doc.what,
                "example": doc.example,
                "tip": doc.question_or_tip,
            })
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "schema": "logos-explain/1",
        "topic": topic,
        "kind": "construct",
        "entries": items,
    }))
    .expect("explain serializes")
}
