//! mutation_lock.rs — Invariant III (the correctness core is mutation-proven), the in-suite half.
//!
//! cargo-mutants itself runs out-of-band (`scripts/test/run-mutation-core.sh`, minutes per file) — far
//! too slow for every `cargo test`. What runs HERE, every suite, is the cheap integrity layer that
//! keeps that nightly gate honest between runs. Three locks, all in the exhaustive-catalog house
//! style (no wildcard escape hatch):
//!
//!   1. SCOPE INTEGRITY — every file enrolled in `.config/mutants.toml`'s Tier-A `examine_globs` still
//!      exists on disk. A renamed or deleted core file silently leaving mutation scope is a P0.
//!   2. MANIFEST COMPLETENESS — every source in the trusted kernel (`logicaffeine_kernel/src`) is
//!      EXPLICITLY classified: enrolled in Tier-A mutation, or acknowledged here with a reason. A
//!      NEW kernel file that is in neither list fails the build until a human classifies it.
//!   3. BASELINE INTEGRITY — every surviving mutant recorded in `.config/mutants/known-survivors.txt`
//!      carries a verdict (EQUIVALENT / UNREACHABLE / a tracked TODO). An unexplained survivor is a
//!      P0; the baseline only shrinks.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("cannot read {rel}: {e}"))
}

/// The full Tier-A `examine_globs` set (single-file globs) from `.config/mutants.toml`, mirrored so the
/// lock is self-describing; [`tier_a_scope_is_intact`] asserts the mirror matches the checked-in
/// config and disk.
const TIER_A_FILES: &[&str] = &[
    "crates/logicaffeine_kernel/src/type_checker.rs",
    "crates/logicaffeine_kernel/src/recheck.rs",
    "crates/logicaffeine_kernel/src/reduction.rs",
    "crates/logicaffeine_kernel/src/recursor.rs",
    "crates/logicaffeine_kernel/src/eval.rs",
    "crates/logicaffeine_kernel/src/term.rs",
    "crates/logicaffeine_kernel/src/context.rs",
    "crates/logicaffeine_proof/src/certifier.rs",
    "crates/logicaffeine_proof/src/proof.rs",
    "crates/logicaffeine_proof/src/dimacs.rs",
    "crates/logicaffeine_proof/src/cnf.rs",
    "crates/logicaffeine_proof/src/xor_drat.rs",
    "crates/logicaffeine_proof/src/rup.rs",
    "crates/logicaffeine_data/src/wire.rs",
    "crates/logicaffeine_system/src/network/wire.rs",
    "crates/logicaffeine_base/src/sha_ops.rs",
    "crates/logicaffeine_base/src/hash.rs",
];

/// The Tier-A kernel files (the `logicaffeine_kernel/src` subset of `TIER_A_FILES`).
const ENROLLED_KERNEL: &[&str] = &[
    "type_checker.rs",
    "recheck.rs",
    "reduction.rs",
    "recursor.rs",
    "eval.rs",
    "term.rs",
    "context.rs",
];

/// Every OTHER current kernel source, each acknowledged with WHY it is not (yet) in Tier-A mutation
/// scope. A kernel file in NEITHER this list nor `ENROLLED_KERNEL` fails
/// [`every_kernel_source_is_classified`] — forcing a conscious decision about anything new.
const ACKNOWLEDGED_KERNEL: &[(&str, &str)] = &[
    ("lib.rs", "crate root: re-exports, no decision logic"),
    ("error.rs", "error type definitions, no decision logic"),
    ("prelude.rs", "StandardLibrary data registration, not a checker"),
    ("reify.rs", "reification / pretty-printing, not soundness-critical"),
    ("elaborate.rs", "Tier-A candidate: surface elaboration (enrollment tracked)"),
    ("certificate.rs", "Tier-A candidate: proof certificates (enrollment tracked)"),
    ("inductive_compile.rs", "Tier-A candidate: inductive compilation (enrollment tracked)"),
    ("termination.rs", "Tier-A candidate: Giménez termination guard (enrollment tracked)"),
    ("positivity.rs", "Tier-A candidate: strict-positivity checker (enrollment tracked)"),
    ("cc.rs", "Tier-A candidate: congruence closure (enrollment tracked)"),
    ("simp.rs", "Tier-A candidate: simplifier (enrollment tracked)"),
    ("bitvector.rs", "Tier-A candidate: bit-vector reasoning (enrollment tracked)"),
    ("field_algebra.rs", "Tier-A candidate: field algebra (enrollment tracked)"),
    ("ring.rs", "Tier-A candidate: ring algebra (enrollment tracked)"),
    ("word_ring.rs", "Tier-A candidate: word-ring algebra (enrollment tracked)"),
    ("lia.rs", "Tier-A candidate: linear integer arithmetic (enrollment tracked)"),
    ("omega.rs", "Tier-A candidate: Omega decision procedure (enrollment tracked)"),
    // The R-series kernel modules. Every one of these is decision or representation logic in the
    // TRUSTED core — a silent bug in any of them forges a proof — so each is a Tier-A candidate on
    // the same footing as the checkers above, never an exclusion.
    ("term_store.rs", "Tier-A candidate: hash-consed term storage — a wrong intern makes two different terms equal (enrollment tracked)"),
    ("tactic_cert.rs", "Tier-A candidate: the certificates the SECOND kernel replays for ring/LIA/congruence (enrollment tracked)"),
    ("annotation.rs", "Tier-A candidate: the transparent annotation carrier, erased at every checking door and in Context storage; its doors carry a 15-mutant schema in term_annotation_transparency_lock (enrollment tracked)"),
    ("int_inductive.rs", "Tier-A candidate: Int as a real inductive with DEFINED arithmetic (enrollment tracked)"),
    ("integer_laws.rs", "Tier-A candidate: the ring and order laws proven over that carrier (enrollment tracked)"),
    ("quotient.rs", "Tier-A candidate: certified quotient primitives — Quot.lift's respect obligation (enrollment tracked)"),
    ("equations.rs", "Tier-A candidate: equation lemmas — the name a proof cites for a definition's behaviour (enrollment tracked)"),
    // Conversion and the second kernel's modules. `recheck.rs` and `type_checker.rs` are enrolled;
    // these files hold decision logic split out of them, so a silent bug in any one forges or
    // loses a verdict exactly as a bug in its parent would.
    ("lazy_delta.rs", "Tier-A candidate: Lean-shaped lazy-delta definitional equality (enrollment tracked)"),
    ("conversion_session.rs", "Tier-A candidate: positive conversion evidence scoped to one borrowed, immutable context (enrollment tracked)"),
    ("observational.rs", "Tier-A candidate: observational equality's computation rules and their guards in the main kernel (enrollment tracked)"),
    ("recheck_observational.rs", "Tier-A candidate: the same rules in the second kernel, an independent de Bruijn implementation (enrollment tracked)"),
    ("reflection.rs", "Tier-A candidate: the attestation that gates the reflection definitions' native accelerators (enrollment tracked)"),
    ("tactics.rs", "the tactic layer builds proof terms the kernel checks; it decides no verdict"),
    ("context_order_cache.rs", "Tier-A candidate: context-scoped recurrence certificates — a stale entry answers for the wrong context (enrollment tracked)"),
    ("structure_eta.rs", "Tier-A candidate: checked structure-η registration — the evidence a structure is η-eligible (enrollment tracked)"),
    ("recheck_arithmetic.rs", "Tier-A candidate: the second kernel's independently checked Nat recurrence acceleration (enrollment tracked)"),
    ("recheck_definitions.rs", "Tier-A candidate: the second kernel's per-check reuse of lowered definition syntax (enrollment tracked)"),
    ("recheck_delta.rs", "Tier-A candidate: the second kernel's bounded delta exposure for its positive conversion probe (enrollment tracked)"),
    ("recheck_equality.rs", "Tier-A candidate: the second kernel's equality certification by de Bruijn conversion (enrollment tracked)"),
    ("recheck_folded.rs", "Tier-A candidate: the second kernel's application congruence before unfolding a common head (enrollment tracked)"),
    ("recheck_head_closure.rs", "Tier-A candidate: the second kernel's beta/zeta/delta congruence over head closures (enrollment tracked)"),
    ("recheck_lazy_delta.rs", "Tier-A candidate: the second kernel's own lazy-delta loop (enrollment tracked)"),
    ("recheck_storage.rs", "Tier-A candidate: the second kernel's shared storage of child edges — a wrong share makes two terms one (enrollment tracked)"),
    ("recheck_division.rs", "Tier-A candidate: the second kernel's own division certificate — the native quotient and remainder stand on its recurrence proof (enrollment tracked)"),
    ("recheck_conversion_memo.rs", "Tier-A candidate: the second kernel's per-check conversion evidence — an entry read in the wrong context or environment answers a pair it never proved (enrollment tracked)"),
    // Bounded construction, level ordering and positive elaboration evidence. Each decides what the
    // kernel builds or accepts under a caller-owned budget, so a silent bug forges a term or a verdict.
    ("term_level_order.rs", "Tier-A candidate: the universe ≤ decision (imax pivot elimination) behind ordinary conversion (enrollment tracked)"),
    ("elaborate_positive.rs", "Tier-A candidate: positive-only closed conversion the elaborator accepts before normalizing (enrollment tracked)"),
    ("recursor_bounded.rs", "Tier-A candidate: bounded ordinary/mutual recursor construction, admitted before materialization (enrollment tracked)"),
    ("recursor_bounded_universes.rs", "Tier-A candidate: admission of universe substitution before the syntax operation allocates (enrollment tracked)"),
    ("elimination_bounded.rs", "Tier-A candidate: bounded nested-inductive elimination, checking the universe meaning of both sort nodes (enrollment tracked)"),
    ("substitution_exact.rs", "Tier-A candidate: deterministic DAG substitution that must match type_checker::substitute exactly (enrollment tracked)"),
    ("native_recursors_bounded.rs", "Tier-A candidate: composition of bounded elimination, ordinary generation and native restoration (enrollment tracked)"),
    ("bounded_native_restoration.rs", "Tier-A candidate: bounded restoration of native terms against the source signature (enrollment tracked)"),
    ("native_specialization.rs", "Tier-A candidate: fresh private names for nested specialization — a collision rebinds a source declaration (enrollment tracked)"),
    // Control modules: each is compiled only under `#[cfg(test)]` beside the module it tests and
    // holds assertions, never decision logic that ships.
];

/// (1) Scope integrity: every Tier-A file is both listed in `.config/mutants.toml` and present on disk.
#[test]
fn tier_a_scope_is_intact() {
    let root = repo_root();
    let toml = read(&root, ".config/mutants.toml");
    for f in TIER_A_FILES {
        assert!(
            toml.contains(f),
            "Tier-A file dropped from .config/mutants.toml examine_globs (mutation scope regressed): {f}",
        );
        assert!(
            root.join(f).exists(),
            "Tier-A file enrolled in .config/mutants.toml is missing on disk (renamed/deleted?): {f}",
        );
    }
}

/// (2) Manifest completeness: the trusted kernel is exhaustively partitioned into enrolled +
/// acknowledged. A new kernel source in neither list fails here.
#[test]
fn every_kernel_source_is_classified() {
    let root = repo_root();
    let dir = root.join("crates/logicaffeine_kernel/src");
    let on_disk: BTreeSet<String> = std::fs::read_dir(&dir)
        .expect("read kernel/src")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".rs"))
        .collect();

    let classified: BTreeSet<String> = ENROLLED_KERNEL
        .iter()
        .map(|s| s.to_string())
        .chain(ACKNOWLEDGED_KERNEL.iter().map(|(f, _)| f.to_string()))
        .collect();

    let unclassified: Vec<&String> = on_disk.difference(&classified).collect();
    assert!(
        unclassified.is_empty(),
        "new trusted-kernel source(s) are unclassified for Tier-A mutation — enroll each in \
         .config/mutants.toml's examine_globs, or acknowledge it (with a reason) in mutation_lock.rs's \
         ACKNOWLEDGED_KERNEL: {unclassified:?}",
    );

    let stale: Vec<&String> = classified.difference(&on_disk).collect();
    assert!(
        stale.is_empty(),
        "mutation_lock.rs lists kernel files that no longer exist on disk: {stale:?}",
    );
}

/// The enrolled-kernel mirror must agree with `TIER_A_FILES` (no drift between the two mirrors).
#[test]
fn enrolled_kernel_mirror_matches_tier_a_files() {
    for f in ENROLLED_KERNEL {
        let full = format!("crates/logicaffeine_kernel/src/{f}");
        assert!(
            TIER_A_FILES.contains(&full.as_str()),
            "ENROLLED_KERNEL lists {f}, which is not in TIER_A_FILES — the mirrors have drifted",
        );
    }
}

/// (3) Baseline integrity: every survivor in the baseline is classified with a verdict.
#[test]
fn known_survivors_are_all_classified() {
    let root = repo_root();
    let txt = read(&root, ".config/mutants/known-survivors.txt");
    for (i, line) in txt.lines().enumerate() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        assert!(
            l.contains("EQUIVALENT") || l.contains("UNREACHABLE") || l.contains("TODO"),
            ".config/mutants/known-survivors.txt line {} records a survivor with NO verdict \
             (every survivor must be classified EQUIVALENT / UNREACHABLE / TODO): {l}",
            i + 1,
        );
    }
}
