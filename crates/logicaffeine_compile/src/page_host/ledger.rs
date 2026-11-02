//! `lgx_known_divergences.txt` — the shrink-only record of every known tier divergence and
//! refusal, one file for every pair of tiers the locks compare. A row's TAG names its pair
//! ([`LedgerTier`]): plain rows are `lowered_vm_tier_lock`'s (the reference against the lowered VM),
//! `browser:` rows `browser_tier_lock`'s (against a real Chrome). ONE reader, so the locks cannot
//! drift on what a row means, which lock a row belongs to, or when a row counts as genuinely stale.

use std::collections::{BTreeMap, BTreeSet};

/// Which comparison a ledger row records — named on the row by its tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LedgerTier {
    /// No tag: the reference against the page's lowered LOGOS on the bytecode VM.
    LoweredVm,
    /// `browser:` — the reference against the page in a real Chrome.
    Browser,
}

impl LedgerTier {
    /// Every tier a row can belong to.
    pub const ALL: [LedgerTier; 2] = [LedgerTier::LoweredVm, LedgerTier::Browser];

    /// The tag a row of this tier carries before its axis (`browser` in `browser:document`), or
    /// `None` for the one untagged tier.
    pub fn tag(self) -> Option<&'static str> {
        match self {
            LedgerTier::LoweredVm => None,
            LedgerTier::Browser => Some("browser"),
        }
    }
}

/// One parsed ledger line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerRow {
    pub scenario: String,
    /// The comparison this row belongs to, read from its tag.
    pub tier: LedgerTier,
    /// The axis name, or `refused` — the tag, if any, already stripped.
    pub kind: String,
    pub statement: usize,
    pub cause: String,
}

/// Parse the ledger's text into every row: three `|`-separated fields
/// (`scenario | [tag:]kind @ statement | cause`), at most one tag and only a [`LedgerTier`]'s own, a
/// parseable statement number, a non-empty cause, and a `kind` that is `refused` or a real
/// [`super::Axis`] name. Panics naming the exact line on a malformed row — this runs at test time,
/// checking an invariant, never a runtime path.
pub fn parse_ledger(text: &str) -> Vec<LedgerRow> {
    let mut rows = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split(" | ").collect();
        assert_eq!(
            parts.len(),
            3,
            "ledger line {}: `scenario | kind @ statement | cause`: {line}",
            n + 1
        );
        let (kind_field, at) = parts[1]
            .split_once(" @ ")
            .unwrap_or_else(|| panic!("ledger line {}: `kind @ statement`: {line}", n + 1));
        let statement: usize = at
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("ledger line {}: a statement number: {line}", n + 1));
        let kind_field = kind_field.trim();
        let (tier, kind) = match kind_field.split_once(':') {
            None => (LedgerTier::LoweredVm, kind_field.to_string()),
            Some((tag, axis)) => {
                let tier = LedgerTier::ALL
                    .into_iter()
                    .find(|t| t.tag() == Some(tag))
                    .unwrap_or_else(|| {
                        panic!(
                            "ledger line {}: `{tag}:` is not a ledger tag: {line}",
                            n + 1
                        )
                    });
                assert!(
                    !axis.contains(':'),
                    "ledger line {}: `{kind_field}` carries more than one tag",
                    n + 1
                );
                (tier, axis.to_string())
            }
        };
        assert!(
            kind == "refused" || super::Axis::from_name(&kind).is_some(),
            "ledger line {}: `{kind_field}` names neither `refused` nor a real axis",
            n + 1
        );
        let cause = parts[2].trim();
        assert!(
            !cause.is_empty(),
            "ledger line {}: every entry says why: {line}",
            n + 1
        );
        rows.push(LedgerRow {
            scenario: parts[0].trim().to_string(),
            tier,
            kind,
            statement,
            cause: cause.to_string(),
        });
    }
    rows
}

/// `rows` of one tier, collected into the `(scenario, kind, statement) -> cause` map that tier's
/// lock compares what it found against. Panics on a duplicate key WITHIN the tier — two tiers'
/// rows sharing a scenario/kind/statement are not a collision, since each lock reads only its own.
pub fn ledger_map(
    rows: &[LedgerRow],
    tier: LedgerTier,
) -> BTreeMap<(String, String, usize), String> {
    let mut out = BTreeMap::new();
    for row in rows.iter().filter(|r| r.tier == tier) {
        let key = (row.scenario.clone(), row.kind.clone(), row.statement);
        assert!(
            out.insert(key.clone(), row.cause.clone()).is_none(),
            "duplicate ledger entry {key:?}"
        );
    }
    out
}

/// The ledger rows a run's OWN results say have genuinely closed — narrower than "not in
/// `found`": a row whose scenario never ran far enough to be checked at all (its page did not
/// parse, or the scenario itself refused at statement 0, before any axis was ever compared) is not
/// evidence the divergence closed, only that THIS run could not see it. Left alone until a run
/// that actually reaches it says otherwise, rather than deleting a real, still-true cause out from
/// under whoever needs it back the moment the page parses or the scenario runs again.
///
/// `unparsed_pages` names each page (by the id its OWN scenarios are prefixed with, `<id>::…`)
/// whose source did not parse at all; `refused_at_zero` names each scenario (`<page id>::<test>`)
/// refused at statement 0. A scenario refused at a LATER statement is not exempted at all — every
/// statement before that refusal DID run, so a row at an earlier statement is checked normally.
///
/// Shared by every tier's lock (over that tier's own [`ledger_map`]) so the SAME rule protects all
/// of them: an unparsed page or an early refusal can mark no lock's rows stale.
pub fn stale_rows(
    known: &BTreeMap<(String, String, usize), String>,
    found: &BTreeMap<(String, String, usize), String>,
    unparsed_pages: &[String],
    refused_at_zero: &BTreeSet<String>,
) -> Vec<(String, String, usize)> {
    known
        .keys()
        .filter(|k| !found.contains_key(*k))
        .filter(|(id, ..)| {
            !refused_at_zero.contains(id)
                && !unparsed_pages.iter().any(|p| {
                    id.strip_prefix(p.as_str())
                        .is_some_and(|rest| rest.starts_with("::"))
                })
        })
        .cloned()
        .collect()
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/page_host/ledger/stale_rows_tests.rs"]
mod stale_rows_tests;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/page_host/ledger/tag_tests.rs"]
mod tag_tests;
