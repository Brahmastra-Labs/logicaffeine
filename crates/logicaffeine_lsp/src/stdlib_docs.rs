//! The stdlib teaching registry: every prelude definition's literate
//! documentation, read once from the RAW embedded module sources
//! (`loader::prelude_module_sources` — Notes included) through
//! `teach::extract_literate_docs`. Hover, completion, and signature help
//! fall back here when a name has no local definition, so `md5` teaches in
//! the editor exactly what its `## Note` says in the source.

use std::collections::HashMap;
use std::sync::OnceLock;

use logicaffeine_language::teach::extract_literate_docs;

/// One stdlib definition's teaching surface.
pub struct StdlibDoc {
    /// The full `##` header (or `## Definition` body type line), verbatim.
    pub signature: String,
    /// The SUMMARY of the `## Note` directly above the definition: its first paragraph — one or
    /// two short sentences, the line hover and completion lead with (the rustdoc convention).
    /// `stdlib_teach_lock` holds every summary to ≤ 200 chars of plain prose.
    pub doc: Option<String>,
    /// The rest of the `## Note` (every paragraph after the first), rendered under the summary in
    /// hover — the protocol tables, edge cases, and citations that make a note long.
    pub details: Option<String>,
    /// True for type definitions (`## A … has/is`), false for functions.
    pub is_type: bool,
}

/// Split a `## Note` body into its summary paragraph and the remaining paragraphs.
fn split_summary(note: Option<String>) -> (Option<String>, Option<String>) {
    let Some(note) = note else {
        return (None, None);
    };
    let note = note.trim();
    match note.find("\n\n") {
        Some(cut) => {
            let details = note[cut..].trim();
            (
                Some(note[..cut].trim().to_string()),
                (!details.is_empty()).then(|| details.to_string()),
            )
        }
        None => (Some(note.to_string()), None),
    }
}

fn registry() -> &'static HashMap<String, StdlibDoc> {
    static DOCS: OnceLock<HashMap<String, StdlibDoc>> = OnceLock::new();
    DOCS.get_or_init(|| {
        let mut map = HashMap::new();
        for src in logicaffeine_compile::loader::prelude_module_sources() {
            for lit in extract_literate_docs(src) {
                let is_type = !lit.signature.contains("## To");
                let (doc, details) = split_summary(lit.doc);
                map.entry(lit.name).or_insert(StdlibDoc {
                    signature: lit.signature,
                    doc,
                    details,
                    is_type,
                });
            }
        }
        // Enum VARIANTS teach through their enum: hovering `Ok` shows the
        // variant's own line as the signature and Result's note as the prose.
        let mut variants: Vec<(String, StdlibDoc)> = Vec::new();
        for src in logicaffeine_compile::loader::prelude_module_sources() {
            let mut current_enum: Option<String> = None;
            for line in src.lines() {
                let t = line.trim();
                let article = t.strip_prefix("A ").or_else(|| t.strip_prefix("An "));
                if !line.starts_with(' ') {
                    // An enum header may carry a markdown `## ` prefix (`## A Json is either:`) or be
                    // bare (`A Result … is either:`); both name the enum whose indented variants follow.
                    let header = t.trim_start_matches('#').trim_start();
                    let header_article = header
                        .strip_prefix("A ")
                        .or_else(|| header.strip_prefix("An "));
                    current_enum = match header_article {
                        Some(rest) if header.ends_with("is either:") => {
                            rest.split_whitespace().next().map(|w| w.to_string())
                        }
                        _ => None,
                    };
                    continue;
                }
                if let (Some(enum_name), Some(rest)) = (&current_enum, article) {
                    if let Some(name) = rest
                        .split(|c: char| c == '(' || c == '.' || c.is_whitespace())
                        .next()
                        .filter(|w| w.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
                    {
                        let (doc, details) = map
                            .get(enum_name.as_str())
                            .map(|d: &StdlibDoc| (d.doc.clone(), d.details.clone()))
                            .unwrap_or((None, None));
                        variants.push((
                            name.to_string(),
                            StdlibDoc {
                                signature: t.to_string(),
                                doc,
                                details,
                                is_type: true,
                            },
                        ));
                    }
                }
            }
        }
        for (name, doc) in variants {
            map.entry(name).or_insert(doc);
        }
        map
    })
}

/// The teaching surface for a stdlib prelude name, if it is one.
pub fn stdlib_doc(name: &str) -> Option<&'static StdlibDoc> {
    registry().get(name)
}

/// Every stdlib name with its doc, name-sorted (completion feed).
pub fn all() -> Vec<(&'static str, &'static StdlibDoc)> {
    let mut entries: Vec<(&'static str, &'static StdlibDoc)> = registry()
        .iter()
        .map(|(name, doc)| (name.as_str(), doc))
        .collect();
    entries.sort_by_key(|(name, _)| *name);
    entries
}

/// Hover markdown for a stdlib name: the signature as code, then the prose.
pub fn hover_md(name: &str, entry: &StdlibDoc) -> String {
    let mut md = format!(
        "**{name}** — standard library\n\n```\n{}\n```",
        entry.signature
    );
    if let Some(doc) = &entry.doc {
        md.push_str("\n\n");
        md.push_str(doc);
    }
    if let Some(details) = &entry.details {
        md.push_str("\n\n");
        md.push_str(details);
    }
    md
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/stdlib_docs/tests.rs"]
mod tests;
