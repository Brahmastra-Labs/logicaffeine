//! The stdlib documentation ratchet: every name the prelude exports must
//! teach. A prelude definition without a `## Note` above it fails here until
//! someone writes the doc — or records the name in the allowlist WITH a
//! reason. Both directions: an allowlisted name that gains a doc must leave
//! the list.

use logicaffeine_compile::loader::prelude_vocabulary;
use logicaffeine_lsp::stdlib_docs::stdlib_doc;

/// Names deliberately undocumented, each with its reason. Currently empty —
/// every prelude definition carries a literate `## Note`.
const UNDOCUMENTED: &[(&str, &str)] = &[];

#[test]
fn every_prelude_name_teaches_or_is_excused() {
    for name in prelude_vocabulary() {
        let entry = stdlib_doc(&name)
            .unwrap_or_else(|| panic!("{name}: prelude name missing from the stdlib registry"));
        let excused = UNDOCUMENTED.iter().any(|(n, _)| *n == name);
        match &entry.doc {
            Some(doc) => {
                assert!(
                    !excused,
                    "{name}: documented but still allowlisted — remove it from UNDOCUMENTED"
                );
                assert!(
                    doc.ends_with('.') || doc.ends_with(')'),
                    "{name}: docs are prose sentences: {doc:?}"
                );
                assert!(
                    doc.chars().count() <= 200,
                    "{name}: keep stdlib docs to one or two short sentences (clear and easy), \
                     got {} chars",
                    doc.chars().count()
                );
            }
            None => {
                assert!(
                    excused,
                    "{name}: prelude definition has no `## Note` doc — write one above its \
                     header (or allowlist with a reason)"
                );
            }
        }
    }
    for (name, reason) in UNDOCUMENTED {
        assert!(!reason.is_empty(), "{name}: excuses record their reason");
        assert!(
            prelude_vocabulary().iter().any(|n| n == name),
            "{name}: stale excuse — not a prelude name"
        );
    }
}

/// Every definition in every prelude module — not just the trigger
/// vocabulary — must teach. The trigger list is the loader's concern; hover
/// reaches EVERY `## To`/`## A` name, so every one needs its Note.
#[test]
fn every_prelude_definition_carries_a_note() {
    let mut undocumented = Vec::new();
    for src in logicaffeine_compile::loader::prelude_module_sources() {
        for lit in logicaffeine_language::teach::extract_literate_docs(src) {
            let excused = UNDOCUMENTED.iter().any(|(n, _)| *n == lit.name);
            if lit.doc.is_none() && !excused {
                undocumented.push(format!("{} ({})", lit.name, lit.signature));
            }
        }
    }
    assert!(
        undocumented.is_empty(),
        "stdlib definitions without a `## Note` doc:\n{}",
        undocumented.join("\n")
    );
}
