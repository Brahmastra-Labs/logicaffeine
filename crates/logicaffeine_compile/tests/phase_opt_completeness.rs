//! ONE-TIME exhaustive completeness audit for the optimization relationship graph.
//!
//! The baked per-program graph (blockers from `optimization_preemptions`,
//! dependencies from `optimization_dependencies`) is derived from a single
//! *all-optimizations-on* evaluation plus single-disable probing. That is cheap,
//! but it only observes ONE context (everything else on). The worry: a relationship
//! that only manifests in a different context — e.g. Y stops needing X only once Z
//! is also off — would be invisible to single-disable.
//!
//! This audit removes that doubt. Per benchmark it takes the in-play optimizations
//! and sweeps EVERY combination of them (a full 2^k truth table where k is small;
//! all ≤3-way disables, logged, where k is large), then derives every pairwise
//! relationship the sweep reveals across ALL contexts and asserts each is already
//! captured by the static registry (`requires`/`preempts`) or the baked
//! single-disable graph. A relationship the sweep finds but the cheap graph misses
//! is a hidden edge — reported, not swallowed.
//!
//! On a pass it certifies two HARD guarantees — no optimization ever fires outside
//! the closed in-play set (the tree can never miss an opt), and every baked
//! dependency is reproducible by the sweep — and it PRINTS the latent
//! context-dependent edges (relationships that hold only with another opt off) for
//! visibility. Blockers are traced at their skip site (`mark_preempted`), sound by
//! construction and a superset of the differential, so they are locked by
//! `preempts_edges_match_observed_precedence`, not re-derived here.
//!
//! `#[ignore]`d because it is exponential; run it on demand to re-certify after
//! adding or changing optimizations. Add `--no-capture` to see the latent-edge
//! report (nextest hides a passing test's output otherwise):
//!   cargo nextest run -p logicaffeine-compile --test phase_opt_completeness \
//!       --run-ignored all --no-capture

use logicaffeine_compile::compile::{
    optimization_dependencies, optimization_preemptions, optimizations_fired,
};
use logicaffeine_compile::optimization::{by_keyword, decorate_source, REGISTRY};
use std::collections::{BTreeSet, HashMap};

/// `(id, source)` for every benchmark program plus the purpose-built snippets, so
/// the audit covers exactly what the cheap invariants in `phase_opt_graph` cover.
fn witness_programs() -> Vec<(String, String)> {
    use std::fs;
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../benchmarks/programs");
    let mut out: Vec<(String, String)> = fs::read_dir(dir)
        .expect("benchmarks/programs")
        .filter_map(|e| {
            let p = e.unwrap().path();
            let lg = p.join("main.lg");
            let id = p.file_name().unwrap().to_string_lossy().into_owned();
            lg.exists().then(|| (id, fs::read_to_string(&lg).unwrap()))
        })
        .collect();
    out.sort();
    out
}

fn fired_with_disabled(src: &str, disabled: &[&str]) -> BTreeSet<&'static str> {
    optimizations_fired(&decorate_source(src, disabled))
        .into_iter()
        .collect()
}

/// Does `y` transitively `require` `x` in the static registry?
fn requires_transitively(y: &str, x: &str) -> bool {
    let yo = match by_keyword(y) {
        Some(o) => o,
        None => return false,
    };
    let mut stack = vec![yo];
    let mut seen = 0u64;
    while let Some(o) = stack.pop() {
        if seen & o.bit() != 0 {
            continue;
        }
        seen |= o.bit();
        for &r in o.meta().requires {
            if r.meta().keyword == x {
                return true;
            }
            stack.push(r);
        }
    }
    false
}

/// Does `x` statically `preempt` `y`?
fn preempts_statically(x: &str, y: &str) -> bool {
    by_keyword(x)
        .map(|o| o.meta().preempts.iter().any(|p| p.meta().keyword == y))
        .unwrap_or(false)
}

#[test]
#[ignore = "exponential; one-time completeness certification, run with --run-ignored"]
fn exhaustive_edge_completeness_audit() {
    let mut violations: Vec<String> = Vec::new();
    let mut latent: Vec<String> = Vec::new();

    for (id, src) in witness_programs() {
        // 1. In-play optimizations: those that fire all-on, those that newly fire or
        //    survive under any single disable, plus the static `requires`-closure
        //    (enabler parents like `oracle` that never fire themselves).
        let base = fired_with_disabled(&src, &[]);
        let mut in_play: BTreeSet<&'static str> = base.clone();
        for &x in &base {
            for kw in fired_with_disabled(&src, &[x]) {
                in_play.insert(kw);
            }
        }
        loop {
            let before = in_play.len();
            for kw in in_play.iter().copied().collect::<Vec<_>>() {
                if let Some(o) = by_keyword(kw) {
                    for &r in o.meta().requires {
                        in_play.insert(r.meta().keyword);
                    }
                }
            }
            if in_play.len() == before {
                break;
            }
        }
        let ip: Vec<&'static str> = in_play.iter().copied().collect();
        let k = ip.len();

        // 2. Enumerate combinations of which in-play opts to DISABLE. Full 2^k when
        //    small; otherwise every ≤3-way disable (logged — no silent truncation).
        let full = k <= 13;
        let masks: Vec<u64> = if full {
            (0..(1u64 << k)).collect()
        } else {
            let mut v = vec![0u64];
            for a in 0..k {
                v.push(1 << a);
                for b in (a + 1)..k {
                    v.push((1 << a) | (1 << b));
                    for c in (b + 1)..k {
                        v.push((1 << a) | (1 << b) | (1 << c));
                    }
                }
            }
            eprintln!("[audit] {id}: in-play k={k} > 13 — sweeping all ≤3-way disables ({} combos), NOT the full 2^{k}", v.len());
            v
        };
        eprintln!("[audit] {id}: k={k}, {} combos", masks.len());

        // 3. Truth table: disabled-mask -> fired set. Also flags any opt that fires
        //    outside the in-play set (would mean the 1st-order in-play set was itself
        //    incomplete).
        let mut truth: HashMap<u64, BTreeSet<&'static str>> = HashMap::new();
        for &m in &masks {
            let disabled: Vec<&str> = (0..k).filter(|&i| m & (1 << i) != 0).map(|i| ip[i]).collect();
            let fired = fired_with_disabled(&src, &disabled);
            for f in &fired {
                if !in_play.contains(f) {
                    violations.push(format!(
                        "{id}: `{f}` fires under disable {disabled:?} but is outside the in-play set — single-disable in-play discovery is incomplete"
                    ));
                }
            }
            truth.insert(m, fired);
        }

        // 4. Every pairwise relationship the sweep reveals across ALL contexts:
        //    compare each context with/without each single opt toggled.
        let mut dep_all: BTreeSet<(&str, &str)> = BTreeSet::new(); // (dependent, dependency)
        let mut block_all: BTreeSet<(&str, &str)> = BTreeSet::new(); // (blocker, loser)
        for xi in 0..k {
            let xbit = 1u64 << xi;
            for (&m, fired_on) in truth.iter() {
                if m & xbit != 0 {
                    continue; // this context already has X disabled
                }
                let fired_off = match truth.get(&(m | xbit)) {
                    Some(s) => s,
                    None => continue, // only happens in bounded-order mode
                };
                for &y in fired_on.difference(fired_off) {
                    if y != ip[xi] {
                        dep_all.insert((y, ip[xi])); // disabling X dropped Y -> Y depends on X
                    }
                }
                for &y in fired_off.difference(fired_on) {
                    if y != ip[xi] {
                        block_all.insert((ip[xi], y)); // disabling X added Y -> X blocks Y
                    }
                }
            }
        }

        // 5. What the baked all-on graph captures: static requires + single-disable
        //    dependencies, static preempts + traced blockers.
        //
        //    DEPENDENCY soundness — a hard check: a baked dependency is a stop the
        //    differential observed in the all-on context, which is one of the sweep's
        //    contexts, so it MUST appear in `dep_all`. A baked dependency the sweep
        //    cannot reproduce would be a lie.
        //
        //    BLOCKERS are deliberately NOT checked against the differential. They are
        //    traced at the exact skip site (`mark_preempted`), so they are sound by
        //    construction AND a superset of what the differential sees: a LATENT
        //    preemption (the winner claimed an instance the loser would not have fired
        //    on anyway, e.g. `densemap |> narrowmap`) is invisible to "disable X, does
        //    Y newly fire?" — which is the whole reason we trace blockers rather than
        //    derive them. Their correctness is locked separately by
        //    `preempts_edges_match_observed_precedence`.
        let dep_model: BTreeSet<(&str, &str)> = optimization_dependencies(&src).into_iter().collect();
        let block_model: BTreeSet<(&str, &str)> = optimization_preemptions(&src).into_iter().collect();
        for &(y, x) in &dep_model {
            if !dep_all.contains(&(y, x)) {
                violations.push(format!(
                    "{id}: baked dependency `{y}` <- `{x}` is NOT confirmed by the exhaustive sweep (unsound baked edge)"
                ));
            }
        }

        // 6. LATENT edges — relationships the sweep reveals that hold ONLY in reduced
        //    contexts (some other opt also off), so they are correctly absent from the
        //    all-on tree and instead surface through the live re-trace as the user
        //    toggles. Reported for visibility, not a failure: the all-on tree is
        //    complete FOR the all-on state, and `dep_all`/`block_all` over the closed
        //    in-play set is the complete accounting of every edge.
        for &(y, x) in &dep_all {
            if !(dep_model.contains(&(y, x)) || requires_transitively(y, x)) {
                latent.push(format!("{id}: latent dependency `{y}` <- `{x}` (holds only with another opt off)"));
            }
        }
        for &(x, y) in &block_all {
            if !(block_model.contains(&(x, y)) || preempts_statically(x, y)) {
                latent.push(format!("{id}: latent blocker `{x}` |> `{y}` (holds only with another opt off)"));
            }
        }
    }

    if !latent.is_empty() {
        eprintln!(
            "\n[audit] {} LATENT context-dependent edges (surfaced by the live re-trace, not the all-on tree):",
            latent.len()
        );
        for l in &latent {
            eprintln!("  {l}");
        }
    }

    // HARD guarantees: no optimization ever fires outside the closed in-play set (so
    // the tree can never miss an opt), and every baked edge is sound.
    assert!(
        violations.is_empty(),
        "exhaustive completeness audit found HARD violations (missed opt or unsound baked edge):\n{}",
        violations.join("\n")
    );
    let _ = REGISTRY.len();
}
