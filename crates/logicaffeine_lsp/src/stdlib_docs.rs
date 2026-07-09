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
    /// The `## Note` prose directly above the definition.
    pub doc: Option<String>,
    /// True for type definitions (`## A … has/is`), false for functions.
    pub is_type: bool,
}

fn registry() -> &'static HashMap<String, StdlibDoc> {
    static DOCS: OnceLock<HashMap<String, StdlibDoc>> = OnceLock::new();
    DOCS.get_or_init(|| {
        let mut map = HashMap::new();
        for src in logicaffeine_compile::loader::prelude_module_sources() {
            for lit in extract_literate_docs(src) {
                let is_type = !lit.signature.contains("## To");
                map.entry(lit.name).or_insert(StdlibDoc {
                    signature: lit.signature,
                    doc: lit.doc,
                    is_type,
                });
            }
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
    let mut entries: Vec<(&'static str, &'static StdlibDoc)> =
        registry().iter().map(|(name, doc)| (name.as_str(), doc)).collect();
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
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_covers_the_prelude_vocabulary() {
        for name in logicaffeine_compile::loader::prelude_vocabulary() {
            assert!(
                stdlib_doc(&name).is_some(),
                "{name}: in prelude_vocabulary but missing from the stdlib docs registry"
            );
        }
    }

    #[test]
    fn md5_teaches_from_its_literate_note() {
        let entry = stdlib_doc("md5").expect("md5 is stdlib");
        assert!(entry.signature.contains("md5"), "{}", entry.signature);
        assert!(
            entry.doc.as_deref().is_some_and(|d| d.contains("MD5")),
            "md5's Note must teach: {:?}",
            entry.doc
        );
        assert!(!entry.is_type);
    }

    #[test]
    fn definition_body_types_are_typed_entries() {
        let entry = stdlib_doc("Message").expect("net.md defines Message");
        assert!(entry.is_type);
        assert!(entry.doc.is_some(), "the Definition block's Note documents Message");
    }

    #[test]
    fn hover_md_carries_signature_and_prose() {
        let entry = stdlib_doc("read").expect("file.lg defines read");
        let md = hover_md("read", entry);
        assert!(md.contains("standard library"), "{md}");
        assert!(md.contains("## To native read"), "{md}");
        assert!(md.contains("Reads a whole file"), "{md}");
    }
}
