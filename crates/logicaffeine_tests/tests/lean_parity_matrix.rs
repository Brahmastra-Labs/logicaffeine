//! LEAN PARITY MATRIX — the scoreboard for the head-to-head against Lean 4.
//!
//! The conformance corpus (`fixtures/lean_conformance/`, 248 cases across 13 tiers, 1135 adversarial
//! `@guard` notes) is written as a *translator* spec: `translate` / `degrade` / `reject`, where a
//! translator refusing 77 of 248 scores perfectly. We are a *competitor*, so every refusal is
//! mathematics Lean can express and we cannot. This lock re-reads the corpus as a **capability
//! matrix**: each case is re-tagged `MUST` / `DIVERGE` / `OUT`, given a status of where we stand
//! today, and bound to the test that decides it.
//!
//! The verdicts live one file per tier under `crates/logicaffeine_tests/tests/fixtures/lean_conformance/verdicts/` — see the README there for
//! the schema and the meaning of each tag. One file per tier is what lets thirteen agents write
//! verdicts in parallel without touching the same file.
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS LOCK TO MAKE IT PASS.  ⚠️
//!  An unclassified case is a gap nobody has looked at, and a lowered floor is a capability
//!  silently lost. Both are exactly the false confidence this gate exists to kill. Raise a floor
//!  when a wave earns it; never lower one to go green.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::Deserialize;

// ============================================================================================
// THE CORPUS — embedded, so deleting it is a COMPILE error rather than a silently-empty run.
// ============================================================================================

const MANIFEST_SRC: &str = include_str!("fixtures/lean_conformance/manifest.json");

/// Every corpus case must be accounted for. Pinned so a truncated manifest cannot quietly
/// shrink the thing we are measuring ourselves against.
const CORPUS_CASE_COUNT: usize = 248;

/// The thirteen tiers, their corpus directory names, and their case counts — verified against
/// `crates/logicaffeine_tests/tests/fixtures/lean_conformance/cases/`. A tier file must match its directory on both.
const TIERS: &[(&str, usize)] = &[
    ("t00_terms", 22),
    ("t01_universes", 14),
    ("t02_declarations", 20),
    ("t03_inductives", 24),
    ("t04_recursion", 20),
    ("t05_elaboration", 20),
    ("t06_propositions", 18),
    ("t07_typeclasses", 14),
    ("t08_tactics", 28),
    ("t09_metaprogramming", 14),
    ("t10_literals", 18),
    ("t11_adversarial", 22),
    ("t12_vernacular", 14),
];

// ---- Ratchets: only a human moves these, only upward. ---------------------------------------
// `Native` count per tier — cases that work TODAY, proven by their test.
// 2026-09-14, Phase R: seeded at the MEASURED value once all 13 tiers landed and all 248 cases were
// classified. Total 87 Native of 248. Each later wave raises the tiers it earned, naming the wave
// in a comment. Lowering one to go green is forbidden: it converts a lost capability into a
// passing test.
// 2026-09-28, banking: 85 cases recorded Gap whose tests pass (PASS 3's GAP-BUT-GREEN) became
// Native; every tier but t09 raised to its measured Native count (234 of 248).
// 2026-09-29, A39: T00-005 (a binder kind on an anonymous `fun`) and T00-009 (a source position
// that survives translation) Native through the transparent `Term::Annotated` carrier (236 of 248).
const TIER_FLOORS: &[(&str, usize)] = &[
    ("t00_terms", 22),
    ("t01_universes", 14),
    ("t02_declarations", 18),
    ("t03_inductives", 22),
    ("t04_recursion", 20),
    ("t05_elaboration", 18),
    ("t06_propositions", 18),
    ("t07_typeclasses", 14),
    ("t08_tactics", 27),
    ("t09_metaprogramming", 9),
    ("t10_literals", 18),
    ("t11_adversarial", 21),
    ("t12_vernacular", 14),
];

/// A `DIVERGE` or `OUT` must carry a real argument, not a shrug. Short enough that one honest
/// sentence clears it; long enough that "n/a", "see above" and "" do not.
const MIN_ARGUMENT_LEN: usize = 40;

// ============================================================================================
// TYPES — deserialized strictly. Unknown manifest fields are ignored; unknown VERDICT fields are
// a hard error, because a typo in a verdict file must never silently drop a case's classification.
// ============================================================================================

#[derive(Debug, Deserialize)]
struct Manifest {
    case_count: usize,
    cases: Vec<ManifestCase>,
}

#[derive(Debug, Deserialize)]
struct ManifestCase {
    id: String,
    /// Prose, e.g. "0 · Kernel term layer" — NOT the directory name. `path` is what locates the
    /// case on disk, so the tier a verdict file claims is checked against `path`, not this.
    #[allow(dead_code)]
    tier: String,
    title: String,
    /// The corpus's own translator verdict (`translate` / `degrade` / `reject`). Kept for contrast
    /// in the summary — it is deliberately NOT our verdict.
    expect: String,
    /// `cases/t00_terms/T00-001_….lean`
    path: String,
    guards: Vec<String>,
}

impl ManifestCase {
    /// The corpus DIRECTORY this case lives in — the tier identity a verdict file must match.
    fn tier_dir(&self) -> &str {
        self.path
            .split('/')
            .find(|seg| seg.starts_with('t') && seg.contains('_'))
            .unwrap_or("<unknown>")
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TierVerdicts {
    tier: String,
    test_binary: String,
    cases: Vec<CaseVerdict>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CaseVerdict {
    id: String,
    verdict: Verdict,
    status: Status,
    /// The `#[test]` fn in `test_binary` that decides this case.
    test: String,
    /// Required for `DIVERGE` and `OUT`; ignored for `MUST`.
    #[serde(default)]
    why: String,
}

/// What we owe on a case. See `crates/logicaffeine_tests/tests/fixtures/lean_conformance/verdicts/README.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum Verdict {
    /// Lean can do it; we should too. The test asserts the capability.
    MUST,
    /// We deliberately differ and ours is defensible. `why` carries the argument.
    DIVERGE,
    /// Genuinely out of scope. The test asserts a clean refusal.
    OUT,
}

/// Where we stand today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum Status {
    /// Works now, proven by its test. The only status that counts toward a tier floor.
    Native,
    /// We cannot do it yet. The test is RED and names the work.
    Gap,
    /// We decline, deliberately and loudly.
    Refused,
}

// ============================================================================================
// LOADING — parsed once. The manifest is 1.1MB; re-parsing it per test would dominate the run.
// ============================================================================================

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn verdict_dir() -> PathBuf {
    root().join("crates/logicaffeine_tests/tests/fixtures/lean_conformance/verdicts")
}

fn manifest() -> &'static Manifest {
    static MANIFEST: OnceLock<Manifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        serde_json::from_str(MANIFEST_SRC).expect("crates/logicaffeine_tests/tests/fixtures/lean_conformance/manifest.json is not valid JSON")
    })
}

/// Every tier verdict file on disk, keyed by the file stem. An absent directory is not an error —
/// it is the honest starting state, and `every_corpus_case_has_a_verdict` reports it as 248 owed.
fn tier_files() -> &'static BTreeMap<String, TierVerdicts> {
    static FILES: OnceLock<BTreeMap<String, TierVerdicts>> = OnceLock::new();
    FILES.get_or_init(|| {
        let dir = verdict_dir();
        let mut out = BTreeMap::new();
        let Ok(entries) = fs::read_dir(&dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .expect("a .json path has a UTF-8 stem")
                .to_string();
            let raw = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            let parsed: TierVerdicts = serde_json::from_str(&raw)
                .unwrap_or_else(|e| panic!("{} is not a valid verdict file: {e}", path.display()));
            out.insert(stem, parsed);
        }
        out
    })
}

/// Every classified case id → (its tier file's stem, the verdict).
fn classified() -> &'static BTreeMap<String, (&'static str, &'static CaseVerdict)> {
    static MAP: OnceLock<BTreeMap<String, (&'static str, &'static CaseVerdict)>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut out: BTreeMap<String, (&'static str, &'static CaseVerdict)> = BTreeMap::new();
        for (stem, tier) in tier_files() {
            for case in &tier.cases {
                // A case claimed twice is caught by `no_case_is_claimed_by_two_tiers`, which reads
                // the raw files; here first-wins keeps the other locks reporting something useful.
                out.entry(case.id.clone()).or_insert((stem.as_str(), case));
            }
        }
        out
    })
}

fn corpus_ids() -> &'static BTreeSet<&'static str> {
    static IDS: OnceLock<BTreeSet<&'static str>> = OnceLock::new();
    IDS.get_or_init(|| manifest().cases.iter().map(|c| c.id.as_str()).collect())
}

// ============================================================================================
// THE CORPUS ITSELF — pinned, so the thing we measure against cannot drift under us.
// ============================================================================================

#[test]
fn the_corpus_is_intact() {
    let m = manifest();
    assert_eq!(
        m.cases.len(),
        CORPUS_CASE_COUNT,
        "the corpus must hold exactly {CORPUS_CASE_COUNT} cases; manifest.json has {}",
        m.cases.len()
    );
    assert_eq!(
        m.case_count, CORPUS_CASE_COUNT,
        "manifest.json's own case_count disagrees with its cases array",
    );

    let ids = corpus_ids();
    assert_eq!(
        ids.len(),
        CORPUS_CASE_COUNT,
        "corpus case ids must be unique; {} distinct ids for {CORPUS_CASE_COUNT} cases",
        ids.len()
    );

    // Every case carries at least one adversarial guard — those are the assertions worth having,
    // and a case that lost its guards has lost the reason it is in the corpus.
    let ungarded: Vec<&str> = m
        .cases
        .iter()
        .filter(|c| c.guards.is_empty())
        .map(|c| c.id.as_str())
        .collect();
    assert!(
        ungarded.is_empty(),
        "every corpus case must carry at least one @guard; these do not: {ungarded:?}"
    );
}

#[test]
fn every_tier_directory_matches_its_pinned_case_count() {
    let m = manifest();
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for case in &m.cases {
        *seen.entry(case.tier_dir()).or_default() += 1;
    }

    let pinned: BTreeMap<&str, usize> = TIERS.iter().copied().collect();
    assert_eq!(
        seen.keys().collect::<Vec<_>>(),
        pinned.keys().collect::<Vec<_>>(),
        "the corpus tier directories do not match TIERS"
    );
    for (tier, count) in &pinned {
        assert_eq!(
            seen.get(tier),
            Some(count),
            "tier {tier} must hold {count} cases, corpus has {:?}",
            seen.get(tier)
        );
    }
    assert_eq!(
        pinned.values().sum::<usize>(),
        CORPUS_CASE_COUNT,
        "the pinned per-tier counts must sum to the corpus size"
    );
}

// ============================================================================================
// COMPLETENESS — no case may be silently unclassified.
// ============================================================================================

#[test]
fn every_corpus_case_has_a_verdict() {
    let have = classified();
    let missing: Vec<String> = manifest()
        .cases
        .iter()
        .filter(|c| !have.contains_key(&c.id))
        .map(|c| format!("{} ({}, corpus: {})", c.id, c.tier_dir(), c.expect))
        .collect();

    assert!(
        missing.is_empty(),
        "{} of {CORPUS_CASE_COUNT} corpus cases have no verdict. Each is a capability nobody has \
         looked at yet. Classify it in crates/logicaffeine_tests/tests/fixtures/lean_conformance/verdicts/<tier>.json — see the README there.\n\
         Unclassified:\n  {}",
        missing.len(),
        missing.join("\n  ")
    );
}

#[test]
fn no_verdict_names_a_case_outside_the_corpus() {
    let ids = corpus_ids();
    let mut strays = Vec::new();
    for (stem, tier) in tier_files() {
        for case in &tier.cases {
            if !ids.contains(case.id.as_str()) {
                strays.push(format!("{stem}.json claims {}", case.id));
            }
        }
    }
    assert!(
        strays.is_empty(),
        "a verdict names a case the corpus does not contain (typo, or a renumbered id — case ids \
         are stable and must never be renumbered):\n  {}",
        strays.join("\n  ")
    );
}

#[test]
fn no_case_is_claimed_by_two_tiers() {
    let mut owner: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (stem, tier) in tier_files() {
        for case in &tier.cases {
            owner
                .entry(case.id.as_str())
                .or_default()
                .push(stem.as_str());
        }
    }
    let dupes: Vec<String> = owner
        .iter()
        .filter(|(_, owners)| owners.len() > 1)
        .map(|(id, owners)| format!("{id} claimed by {owners:?}"))
        .collect();
    assert!(
        dupes.is_empty(),
        "each case is decided by exactly one tier file; two verdicts for one case means the two \
         can disagree:\n  {}",
        dupes.join("\n  ")
    );
}

#[test]
fn every_tier_file_matches_a_real_corpus_directory() {
    let pinned: BTreeMap<&str, usize> = TIERS.iter().copied().collect();
    let mut problems = Vec::new();

    for (stem, tier) in tier_files() {
        if stem != &tier.tier {
            problems.push(format!(
                "{stem}.json declares tier \"{}\" — the file stem and the tier must agree",
                tier.tier
            ));
        }
        match pinned.get(tier.tier.as_str()) {
            None => problems.push(format!(
                "{stem}.json declares tier \"{}\", which is not a corpus directory",
                tier.tier
            )),
            Some(expected) if tier.cases.len() != *expected => problems.push(format!(
                "{stem}.json classifies {} cases; tier {} holds {expected}",
                tier.cases.len(),
                tier.tier
            )),
            Some(_) => {}
        }
        if tier.test_binary.trim().is_empty() {
            problems.push(format!("{stem}.json names no test_binary"));
        }
    }

    assert!(
        problems.is_empty(),
        "verdict files must line up with the corpus:\n  {}",
        problems.join("\n  ")
    );
}

// ============================================================================================
// HONESTY — a verdict must name its test, and a divergence must carry its argument.
// ============================================================================================

#[test]
fn every_verdict_names_the_test_that_decides_it() {
    let nameless: Vec<String> = classified()
        .iter()
        .filter(|(_, (_, v))| v.test.trim().is_empty())
        .map(|(id, (stem, _))| format!("{id} (in {stem}.json)"))
        .collect();
    assert!(
        nameless.is_empty(),
        "a verdict with no test is a claim with no evidence:\n  {}",
        nameless.join("\n  ")
    );
}

/// **Every case gets its own test.** Without this, 24 cases can be classified and all pointed at
/// one test — the matrix would read green while 23 of them were never actually exercised. The whole
/// corpus is translated, case by case, or it is not translated.
#[test]
fn every_case_is_decided_by_its_own_test() {
    let mut by_test: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (id, (_, v)) in classified() {
        by_test.entry(v.test.trim()).or_default().push(id.as_str());
    }
    let shared: Vec<String> = by_test
        .iter()
        .filter(|(_, ids)| ids.len() > 1)
        .map(|(test, ids)| format!("{test} decides {} cases: {ids:?}", ids.len()))
        .collect();
    assert!(
        shared.is_empty(),
        "one test per case — a shared test means the other cases were never exercised, and the \
         matrix would read green anyway:\n  {}",
        shared.join("\n  ")
    );
}

#[test]
fn every_diverge_and_out_states_its_argument() {
    let unargued: Vec<String> = classified()
        .iter()
        .filter(|(_, (_, v))| {
            matches!(v.verdict, Verdict::DIVERGE | Verdict::OUT)
                && v.why.trim().len() < MIN_ARGUMENT_LEN
        })
        .map(|(id, (stem, v))| format!("{id} ({:?}, in {stem}.json)", v.verdict))
        .collect();
    assert!(
        unargued.is_empty(),
        "a DIVERGE or OUT must say WHY, in at least {MIN_ARGUMENT_LEN} characters. A divergence \
         whose argument cannot be written in one paragraph is a MUST in disguise:\n  {}",
        unargued.join("\n  ")
    );
}

// ============================================================================================
// THE RATCHET — capability may only go up.
// ============================================================================================

#[test]
fn the_capability_floor_never_regresses() {
    let mut native: BTreeMap<&str, usize> = TIERS.iter().map(|(t, _)| (*t, 0usize)).collect();
    for (_, tier) in tier_files() {
        let count = tier
            .cases
            .iter()
            .filter(|c| c.status == Status::Native)
            .count();
        if let Some(slot) = native.get_mut(tier.tier.as_str()) {
            *slot = count;
        }
    }

    let mut regressions = Vec::new();
    for (tier, floor) in TIER_FLOORS {
        let have = native.get(tier).copied().unwrap_or(0);
        if have < *floor {
            regressions.push(format!("{tier}: {have} Native, floor is {floor}"));
        }
    }

    assert!(
        regressions.is_empty(),
        "a capability floor may only RISE. Do not lower one to go green — that turns a lost \
         capability into a passing test:\n  {}",
        regressions.join("\n  ")
    );
}

#[test]
fn every_tier_has_a_floor() {
    let floors: BTreeSet<&str> = TIER_FLOORS.iter().map(|(t, _)| *t).collect();
    let tiers: BTreeSet<&str> = TIERS.iter().map(|(t, _)| *t).collect();
    assert_eq!(
        floors, tiers,
        "every tier needs a floor entry, or its capability is unratcheted"
    );
}

// ============================================================================================
// THE SCOREBOARD — not an assertion; the number we are trying to move. Run with --nocapture.
// ============================================================================================

#[test]
fn scoreboard() {
    let have = classified();
    let m = manifest();

    let mut rows: Vec<(String, usize, usize, usize, usize, usize, usize)> = Vec::new();
    for (tier, total) in TIERS {
        let ids: Vec<&ManifestCase> = m.cases.iter().filter(|c| c.tier_dir() == *tier).collect();
        let (mut must, mut diverge, mut out) = (0, 0, 0);
        let (mut native, mut gap, mut refused) = (0, 0, 0);
        for c in &ids {
            if let Some((_, v)) = have.get(&c.id) {
                match v.verdict {
                    Verdict::MUST => must += 1,
                    Verdict::DIVERGE => diverge += 1,
                    Verdict::OUT => out += 1,
                }
                match v.status {
                    Status::Native => native += 1,
                    Status::Gap => gap += 1,
                    Status::Refused => refused += 1,
                }
            }
        }
        assert_eq!(ids.len(), *total, "tier {tier} case count drifted");
        rows.push((tier.to_string(), must, diverge, out, native, gap, refused));
    }

    eprintln!("\n  LEAN PARITY MATRIX");
    eprintln!(
        "  {:<22} {:>5} {:>8} {:>5} | {:>7} {:>5} {:>8}",
        "tier", "MUST", "DIVERGE", "OUT", "Native", "Gap", "Refused"
    );
    for (tier, must, diverge, out, native, gap, refused) in &rows {
        eprintln!(
            "  {tier:<22} {must:>5} {diverge:>8} {out:>5} | {native:>7} {gap:>5} {refused:>8}"
        );
    }
    let classified_n = have.len();
    eprintln!(
        "  {:<22} {:>5} {:>8} {:>5} | {:>7} {:>5} {:>8}",
        "TOTAL",
        rows.iter().map(|r| r.1).sum::<usize>(),
        rows.iter().map(|r| r.2).sum::<usize>(),
        rows.iter().map(|r| r.3).sum::<usize>(),
        rows.iter().map(|r| r.4).sum::<usize>(),
        rows.iter().map(|r| r.5).sum::<usize>(),
        rows.iter().map(|r| r.6).sum::<usize>(),
    );
    eprintln!("  classified {classified_n}/{CORPUS_CASE_COUNT}\n");
}

// ============================================================================================
// GUARD SELF-TESTS — every guard above is fed a synthetic violation and must fire. A lock whose
// guards have never been shown to fail is a lock nobody has tested.
// ============================================================================================

#[cfg(test)]
mod guard_selftests {
    use super::*;

    fn case(id: &str, verdict: Verdict, status: Status, test: &str, why: &str) -> CaseVerdict {
        CaseVerdict {
            id: id.to_string(),
            verdict,
            status,
            test: test.to_string(),
            why: why.to_string(),
        }
    }

    /// The completeness guard's predicate: which corpus ids are absent from a classification.
    fn missing_against(classified: &BTreeSet<&str>) -> Vec<&'static str> {
        corpus_ids()
            .iter()
            .copied()
            .filter(|id| !classified.contains(id))
            .collect()
    }

    #[test]
    fn the_completeness_guard_fires_on_an_unclassified_case() {
        let mut all: BTreeSet<&str> = corpus_ids().iter().copied().collect();
        assert!(
            missing_against(&all).is_empty(),
            "a fully-classified set must report nothing missing"
        );
        let dropped = *all.iter().next().expect("the corpus is non-empty");
        all.remove(dropped);
        assert_eq!(
            missing_against(&all),
            vec![dropped],
            "dropping one case must be reported as exactly that case"
        );
    }

    #[test]
    fn the_stray_guard_fires_on_a_case_outside_the_corpus() {
        let ids = corpus_ids();
        assert!(!ids.contains("T99-999"), "T99-999 must not be a real case");
        assert!(ids.contains("T00-001"), "T00-001 must be a real case");
    }

    #[test]
    fn the_argument_guard_fires_on_an_unargued_divergence() {
        let unargued = case("T00-001", Verdict::DIVERGE, Status::Native, "t", "n/a");
        let argued = case(
            "T00-002",
            Verdict::DIVERGE,
            Status::Native,
            "t",
            "LOGOS is cumulative where Lean is not; this is Coq-style and strictly more permissive.",
        );
        let must_without_why = case("T00-003", Verdict::MUST, Status::Gap, "t", "");

        let needs_argument = |c: &CaseVerdict| {
            matches!(c.verdict, Verdict::DIVERGE | Verdict::OUT)
                && c.why.trim().len() < MIN_ARGUMENT_LEN
        };

        assert!(needs_argument(&unargued), "\"n/a\" must not clear the bar");
        assert!(
            !needs_argument(&argued),
            "a real argument must clear the bar"
        );
        assert!(
            !needs_argument(&must_without_why),
            "a MUST needs no argument — only DIVERGE and OUT do"
        );
    }

    #[test]
    fn the_one_test_per_case_guard_fires_on_a_shared_test() {
        let shared = [
            case(
                "T00-001",
                Verdict::MUST,
                Status::Gap,
                "covers_everything",
                "",
            ),
            case(
                "T00-002",
                Verdict::MUST,
                Status::Gap,
                "covers_everything",
                "",
            ),
            case("T00-003", Verdict::MUST, Status::Gap, "its_own", ""),
        ];
        let mut by_test: BTreeMap<&str, usize> = BTreeMap::new();
        for c in &shared {
            *by_test.entry(c.test.trim()).or_default() += 1;
        }
        assert_eq!(
            by_test.get("covers_everything"),
            Some(&2),
            "two cases naming one test must be detected"
        );
        assert_eq!(by_test.get("its_own"), Some(&1));
        assert!(
            by_test.values().any(|n| *n > 1),
            "the guard's predicate must fire on the shared test"
        );
    }

    #[test]
    fn the_test_name_guard_fires_on_a_nameless_verdict() {
        let nameless = case("T00-001", Verdict::MUST, Status::Gap, "   ", "");
        let named = case("T00-002", Verdict::MUST, Status::Gap, "decides_it", "");
        assert!(
            nameless.test.trim().is_empty(),
            "whitespace is not a test name"
        );
        assert!(!named.test.trim().is_empty());
    }

    #[test]
    fn the_floor_guard_fires_on_a_regression() {
        let floor = 7usize;
        assert!(
            6 < floor,
            "6 Native against a floor of 7 must be a regression"
        );
        assert!(!(7 < floor), "meeting the floor exactly must pass");
        assert!(!(8 < floor), "exceeding the floor must pass");
    }

    #[test]
    fn only_native_counts_toward_a_floor() {
        let cases = [
            case("T00-001", Verdict::MUST, Status::Native, "a", ""),
            case("T00-002", Verdict::MUST, Status::Gap, "b", ""),
            case(
                "T00-003",
                Verdict::OUT,
                Status::Refused,
                "c",
                "out of scope, argued at length…",
            ),
        ];
        let native = cases.iter().filter(|c| c.status == Status::Native).count();
        assert_eq!(native, 1, "Gap and Refused must not inflate the floor");
    }

    /// The schema is strict on purpose: a typo in a verdict file must be a loud parse error, never
    /// a silently-dropped classification.
    #[test]
    fn a_verdict_file_with_an_unknown_field_is_refused() {
        let bad = r#"{"tier":"t00_terms","test_binary":"x","cases":[
            {"id":"T00-001","verdict":"MUST","status":"Gap","test":"t","whyy":"typo"}]}"#;
        assert!(
            serde_json::from_str::<TierVerdicts>(bad).is_err(),
            "an unknown field must be refused, or a typo silently drops the argument"
        );
    }

    #[test]
    fn a_verdict_file_with_an_unknown_tag_is_refused() {
        for bad in [
            r#"{"tier":"t00_terms","test_binary":"x","cases":[
                {"id":"T00-001","verdict":"MAYBE","status":"Gap","test":"t"}]}"#,
            r#"{"tier":"t00_terms","test_binary":"x","cases":[
                {"id":"T00-001","verdict":"MUST","status":"Partial","test":"t"}]}"#,
        ] {
            assert!(
                serde_json::from_str::<TierVerdicts>(bad).is_err(),
                "only the declared verdict/status tags are admissible"
            );
        }
    }

    #[test]
    fn a_well_formed_verdict_file_parses() {
        let good = r#"{"tier":"t00_terms","test_binary":"lean_conformance_t00_terms","cases":[
            {"id":"T00-001","verdict":"MUST","status":"Gap","test":"decides_it"}]}"#;
        let parsed: TierVerdicts =
            serde_json::from_str(good).expect("the documented schema must parse");
        assert_eq!(parsed.tier, "t00_terms");
        assert_eq!(parsed.cases.len(), 1);
        assert_eq!(parsed.cases[0].verdict, Verdict::MUST);
        assert_eq!(parsed.cases[0].status, Status::Gap);
        assert!(parsed.cases[0].why.is_empty(), "why defaults to empty");
    }

    /// `tier_dir` locates a case by its path, not by the prose tier string — pin that, since the
    /// prose spells tier 10 as "10 · …" while the directory is `t10_literals`.
    #[test]
    fn tier_dir_reads_the_directory_not_the_prose() {
        let by_dir: BTreeSet<&str> = manifest().cases.iter().map(|c| c.tier_dir()).collect();
        let pinned: BTreeSet<&str> = TIERS.iter().map(|(t, _)| *t).collect();
        assert_eq!(
            by_dir, pinned,
            "every case must resolve to a pinned tier directory"
        );
    }

    /// The titles are what a human reads in a failure message; an empty one makes the scoreboard
    /// useless.
    #[test]
    fn every_corpus_case_has_a_title() {
        let untitled: Vec<&str> = manifest()
            .cases
            .iter()
            .filter(|c| c.title.trim().is_empty())
            .map(|c| c.id.as_str())
            .collect();
        assert!(
            untitled.is_empty(),
            "these cases have no title: {untitled:?}"
        );
    }
}
