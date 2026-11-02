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
    // Collect EVERY violation, then report the whole set at once. A lock that
    // panics on the first offender turns one full-suite window into N: you fix
    // one, re-run, discover the next. The complete list fixes all in one pass.
    let mut violations: Vec<String> = Vec::new();
    for name in prelude_vocabulary() {
        let Some(entry) = stdlib_doc(&name) else {
            violations.push(format!(
                "{name}: prelude name missing from the stdlib registry"
            ));
            continue;
        };
        let excused = UNDOCUMENTED.iter().any(|(n, _)| *n == name);
        match &entry.doc {
            Some(doc) => {
                if excused {
                    violations.push(format!(
                        "{name}: documented but still allowlisted — remove it from UNDOCUMENTED"
                    ));
                }
                if !(doc.ends_with('.') || doc.ends_with(')')) {
                    violations.push(format!(
                        "{name}: docs are prose sentences ending in '.'/')' : {doc:?}"
                    ));
                }
                let len = doc.chars().count();
                if len > 200 {
                    violations.push(format!(
                        "{name}: keep stdlib docs to one or two short sentences (clear and easy), \
                         got {len} chars"
                    ));
                }
            }
            None => {
                if !excused {
                    violations.push(format!(
                        "{name}: prelude definition has no `## Note` doc — write one above its \
                         header (or allowlist with a reason)"
                    ));
                }
            }
        }
    }
    for (name, reason) in UNDOCUMENTED {
        if reason.is_empty() {
            violations.push(format!("{name}: excuses record their reason"));
        }
        if !prelude_vocabulary().iter().any(|n| n == name) {
            violations.push(format!("{name}: stale excuse — not a prelude name"));
        }
    }
    assert!(
        violations.is_empty(),
        "stdlib_teach_lock: {} name(s) fail the documentation ratchet:\n  {}",
        violations.len(),
        violations.join("\n  ")
    );
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
