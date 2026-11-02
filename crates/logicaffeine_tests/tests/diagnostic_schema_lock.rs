//! Schema lock for the canonical diagnostic surface (`logos-diagnostic/1`).
//!
//! Parses the `ParseErrorKind` enum straight out of the language crate's
//! source and asserts the diagnostic catalog covers every variant — a new
//! error kind cannot ship without a stable code, severity, headline, and
//! rendered catalog entry. Same file-reading ratchet style as
//! `version_lock.rs`: the source of truth is the checked-in text itself.

use std::collections::BTreeSet;

/// Variant names of `pub enum ParseErrorKind`, parsed from `error.rs`.
fn parse_error_kind_variants() -> BTreeSet<String> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../logicaffeine_language/src/error.rs"
    );
    let source = std::fs::read_to_string(path).expect("read logicaffeine_language error.rs");
    let start = source
        .find("pub enum ParseErrorKind {")
        .expect("error.rs must define pub enum ParseErrorKind");

    let mut depth = 0usize;
    let mut variants = BTreeSet::new();
    for line in source[start..].lines() {
        let trimmed = line.trim();
        if depth == 1 {
            if let Some(first) = trimmed.chars().next() {
                if first.is_ascii_uppercase() {
                    let name: String = trimmed
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric())
                        .collect();
                    if !name.is_empty() {
                        variants.insert(name);
                    }
                }
            }
        }
        depth += line.matches('{').count();
        depth = depth.saturating_sub(line.matches('}').count());
        if depth == 0 && !variants.is_empty() {
            break;
        }
    }
    variants
}

#[test]
fn catalog_covers_every_parse_error_kind() {
    let parsed = parse_error_kind_variants();
    assert!(
        parsed.len() >= 39,
        "sanity: the enum parser found only {} variants: {parsed:?}",
        parsed.len()
    );

    let catalog: BTreeSet<String> = logicaffeine_language::diagnostic::catalog()
        .into_iter()
        .map(|entry| entry.variant.to_string())
        .collect();

    assert_eq!(
        parsed, catalog,
        "every ParseErrorKind variant must have exactly one catalog entry \
         (missing = new variant without a diagnostic decision; extra = ghost \
         entry for a removed variant)"
    );
}
