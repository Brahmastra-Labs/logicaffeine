//! Every number a machine or a part carries names where it comes from.

use logicaffeine_compile::spatial::rdu_chip::Provenance;

/// Holds each fact to its provenance: a published or derived fact cites one of the entry's sources, a measured one
/// names the run ledger it was read from, and a modeled one says what gap it fills.
pub fn assert_cited(name: &str, facts: &[(String, Provenance, &'static str)], sources: &[&str]) {
    assert!(!facts.is_empty(), "{name}: no facts");
    for (path, provenance, cite) in facts {
        assert!(!cite.trim().is_empty(), "{name} `{path}`: an empty citation");
        match provenance {
            Provenance::Published | Provenance::Derived => assert!(
                sources.iter().any(|s| cite.contains(s)),
                "{name} `{path}` is {provenance:?} but cites `{cite}`, which names none of its sources {sources:?}"
            ),
            Provenance::Measured => assert!(
                cite.contains("benchmarks/") && cite.contains(".json"),
                "{name} `{path}` is measured but cites `{cite}`, which names no run ledger"
            ),
            Provenance::Modeled => assert!(
                ["not public", "not published", "inherited", "not tabulated"].iter().any(|m| cite.contains(m)),
                "{name} `{path}` is modeled but its cite `{cite}` does not say what gap it fills"
            ),
        }
    }
}
