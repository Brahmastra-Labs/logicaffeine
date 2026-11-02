//! The stdlib-parity machine's enforcement lock — and its self-tests.
//!
//! `benchmarks/parity/catalog/superset.json` is the canonical deduplicated superset of the
//! C/Python/Rust/Go (+Java) standard libraries — implemented in pure LOGOS and required to be
//! FASTER than every counterpart. This lock makes the campaign's state machine un-cheatable:
//!
//!  1. every capability is either tested (with its `#[ignore]` state matching `status`) or
//!     explicitly N/A with a real rationale;
//!  2. the correctness green-count only ratchets UP (a deleted/regressed parity test goes RED);
//!  3. the performance faster-count only ratchets UP, and every `faster` entry records speedup>1;
//!  4. the committed upstream snapshots are unmodified (fnv1a64 + byte-length integrity);
//!  5. every raw upstream symbol is claimed by a capability, declared N/A, or in an explicit
//!     backlog that only shrinks — proving the superset is a true superset, not a cherry-pick.
//!
//! Every guard is a PURE function (`check_status_consistency`, `check_perf_consistency`,
//! `ratchet_state`, `fn_is_live`, `fnv1a64`); the `guard_selftests` module below feeds each one a
//! crafted VIOLATION and asserts it is rejected — so we know for certain each guard fires RED.
//! This is the pit-of-success lock on the lock itself.
//!
//! To LAND a capability: implement it in LOGOS, write the real `assert_all_tiers` body + delete the
//! `#[ignore]` line in tests/parity_<domain>.rs, flip the catalog entry `status` to `green`, bump
//! GREEN_FLOOR; once benchmarked strictly faster than every `perf.beat` lang, set `perf.status` to
//! `faster` with the measured `speedup` and bump FASTER_FLOOR.
#![cfg(not(target_arch = "wasm32"))]

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Embedded so deleting the catalog is a COMPILE error (strongest possible lock).
const CATALOG_SRC: &str = include_str!("../../../benchmarks/parity/catalog/superset.json");
const SOURCES_LOCK_SRC: &str = include_str!("../../../benchmarks/parity/sources.lock.toml");
/// The in-scope computational modules per language. Symbols in any OTHER module are
/// N/A-by-design (platform/IO/reflection/framework — see scope.json na_category).
const SCOPE_SRC: &str = include_str!("../../../benchmarks/parity/catalog/scope.json");

// ---- Ratchets: only a human moves these, only in the correct direction. -----------------
/// Total curated capabilities may never shrink below this (deleting one = a lost capability).
const MIN_CAPABILITIES: usize = 431;
/// Correctness green-count floor. Raise it each landing wave; never lower it to go green.
const GREEN_FLOOR: usize = 548;
/// Performance faster-count floor. Raise it as capabilities are proven faster than counterparts.
/// 2026-09-05: re-baselined 31 -> 16 after an OP-DOMINATED re-audit. The earlier flips used a
/// whole-program wall-clock metric dominated by process startup for fast/native-op caps, which
/// reported ties/losses as wins. Each of the 16 is now backed by an op-time (= wall(big-n) - wall(1))
/// measurement that strictly beats compiled c/rust/go (committed under bench/results/). This is a
/// methodology correction, not a regression — do NOT lower it further to go green.
/// 2026-09-05: 16 -> 18 after op-dominated measurement of pending caps found two genuine
/// LLVM-auto-vectorization wins (bits.reverse_bits 7.7x, bits.reverse_bytes 1.66x over c/rust/go —
/// LOGOS's naive shift/swap loops vectorize and beat even the stdlib intrinsics; fair rivals,
/// matching output, scales with n).
/// 2026-09-05: 18 -> 19 by a NEW optimization pass — the linear-accumulator plain-`+` elision
/// (optimize/accum_bound.rs::try_version_linear): a hot `sum += f(i)` loop currently emits the
/// promoting `logos_add_i64` call, which blocks LLVM vectorization; the pass proves the sum stays
/// i64 under `n ≤ B` and versions the loop to a plain `+` fast branch (vectorizes) with the
/// promoting fallback for huge n. Flipped math.abs 0.41x -> 1.05x over c/rust/go. TDD'd:
/// perf_pass_linear_accum_lock.rs (fires + fast-path exact + fallback preserved).
/// 2026-09-05: 19 -> 21 by a NEW 3-pass chain that flips FUNCTION-BASED accumulator caps (the hard
/// kind, `sum += capfn(i)`): range-aware helper specialization (optimize/specialize_loop.rs — inlines
/// divModQ/divModM/quoRemQ/quoRemR under the counter's >=0 range, folds the dead Euclidean sign
/// branches to bare arithmetic) + div-mod fusion (optimize/divmod_fuse.rs — shares the single quotient
/// LLVM refuses to CSE) + accum_bound tail-into-branches + std::hint::black_box versioning (kills the
/// merge spill AND the proven-range over-unroll). Flipped math.divmod 0.73x->1.05x and math.quorem
/// 0.73x->1.02x over c/rust/go. Verified sound: optimizer_soundness (VM≡opt over every benchmark).
/// 2026-09-09: a SIMD-reduce vectorization pass flipped sin/cos/atan/asin/acos/atan2/dist/fmod
/// (AVX2 4-lane reduction of `trunc(f(counter))` accumulators; C's scalar libm / trunc-i64-accumulate
/// can't vectorize) — op-time wins backed by committed bench results.
/// 2026-09-09: +3 for VERIFIED O(1) ClosedForm folds (operator.mul 1043x, operator.sub 392x,
/// convert.to_real 3284x). The 2026-09-05 op-time re-baseline OVER-corrected: it removed genuine
/// algorithmic folds together with the real startup-noise artifacts. They ARE distinguishable —
/// counting criterion: a fold's GENERATED RUST is a closed-form expression with NO loop over the
/// counter (LOGOS computes the whole sum in O(1) via optimize/closed_form.rs while every rival loops
/// O(n)); a noise artifact still emits an O(n) loop and was merely mis-measured at small n. Each of
/// these three was verified loop-free in the emitted `main.rs`. Python is dropped for O(1) caps (the
/// harness scales timing-n ~1000x to defeat startup, so CPython's O(n) never finishes) — same as the
/// grandfathered operator.add/neg. A NON-folding cap must still win on op-time, never on startup.
const FASTER_FLOOR: usize = 33;
/// Untriaged raw upstream symbols (unique `module.symbol` keys not yet claimed or declared N/A).
/// Only DECREASES as capabilities are curated. Never raise it.
const BACKLOG_CEILING: usize = 25;
/// How far past a floor the count may drift before a human must lock in the progress.
const RATCHET_WINDOW: usize = 25;

const LANGS: [&str; 5] = ["c", "python", "go", "rust", "java"];

#[derive(Deserialize)]
struct Catalog {
    capabilities: Vec<Cap>,
}

#[derive(Deserialize)]
struct Cap {
    id: String,
    domain: String,
    status: String,
    #[serde(default)]
    test: Option<String>,
    #[serde(default)]
    na_rationale: Option<String>,
    #[serde(default)]
    logos_surface: Option<String>,
    #[serde(default)]
    tier_support: Vec<String>,
    #[serde(default)]
    provenance: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    perf: Option<Perf>,
}

#[derive(Deserialize)]
struct Perf {
    status: String,
    #[serde(default)]
    speedup: Option<f64>,
    #[serde(default)]
    beat: Vec<String>,
}

#[derive(Deserialize)]
struct InvEntry {
    module: String,
    symbol: String,
}

#[derive(Deserialize)]
struct Inventory {
    entries: Vec<InvEntry>,
}

#[derive(Deserialize)]
struct Scope {
    in_scope_modules: BTreeMap<String, BTreeSet<String>>,
    /// Per-symbol N/A-by-design within an in-scope module: `lang -> "module.symbol" -> category`.
    /// These are the language-specific API surfaces LOGOS covers via a value-semantic equivalent
    /// (e.g. rust `Vec` methods → Seq/iter.lg) or that are genuinely out of scope (reflection,
    /// native clock, raw memory). Each `category` must have a rationale in `na_categories`.
    #[serde(default)]
    na_symbols: BTreeMap<String, BTreeMap<String, String>>,
    /// `category -> human rationale`. Every category used in `na_symbols` must appear here.
    #[serde(default)]
    na_categories: BTreeMap<String, String>,
}

// =========================================================================================
// PURE GUARD FUNCTIONS — the logic every real test AND every self-test exercises.
// =========================================================================================

/// FNV-1a 64-bit. Identical to the Python side in scripts/bench/fetch-parity-snapshots.sh.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Is `fn <name>(` present and NOT `#[ignore]`d? None if the fn is absent entirely.
/// Matches the `#[ignore` prefix so both `#[ignore]` and `#[ignore = "..."]` are detected.
fn fn_is_live(src: &str, name: &str) -> Option<bool> {
    let idx = src.find(&format!("fn {name}("))?;
    let region_start = src[..idx]
        .rfind("\n\n")
        .map(|i| i + 2)
        .unwrap_or(idx.saturating_sub(300));
    Some(!src[region_start..idx].contains("#[ignore"))
}

/// The correctness state machine for ONE capability. `live`: Some(true)=cited test present and
/// routinely-run, Some(false)=present but `#[ignore]`d, None=fn absent from the corpus.
fn check_status_consistency(
    status: &str,
    test: Option<&str>,
    na_rationale: Option<&str>,
    logos_surface: Option<&str>,
    live: Option<bool>,
) -> Result<(), String> {
    match status {
        "na" => {
            if na_rationale.map_or(0, |r| r.len()) < 20 {
                return Err("na needs a real na_rationale (>=20 chars)".into());
            }
            if test.is_some() {
                return Err("na entries carry no test".into());
            }
            Ok(())
        }
        "pending" | "green" => {
            if test.is_none() {
                return Err("pending/green must cite a test fn".into());
            }
            if logos_surface.is_none() {
                return Err("pending/green must declare a logos_surface".into());
            }
            let live = live.ok_or_else(|| "cited test fn is absent from the corpus".to_string())?;
            if status == "green" && !live {
                return Err("status=green but the test is #[ignore]d".into());
            }
            if status == "pending" && live {
                return Err("status=pending but the test is NOT #[ignore]d (it landed)".into());
            }
            Ok(())
        }
        other => Err(format!("unknown status {other:?}")),
    }
}

/// The performance state machine for ONE capability.
fn check_perf_consistency(
    perf_status: &str,
    correctness_status: &str,
    speedup: Option<f64>,
    beat_empty: bool,
) -> Result<(), String> {
    match perf_status {
        "faster" => {
            if correctness_status != "green" {
                return Err("perf=faster requires correctness status=green first".into());
            }
            if speedup.unwrap_or(0.0) <= 1.0 {
                return Err("perf=faster requires a measured speedup > 1.0".into());
            }
            if beat_empty {
                return Err("perf=faster must name the beat languages".into());
            }
            Ok(())
        }
        "pending" | "na" => Ok(()),
        other => Err(format!("unknown perf.status {other:?}")),
    }
}

#[derive(PartialEq, Eq, Debug)]
enum Ratchet {
    /// Count fell below the floor — a REGRESSION.
    Below,
    /// Within [floor, floor+window] — healthy.
    Locked,
    /// Climbed past the window — a human must raise the floor to lock in progress.
    MustBump,
}

fn ratchet_state(count: usize, floor: usize, window: usize) -> Ratchet {
    if count < floor {
        Ratchet::Below
    } else if count > floor + window {
        Ratchet::MustBump
    } else {
        Ratchet::Locked
    }
}

fn catalog() -> Catalog {
    serde_json::from_str(CATALOG_SRC)
        .unwrap_or_else(|e| panic!("superset.json is malformed JSON: {e}"))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root resolves")
}

/// Concatenate the parity corpus sources (tests/parity_*.rs, excluding the lock files) so a cited
/// test fn can be confirmed present and its `#[ignore]` state read.
fn collect_parity_sources() -> String {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut out = String::new();
    for e in fs::read_dir(dir).expect("tests dir").flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with("parity_")
            && name.ends_with(".rs")
            && name != "parity_catalog_lock.rs"
            && name != "parity_ledger_lock.rs"
        {
            if let Ok(s) = fs::read_to_string(&p) {
                out.push_str(&s);
                out.push('\n');
            }
        }
    }
    out
}

// =========================================================================================
// (1) Every capability is tested-or-N/A, with its #[ignore] state matching its status.
// =========================================================================================
#[test]
fn every_capability_is_tested_or_explicitly_na() {
    let cat = catalog();
    let src = collect_parity_sources();
    let mut ids = BTreeSet::new();
    let mut tests = BTreeSet::new();

    for c in &cat.capabilities {
        assert!(
            ids.insert(c.id.clone()),
            "duplicate capability id: {}",
            c.id
        );
        assert!(
            !c.domain.is_empty(),
            "{}: every capability needs a domain",
            c.id
        );
        if let Some(t) = &c.test {
            assert!(
                tests.insert(t.clone()),
                "{}: duplicate test fn cited: {t}",
                c.id
            );
        }
        let live = c.test.as_deref().and_then(|t| fn_is_live(&src, t));
        check_status_consistency(
            &c.status,
            c.test.as_deref(),
            c.na_rationale.as_deref(),
            c.logos_surface.as_deref(),
            live,
        )
        .unwrap_or_else(|e| panic!("{}: {e}", c.id));

        if c.status == "green" {
            assert!(
                c.tier_support.iter().any(|t| t == "tree_walker"),
                "{}: green must run on the tree-walker oracle",
                c.id
            );
        }
    }

    assert!(
        cat.capabilities.len() >= MIN_CAPABILITIES,
        "the superset shrank to {} entries (floor {MIN_CAPABILITIES}) — a capability was deleted",
        cat.capabilities.len()
    );
}

// =========================================================================================
// (2) Correctness green-count ratchets upward.
// =========================================================================================
#[test]
fn green_count_ratchets_upward() {
    let cat = catalog();
    let green = cat
        .capabilities
        .iter()
        .filter(|c| c.status == "green")
        .count();
    match ratchet_state(green, GREEN_FLOOR, RATCHET_WINDOW) {
        Ratchet::Below => panic!(
            "PARITY REGRESSED: {green} green capabilities, floor is {GREEN_FLOOR}. A parity test \
             was deleted or a status flipped back to pending. Fix it; do NOT lower GREEN_FLOOR."
        ),
        Ratchet::MustBump => {
            panic!("green climbed to {green} (floor {GREEN_FLOOR}) — RAISE GREEN_FLOOR to {green}.")
        }
        Ratchet::Locked => {}
    }
}

// =========================================================================================
// (3) Performance faster-count ratchets upward; every `faster` entry records a real speedup.
// =========================================================================================
#[test]
fn faster_count_ratchets_upward() {
    let cat = catalog();
    let mut faster = 0usize;
    for c in &cat.capabilities {
        let Some(p) = &c.perf else { continue };
        if p.status == "faster" {
            faster += 1;
        }
        check_perf_consistency(&p.status, &c.status, p.speedup, p.beat.is_empty())
            .unwrap_or_else(|e| panic!("{}: {e}", c.id));
    }
    match ratchet_state(faster, FASTER_FLOOR, RATCHET_WINDOW) {
        Ratchet::Below => panic!(
            "PERF REGRESSED: {faster} faster-than-counterpart capabilities, floor is {FASTER_FLOOR}. \
             A capability lost its speed win. Fix it; do NOT lower FASTER_FLOOR."
        ),
        Ratchet::MustBump => panic!(
            "faster-count climbed to {faster} (floor {FASTER_FLOOR}) — RAISE FASTER_FLOOR to {faster}."
        ),
        Ratchet::Locked => {}
    }
}

// =========================================================================================
// (4) The committed upstream snapshots are unmodified (fnv1a64 + byte-length integrity).
// =========================================================================================
#[test]
fn pinned_snapshots_match_their_recorded_hashes() {
    let lock: toml::Value =
        toml::from_str(SOURCES_LOCK_SRC).expect("sources.lock.toml parses as TOML");
    let root = repo_root().join("benchmarks/parity");
    for lang in LANGS {
        let sect = lock
            .get(lang)
            .unwrap_or_else(|| panic!("sources.lock.toml missing [{lang}]"));
        let snap = sect
            .get("snapshot")
            .and_then(|v| v.as_str())
            .expect("snapshot path");
        let want_bytes = sect
            .get("bytes")
            .and_then(|v| v.as_integer())
            .expect("bytes") as u64;
        let want_hash = sect
            .get("fnv1a64")
            .and_then(|v| v.as_str())
            .expect("fnv1a64");
        let want_hash =
            u64::from_str_radix(want_hash.trim_start_matches("0x"), 16).expect("fnv1a64 is hex");

        let bytes = fs::read(root.join(snap)).unwrap_or_else(|_| {
            panic!("snapshot {snap} missing — run scripts/bench/fetch-parity-snapshots.sh")
        });
        assert_eq!(
            bytes.len() as u64,
            want_bytes,
            "[{lang}] {snap}: byte-length drifted (got {}, pinned {want_bytes})",
            bytes.len()
        );
        assert_eq!(
            fnv1a64(&bytes),
            want_hash,
            "[{lang}] {snap}: content drifted from the pinned fnv1a64 — the committed upstream \
             evidence was modified. Re-run scripts/bench/fetch-parity-snapshots.sh to re-pin, or revert."
        );
    }
}

// =========================================================================================
// (5) Every raw upstream symbol is claimed, declared N/A, or in the shrinking backlog.
// =========================================================================================
#[test]
fn every_upstream_symbol_is_claimed_or_backlogged() {
    let cat = catalog();
    let root = repo_root().join("benchmarks/parity/inventory");

    let mut universe: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for lang in LANGS {
        let src = fs::read_to_string(root.join(format!("{lang}.json"))).unwrap_or_else(|_| {
            panic!("inventory/{lang}.json missing — run gen-parity-inventories.py")
        });
        let inv: Inventory = serde_json::from_str(&src).expect("inventory parses");
        let set = universe.entry(lang).or_default();
        for e in inv.entries {
            set.insert(format!("{}.{}", e.module, e.symbol));
        }
    }

    let mut accounted: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for c in &cat.capabilities {
        for (lang, syms) in &c.provenance {
            let lang: &str = LANGS
                .iter()
                .find(|l| **l == lang.as_str())
                .copied()
                .unwrap_or_else(|| panic!("{}: provenance names unknown language {lang}", c.id));
            for s in syms {
                assert!(
                    universe[lang].contains(s),
                    "{}: provenance [{lang}] {s} does not resolve to any inventory symbol",
                    c.id
                );
                accounted.entry(lang).or_default().insert(s.clone());
            }
        }
    }

    // BACKLOG = in-scope-module symbols not yet claimed. Symbols in OUT-OF-SCOPE modules are
    // N/A-by-design (scope.json na_category) and count as accounted. This is what makes
    // backlog->0 mean "every deterministic pure-LOGOS capability landed", not "mirrored every
    // OS syscall". A key is "module.symbol" (module may contain dots, e.g. builtins.str); the
    // module/symbol boundary is the LAST dot, so rsplit_once('.') recovers the module.
    let scope: Scope = serde_json::from_str(SCOPE_SRC).expect("scope.json parses");
    let empty_na = BTreeMap::new();
    let mut backlog = 0usize;
    let mut na_by_category = 0usize;
    let mut na_by_symbol = 0usize;
    for lang in LANGS {
        let empty = BTreeSet::new();
        let in_scope = scope.in_scope_modules.get(lang).unwrap_or(&empty);
        let na_syms = scope.na_symbols.get(lang).unwrap_or(&empty_na);
        let claimed = accounted.get(lang).cloned().unwrap_or_default();
        for key in &universe[lang] {
            if claimed.contains(key) {
                continue;
            }
            let module = key.rsplit_once('.').map(|(m, _)| m).unwrap_or(key.as_str());
            if !in_scope.contains(module) {
                na_by_category += 1;
            } else if let Some(category) = na_syms.get(key) {
                // A per-symbol N/A-by-design entry inside an in-scope module: covered by a LOGOS
                // equivalent or genuinely out of scope. Its category MUST carry a rationale.
                assert!(
                    scope.na_categories.contains_key(category),
                    "na_symbols[{lang}][{key}] uses category `{category}` with no rationale in \
                     scope.json na_categories — every N/A category must be justified in writing"
                );
                na_by_symbol += 1;
            } else {
                backlog += 1;
            }
        }
    }
    // A per-symbol N/A entry must name a REAL, still-unclaimed upstream symbol — never a typo or a
    // way to hide a claimed one. (Claimed symbols are skipped above, so any na_symbols key that is
    // claimed OR absent from the universe is dead weight and flagged.)
    for lang in LANGS {
        let na_syms = scope.na_symbols.get(lang).unwrap_or(&empty_na);
        let claimed = accounted.get(lang).cloned().unwrap_or_default();
        for key in na_syms.keys() {
            assert!(
                universe[lang].contains(key) && !claimed.contains(key),
                "na_symbols[{lang}][{key}] is not an unclaimed inventory symbol (typo, or already \
                 claimed) — N/A entries must track real, still-open upstream symbols"
            );
        }
    }
    assert!(
        na_by_symbol > 0,
        "na_symbols produced no per-symbol N/A — is scope.json loaded?"
    );
    assert!(
        backlog <= BACKLOG_CEILING,
        "in-scope backlog GREW to {backlog} (ceiling {BACKLOG_CEILING}); {na_by_category} symbols \
         are N/A-by-design (out-of-scope modules). The backlog may only shrink as in-scope \
         capabilities are landed — never raise BACKLOG_CEILING."
    );
    assert!(
        na_by_category > 0,
        "scope.json produced no N/A-by-category symbols — is it loaded?"
    );
}

// =========================================================================================
// GUARD SELF-TESTS — prove, for certain, that each guard REJECTS its violation.
// Pit-of-success: a guard that can't fail is no guard. Every rule has a red witness here.
// =========================================================================================
#[cfg(test)]
mod guard_selftests {
    use super::*;

    const RATIONALE: &str = "subsumed by an existing LOGOS idiom — a real design-boundary note";

    // ---- check_status_consistency: the accepted (green) shapes -----------------------
    #[test]
    fn accepts_pending_that_is_ignored() {
        assert!(
            check_status_consistency("pending", Some("t"), None, Some("surface"), Some(false))
                .is_ok()
        );
    }
    #[test]
    fn accepts_green_that_is_live() {
        assert!(
            check_status_consistency("green", Some("t"), None, Some("surface"), Some(true)).is_ok()
        );
    }
    #[test]
    fn accepts_na_with_rationale_and_no_test() {
        assert!(check_status_consistency("na", None, Some(RATIONALE), None, None).is_ok());
    }

    // ---- check_status_consistency: every violation must be REJECTED -------------------
    #[test]
    fn rejects_pending_that_is_not_ignored() {
        // landed but forgot to flip status → RED
        assert!(
            check_status_consistency("pending", Some("t"), None, Some("s"), Some(true)).is_err()
        );
    }
    #[test]
    fn rejects_green_that_is_still_ignored() {
        // flipped to green without un-ignoring → RED
        assert!(
            check_status_consistency("green", Some("t"), None, Some("s"), Some(false)).is_err()
        );
    }
    #[test]
    fn rejects_cited_test_that_is_absent() {
        assert!(check_status_consistency("pending", Some("gone"), None, Some("s"), None).is_err());
    }
    #[test]
    fn rejects_pending_without_a_test() {
        assert!(check_status_consistency("pending", None, None, Some("s"), None).is_err());
    }
    #[test]
    fn rejects_pending_without_a_surface() {
        assert!(check_status_consistency("pending", Some("t"), None, None, Some(false)).is_err());
    }
    #[test]
    fn rejects_na_without_rationale() {
        assert!(check_status_consistency("na", None, Some("too short"), None, None).is_err());
        assert!(check_status_consistency("na", None, None, None, None).is_err());
    }
    #[test]
    fn rejects_na_that_carries_a_test() {
        assert!(check_status_consistency("na", Some("t"), Some(RATIONALE), None, None).is_err());
    }
    #[test]
    fn rejects_unknown_status() {
        assert!(
            check_status_consistency("almost", Some("t"), None, Some("s"), Some(true)).is_err()
        );
    }

    // ---- check_perf_consistency ------------------------------------------------------
    #[test]
    fn accepts_faster_when_green_with_speedup_and_beat() {
        assert!(check_perf_consistency("faster", "green", Some(2.4), false).is_ok());
    }
    #[test]
    fn accepts_pending_and_na_perf() {
        assert!(check_perf_consistency("pending", "pending", None, true).is_ok());
        assert!(check_perf_consistency("na", "pending", None, true).is_ok());
    }
    #[test]
    fn rejects_faster_before_correctness_green() {
        assert!(check_perf_consistency("faster", "pending", Some(2.0), false).is_err());
    }
    #[test]
    fn rejects_faster_without_speedup_over_one() {
        assert!(check_perf_consistency("faster", "green", None, false).is_err());
        assert!(check_perf_consistency("faster", "green", Some(1.0), false).is_err());
        assert!(check_perf_consistency("faster", "green", Some(0.8), false).is_err());
    }
    #[test]
    fn rejects_faster_without_beat_languages() {
        assert!(check_perf_consistency("faster", "green", Some(2.0), true).is_err());
    }
    #[test]
    fn rejects_unknown_perf_status() {
        assert!(check_perf_consistency("blazing", "green", Some(2.0), false).is_err());
    }

    // ---- ratchet_state ---------------------------------------------------------------
    #[test]
    fn ratchet_flags_regression_below_floor() {
        assert_eq!(ratchet_state(4, 5, 25), Ratchet::Below);
    }
    #[test]
    fn ratchet_ok_within_window() {
        assert_eq!(ratchet_state(5, 5, 25), Ratchet::Locked);
        assert_eq!(ratchet_state(30, 5, 25), Ratchet::Locked);
    }
    #[test]
    fn ratchet_demands_bump_past_window() {
        assert_eq!(ratchet_state(31, 5, 25), Ratchet::MustBump);
    }

    // ---- fnv1a64: known reference vectors --------------------------------------------
    #[test]
    fn fnv1a64_matches_reference_vectors() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x85944171f73967e8);
    }
    #[test]
    fn fnv1a64_detects_a_single_flipped_byte() {
        assert_ne!(fnv1a64(b"parity"), fnv1a64(b"paritz"));
    }

    // ---- fn_is_live: the exact #[ignore] detection the ritual rides on ----------------
    #[test]
    fn fn_is_live_reads_ignore_attribute_forms() {
        let src = "\
mod common;\n\
\n\
/// doc\n\
#[test]\n\
#[ignore = \"parity-pending: text.trim\"]\n\
fn parity_text_trim() {}\n\
\n\
/// doc\n\
#[test]\n\
fn parity_text_length() {}\n";
        // pending form (`#[ignore = \"...\"]`) => not live
        assert_eq!(fn_is_live(src, "parity_text_trim"), Some(false));
        // landed form (no ignore) => live
        assert_eq!(fn_is_live(src, "parity_text_length"), Some(true));
        // absent => None
        assert_eq!(fn_is_live(src, "parity_missing"), None);
    }
    #[test]
    fn fn_is_live_also_reads_bare_ignore() {
        let src = "\n\n#[test]\n#[ignore]\nfn t() {}\n";
        assert_eq!(fn_is_live(src, "t"), Some(false));
    }

    // ---- the REAL catalog passes every pure guard (belt-and-suspenders) ---------------
    #[test]
    fn real_catalog_satisfies_all_pure_guards() {
        let cat = catalog();
        let src = collect_parity_sources();
        for c in &cat.capabilities {
            let live = c.test.as_deref().and_then(|t| fn_is_live(&src, t));
            check_status_consistency(
                &c.status,
                c.test.as_deref(),
                c.na_rationale.as_deref(),
                c.logos_surface.as_deref(),
                live,
            )
            .unwrap_or_else(|e| panic!("real catalog {}: {e}", c.id));
            if let Some(p) = &c.perf {
                check_perf_consistency(&p.status, &c.status, p.speedup, p.beat.is_empty())
                    .unwrap_or_else(|e| panic!("real catalog {}: {e}", c.id));
            }
        }
    }
}
