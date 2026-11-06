//! Optimization-lattice completeness lock.
//!
//! The optimization lattice (`logicaffeine_language::optimization`) is the single
//! source of truth for *which* optimizations exist and how they are toggled
//! (`## No <keyword>`). The language crate already locks the lattice's INTERNAL
//! consistency (one `REGISTRY` row per `Opt`, unique keywords, `requires`/`conflicts`
//! referential integrity + acyclicity, `normalize` closure, cost monotonicity).
//!
//! What those in-crate tests cannot see is the *other* crate: whether every
//! optimization the compiler actually performs is wired to a lattice toggle. This
//! lock closes that cross-crate gap in both directions:
//!
//!  1. **No dead toggle** — every `Opt` in the registry is consulted at a real gate
//!     site in `logicaffeine_compile/src/` (`run_traced` / `is_on` / `admits`). A
//!     registry row the compiler never reads would be a lie in the UI.
//!  2. **No ungated optimization** — the set of gate-consulted opts equals the whole
//!     registry (`OPT_COUNT`), so adding an `Opt` without wiring it — or wiring a
//!     pass without a registry row — goes RED.
//!
//! And because the lattice is a *dependency graph*, not a flat set of bits, the lock
//! also pins the linked structure the config relies on:
//!
//!  3. **`requires` cascades** — disabling any optimization provably disables its
//!     transitive dependents through `normalize()` (the "linked/dependent types"
//!     invariant, asserted over the whole graph, not a hand-picked pair).
//!  4. **Default is everything-on, each individually addressable** — the default
//!     config has every default-on opt enabled, and each opt is reachable by its
//!     `## No <keyword>` and independently toggleable off.
//!  5. **`preempts` keeps both edges live** — a preempts edge references real
//!     variants, never itself, and leaves both optimizations enabled by default
//!     (the fallback surfaces only where the preemptor declined).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use logicaffeine_language::optimization::{
    by_keyword, Opt, OptimizationConfig, OPT_COUNT, REGISTRY,
};

/// Tokens that mark a line as a lattice *gate*: the config being consulted to decide
/// whether an optimization may run. All three gate forms live in `compile/src`:
///  - `run_traced(Opt::X, …)` — the AST optimizer pipeline (`optimize/mod.rs`).
///  - `is_on(Opt::X)` / `.is_on(Opt::X)` — codegen + VM per-site gates.
///  - `admits(…, Opt::X)` / `admits_pinned(…)` — the tiered-optimizer budget gate.
///
/// If a genuinely new gate form is ever introduced, add its token here in the same
/// reviewed change that introduces it; that is the one deliberate seam.
const GATE_TOKENS: &[&str] = &["run_traced", "is_on", "admits"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn compile_src() -> PathBuf {
    repo_root().join("crates/logicaffeine_compile/src")
}

/// Every `.rs` file under a directory, recursively.
fn rust_sources(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                out.extend(rust_sources(&p));
            } else if p.extension().is_some_and(|e| e == "rs") {
                out.push(p);
            }
        }
    }
    out
}

/// The variant name of an `Opt` (its `Debug` spelling, e.g. `ClosedForm`), which is
/// exactly how it is written in source as `Opt::ClosedForm`.
fn variant_name(opt: Opt) -> String {
    format!("{opt:?}")
}

/// The set of every registry variant name — the authoritative expected surface.
fn all_variant_names() -> BTreeSet<String> {
    REGISTRY.iter().map(|m| variant_name(m.opt)).collect()
}

/// Parse `Opt::<Ident>` occurrences out of a single source line.
fn opt_refs_on_line(line: &str) -> Vec<String> {
    let mut names = Vec::new();
    let bytes = line.as_bytes();
    let needle = b"Opt::";
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        let whole = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
        if whole && &bytes[i..i + needle.len()] == needle {
            let mut j = i + needle.len();
            let start = j;
            while j < bytes.len() {
                let c = bytes[j];
                let ident = c.is_ascii_alphanumeric() || c == b'_';
                if ident {
                    j += 1;
                } else {
                    break;
                }
            }
            if j > start {
                names.push(line[start..j].to_string());
            }
            i = j;
        } else {
            i += 1;
        }
    }
    names
}

/// Walk the compiler source and collect, separately:
///  - `gated`: opts named on a line that also contains a gate token (a real toggle).
///  - `referenced`: every opt named anywhere (for the phantom-reference check).
fn scan_compiler() -> (BTreeSet<String>, BTreeSet<String>) {
    let mut gated = BTreeSet::new();
    let mut referenced = BTreeSet::new();
    for file in rust_sources(&compile_src()) {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        for raw in text.lines() {
            let line = raw.trim();
            // Skip line comments so an opt mentioned only in prose is not counted as
            // a gate (block comments are rare in gate positions and harmless here).
            if line.starts_with("//") {
                continue;
            }
            let refs = opt_refs_on_line(line);
            if refs.is_empty() {
                continue;
            }
            let is_gate = GATE_TOKENS.iter().any(|t| line.contains(t));
            for name in refs {
                referenced.insert(name.clone());
                if is_gate {
                    gated.insert(name);
                }
            }
        }
    }
    (gated, referenced)
}

/// Direct `requires` edges: child variant name → the variants it directly requires.
fn direct_requires() -> BTreeMap<String, Vec<String>> {
    REGISTRY
        .iter()
        .map(|m| {
            (
                variant_name(m.opt),
                m.requires.iter().map(|r| variant_name(*r)).collect(),
            )
        })
        .collect()
}

/// For each opt, the transitive set of opts it (directly or indirectly) requires.
fn transitive_requires() -> BTreeMap<String, BTreeSet<String>> {
    let direct = direct_requires();
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for name in direct.keys() {
        let mut seen = BTreeSet::new();
        let mut stack: Vec<String> = direct[name].clone();
        while let Some(cur) = stack.pop() {
            if seen.insert(cur.clone()) {
                if let Some(parents) = direct.get(&cur) {
                    stack.extend(parents.iter().cloned());
                }
            }
        }
        out.insert(name.clone(), seen);
    }
    out
}

// ---------------------------------------------------------------------------
// 1. No dead toggle: every registry opt is wired to a real gate in the compiler.
// ---------------------------------------------------------------------------
#[test]
fn every_registry_opt_is_wired_to_a_gate_in_the_compiler() {
    let expected = all_variant_names();
    let (gated, _referenced) = scan_compiler();

    let dead: Vec<&String> = expected.difference(&gated).collect();
    assert!(
        dead.is_empty(),
        "these optimizations exist in the registry but are never consulted at a gate \
         ({:?}) in logicaffeine_compile/src — a dead toggle is a lie in the UI. Wire \
         each to a run_traced/is_on/admits site (or, if a new gate form was added, \
         list its token in GATE_TOKENS):\n{dead:#?}",
        GATE_TOKENS,
    );
}

// ---------------------------------------------------------------------------
// 2. No ungated pass / phantom gate: gated set == the whole registry, and every
//    named reference is a real variant.
// ---------------------------------------------------------------------------
#[test]
fn gated_opt_set_is_exactly_the_registry() {
    let expected = all_variant_names();
    let (gated, referenced) = scan_compiler();

    // The compiler consults exactly the registry — nothing more, nothing less.
    assert_eq!(
        gated, expected,
        "the set of gate-consulted optimizations must equal the registry \
         (OPT_COUNT = {OPT_COUNT}). Extra ⇒ a gate on a non-existent variant; \
         missing ⇒ an un-toggleable optimization.",
    );
    assert_eq!(
        expected.len(),
        OPT_COUNT,
        "registry size drifted from OPT_COUNT"
    );

    // Every `Opt::X` anywhere in the compiler names a real registry variant.
    let phantom: Vec<&String> = referenced.difference(&expected).collect();
    assert!(
        phantom.is_empty(),
        "these `Opt::X` references name variants absent from the registry: {phantom:#?}",
    );
}

// ---------------------------------------------------------------------------
// 3. The linked/dependent invariant: disabling any optimization provably disables
//    its transitive dependents through normalize().
// ---------------------------------------------------------------------------
#[test]
fn requires_edges_cascade_off_through_normalize() {
    let trans = transitive_requires();

    // Guard against a vacuous pass: the graph really has dependency edges.
    let edge_count: usize = trans.values().map(|s| s.len()).sum();
    assert!(
        edge_count > 0,
        "expected a non-empty requires graph — the cascade check would be vacuous",
    );

    let name_to_opt: BTreeMap<String, Opt> = REGISTRY
        .iter()
        .map(|m| (variant_name(m.opt), m.opt))
        .collect();

    // For every (child, ancestor-it-requires) pair, turning the ancestor off and
    // normalizing must turn the child off too.
    for (child_name, ancestors) in &trans {
        let child = name_to_opt[child_name];
        for ancestor_name in ancestors {
            let ancestor = name_to_opt[ancestor_name];
            let mut cfg = OptimizationConfig::all_on();
            cfg.set(ancestor, false);
            cfg.normalize();
            assert!(
                !cfg.is_on(child),
                "disabling {ancestor_name} must cascade off its dependent \
                 {child_name} (it transitively requires {ancestor_name}), but \
                 normalize() left {child_name} on",
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 4. Default is everything-on, and each optimization is individually addressable
//    by its `## No <keyword>` and independently toggleable.
// ---------------------------------------------------------------------------
#[test]
fn default_is_all_on_and_each_opt_is_individually_addressable() {
    let default = OptimizationConfig::default();
    for m in REGISTRY {
        if m.default_on {
            assert!(
                default.is_on(m.opt),
                "default_on optimization {:?} is not on in the default config",
                m.opt,
            );
        }

        // `## No <keyword>` round-trips to exactly this opt.
        let kw = m.keyword;
        assert!(
            !kw.is_empty() && kw == kw.to_ascii_lowercase() && !kw.contains(char::is_whitespace),
            "keyword {kw:?} must be a lowercase single token",
        );
        assert_eq!(
            by_keyword(kw),
            Some(m.opt),
            "`## No {kw}` must resolve to {:?}",
            m.opt,
        );

        // Each opt is independently switchable off from the all-on baseline.
        let off = OptimizationConfig::all_on().disable(m.opt);
        assert!(
            !off.is_on(m.opt),
            "{:?} could not be individually disabled",
            m.opt,
        );
    }

    // The all-on baseline is a normalize fixed point (every requirement satisfied).
    let mut cfg = OptimizationConfig::default();
    assert!(
        cfg.normalize().is_empty(),
        "the default (all-on) config must be stable under normalize()",
    );
}

// ---------------------------------------------------------------------------
// 5. preempts edges keep both optimizations live (distinct from conflicts).
// ---------------------------------------------------------------------------
#[test]
fn preempts_edges_are_coherent_and_keep_both_enabled() {
    let default = OptimizationConfig::default();
    for m in REGISTRY {
        for &target in m.preempts {
            assert_ne!(
                variant_name(m.opt),
                variant_name(target),
                "{:?} preempts itself",
                m.opt,
            );
            // preempts (unlike conflicts) leaves BOTH enabled: the preempted opt is
            // the fallback for instances the preemptor declined.
            assert!(
                default.is_on(m.opt) && default.is_on(target),
                "preempts edge {:?} ⊐ {target:?} must keep both enabled by default",
                m.opt,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Teeth: prove the guards actually fire (trace-free — the pure predicates are
// exercised against seeded violations, so a green tree is never a vacuous one).
// ---------------------------------------------------------------------------
#[cfg(test)]
mod teeth {
    use super::*;

    fn dead_toggles(expected: &BTreeSet<String>, gated: &BTreeSet<String>) -> Vec<String> {
        expected.difference(gated).cloned().collect()
    }

    #[test]
    fn dead_toggle_detector_fires_on_a_missing_gate() {
        let expected = all_variant_names();
        let mut gated = expected.clone();
        // Simulate an optimization that lost its gate.
        gated.remove("Simd");
        let dead = dead_toggles(&expected, &gated);
        assert_eq!(dead, vec!["Simd".to_string()]);

        // …and stays silent when coverage is complete.
        assert!(dead_toggles(&expected, &expected).is_empty());
    }

    #[test]
    fn line_scanner_extracts_gated_opt_names() {
        assert_eq!(
            opt_refs_on_line("if self.cfg.is_on(Opt::Narrow) && other {"),
            vec!["Narrow".to_string()],
        );
        assert_eq!(
            opt_refs_on_line("run_traced(Opt::Comptime, current, |c| ctfe(c))"),
            vec!["Comptime".to_string()],
        );
        assert!(opt_refs_on_line("// just a comment about Opt::Fuse").len() == 1);
        assert_eq!(
            opt_refs_on_line("crate::optimize::Opt::Narrow"),
            vec!["Narrow".to_string()],
        );
        assert!(
            opt_refs_on_line("logicaffeine_data::LogosOpt::from(present)").is_empty(),
            "a type whose name ends in `Opt` is not the registry"
        );
    }

    #[test]
    fn cascade_graph_is_non_empty() {
        let trans = transitive_requires();
        // Scalarize requires Cse; Interleave requires Scalarize (⇒ Cse) — real edges.
        assert!(trans["Interleave"].contains("Cse"));
        assert!(trans["Scalarize"].contains("Cse"));
    }
}
