//! The optimization SYSTEM's relationship graph, encoded once in the registry
//! and proven against real compiler behavior — so the declared `requires` /
//! `preempts` edges and the (necessarily sprinkled) per-instance fallback code
//! can never drift apart. Each test is GENERIC over the registry: it derives its
//! obligations from the encoded edges, so a new edge is covered automatically and
//! a mis-declared edge fails here.

use logicaffeine_compile::compile::{optimization_preemptions, optimizations_fired};
use logicaffeine_compile::optimization::{by_keyword, decorate_source, Opt, OptimizationConfig, REGISTRY};
use std::collections::BTreeSet;

/// Programs that, together, fire every optimization which participates in a
/// relationship edge (so every edge has a witness). Benchmarks read from disk +
/// a few purpose-built snippets for opts no benchmark exercises.
fn witness_programs() -> Vec<String> {
    use std::fs;
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../benchmarks/programs");
    let mut out: Vec<String> = fs::read_dir(dir)
        .expect("benchmarks/programs")
        .filter_map(|e| {
            let lg = e.unwrap().path().join("main.lg");
            lg.exists().then(|| fs::read_to_string(&lg).unwrap())
        })
        .collect();
    // Deterministic order: `read_dir` yields entries in unstable filesystem order,
    // but the per-program shards below partition this list by index, so every shard
    // process must see the identical ordering or coverage would drift. Sorting the
    // disk programs (snippets appended after, at fixed positions) makes the list
    // reproducible. Order is irrelevant to the aggregating tests, so this is safe.
    out.sort();
    // Snippets for relationships no benchmark exercises (closures, e-graph copies,
    // memory-resident AoS, deep recursion the comptime path subsumes).
    let hdr = "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n";
    out.push(format!("{hdr}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\nLet mutable xs be a new Seq of Int.\nLet mutable ys be a new Seq of Int.\nPush 1 to xs.\nPush 2 to ys.\nPush 3 to xs.\nPush 4 to ys.\nPush 5 to xs.\nPush 6 to ys.\nPush 7 to xs.\nPush 8 to ys.\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most n:\n    Set acc to acc + item ((i % 4) + 1) of xs + item ((i % 4) + 1) of ys.\n    Set i to i + 1.\nShow acc.\n"));
    out.push("## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nLet result be factorial(10).\nShow result.\n".to_string());
    out.push(format!("{hdr}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\nLet mutable m be a new Map of Int to Int.\nLet mutable i be 1.\nWhile i is at most n:\n    Set m at (i % 100) to (i % 100).\n    Set i to i + 1.\nLet mutable hit be 0.\nIf m contains 5:\n    Set hit to 1.\nShow hit.\n"));
    out
}

fn keyword(opt: Opt) -> &'static str {
    opt.meta().keyword
}

// ── Sharding ─────────────────────────────────────────────────────────────────
// The two PER-PROGRAM differentials below (`relationship_tree…` and
// `optimization_dependencies…`) re-run the optimizer over every witness program
// and are multi-minute; nextest parallelizes ACROSS tests, never within one. Each
// program is checked independently of the others (no cross-program aggregation),
// so each test is split into independent `#[test]` shards over the SAME
// deterministic `witness_programs()` list, selected by `index % OPT_GRAPH_SHARDS`.
// The shards together check the identical program set the original single test
// did — `opt_graph_partition_tiles_programs` proves the tiling.
//
// The AGGREGATING tests here are deliberately NOT sharded: `preempts_edges…` must
// union the observed precedence edges across ALL programs to detect a *phantom*
// declared edge (one no program witnesses), so it has no per-program decomposition.
// It relies on the `test-opt` build profile for speed instead.

/// Number of parallel shards the per-program differentials are split across.
const OPT_GRAPH_SHARDS: usize = 8;

/// The indices into `witness_programs()` owned by `shard` — every `i` with
/// `i % OPT_GRAPH_SHARDS == shard`. Derived from the one shared (now deterministic)
/// list, so the partition is identical in every shard process.
fn opt_graph_shard(shard: usize) -> impl Iterator<Item = usize> {
    (0..witness_programs().len()).filter(move |i| i % OPT_GRAPH_SHARDS == shard)
}

/// Coverage guard: the `OPT_GRAPH_SHARDS` shards tile `witness_programs()` exactly
/// — every program owned by exactly one shard, every shard non-empty. Proves the
/// shard fns together check the identical program set the originals did.
#[test]
fn opt_graph_partition_tiles_programs() {
    let len = witness_programs().len();
    let mut hits = vec![0u32; len];
    for shard in 0..OPT_GRAPH_SHARDS {
        for i in opt_graph_shard(shard) {
            hits[i] += 1;
        }
    }
    assert!(
        hits.iter().all(|&h| h == 1),
        "witness programs not tiled exactly once by {OPT_GRAPH_SHARDS} shards: {hits:?}"
    );
    for shard in 0..OPT_GRAPH_SHARDS {
        assert!(opt_graph_shard(shard).count() > 0, "opt_graph shard {shard} owns no programs");
    }
}

/// `requires` invariant: if optimization A declares it requires B, then on a
/// program where A fires, turning B off must STOP A from firing. This proves the
/// encoded dependency matches what `normalize` + the pass preconditions actually
/// do — for every edge, automatically.
#[test]
fn requires_edges_match_behavior() {
    let progs = witness_programs();
    for m in REGISTRY {
        if m.requires.is_empty() {
            continue;
        }
        // A program where this optimization actually fires.
        let witness = progs
            .iter()
            .find(|src| optimizations_fired(src).contains(&m.keyword));
        let witness = match witness {
            Some(w) => w,
            None => panic!(
                "no witness program fires `{}` (it has a requires edge that can't be exercised) — add one",
                m.keyword
            ),
        };
        for &req in m.requires {
            let off = optimizations_fired(&decorate_source(witness, &[keyword(req)]));
            assert!(
                !off.contains(&m.keyword),
                "`{}` requires `{}`, but it still fired with `{}` disabled — the encoded dependency is not enforced",
                m.keyword,
                keyword(req),
                keyword(req)
            );
        }
    }
}

/// `preempts` invariant — the tie between the registry and the compiler's real
/// behavior. The set of precedence edges the compiler actually exhibits must be
/// EXACTLY the set the registry declares, where "exhibits" is the UNION of two
/// honest sources:
///   - **traced** (`mark_preempted`): the explicit skips, recorded at the exact
///     code site — this catches even the *latent* conflicts that never change the
///     final fired set (the differential is blind to those);
///   - **differential**: emergent subsumption (disable a fired opt X, and Y newly
///     fires) — for which there is no skip site to tag.
/// No exhibited precedence may go undeclared (the registry lying by omission), and
/// no declared edge may be un-exhibited on the witness corpus (a phantom). Generic
/// over the registry, so a conflict in the code is forced into the declaration.
#[test]
fn preempts_edges_match_observed_precedence() {
    let progs = witness_programs();

    // What the registry DECLARES.
    let mut declared: BTreeSet<(&'static str, &'static str)> = BTreeSet::new();
    for m in REGISTRY {
        for &loser in m.preempts {
            declared.insert((m.keyword, keyword(loser)));
        }
    }

    // What the compiler ACTUALLY exhibits = traced ∪ differential.
    let mut observed: BTreeSet<(&'static str, &'static str)> = BTreeSet::new();
    for src in &progs {
        // Traced explicit preemptions — the conflicts the code performs, at source.
        for (w, l) in optimization_preemptions(src) {
            observed.insert((w, l));
        }
        // Differential — emergent subsumption with no skip site to tag.
        let base: BTreeSet<&'static str> = optimizations_fired(src).into_iter().collect();
        for &x in &base {
            let off: BTreeSet<&'static str> =
                optimizations_fired(&decorate_source(src, &[x])).into_iter().collect();
            for &y in off.difference(&base) {
                if y != x {
                    observed.insert((x, y));
                }
            }
        }
    }

    let undeclared: Vec<_> = observed.difference(&declared).collect();
    let phantom: Vec<_> = declared.difference(&observed).collect();
    assert!(
        undeclared.is_empty(),
        "the compiler exhibits precedence the registry does not declare: {undeclared:?} — declare them in `preempts`"
    );
    assert!(
        phantom.is_empty(),
        "the registry declares precedence the compiler never exhibits on the witness corpus (phantom): {phantom:?} — remove them or add a witness program"
    );
}

/// The page's tree is built by `relationship_tree` from one all-on trace — no
/// differential walking. This locks that deterministic derivation against REAL
/// compiles across the corpus: for every witness program, the tree the page would
/// render must (a) mark every fired opt `Fired`, (b) include every preemption loser
/// with the winner that beat it, (c) have no orphans (each node's `requires`-parents
/// are present), and (d) contain EXACTLY the in-play set `fired ∪ losers ∪
/// requires-closure` — nothing more, nothing less. So the in-crate derivation the UI
/// trusts can never drift from what a traced compile actually reports.
fn run_relationship_tree_shard(shard: usize) {
    use logicaffeine_compile::compile::{optimization_dependencies, optimization_preemptions};
    use logicaffeine_compile::optimization::{by_keyword, relationship_tree, OptRole};

    let progs = witness_programs();
    let mut owned = 0usize;
    for i in opt_graph_shard(shard) {
        owned += 1;
        let src = &progs[i];
        let fired: Vec<Opt> = optimizations_fired(src)
            .iter()
            .filter_map(|kw| by_keyword(kw))
            .collect();
        let preempted: Vec<(Opt, Opt)> = optimization_preemptions(src)
            .iter()
            .filter_map(|(w, l)| Some((by_keyword(w)?, by_keyword(l)?)))
            .collect();
        let dependencies: Vec<(Opt, Opt)> = optimization_dependencies(src)
            .iter()
            .filter_map(|(d, x)| Some((by_keyword(d)?, by_keyword(x)?)))
            .collect();
        let tree = relationship_tree(&fired, &preempted, &dependencies);
        let present: BTreeSet<Opt> = tree.iter().map(|n| n.opt).collect();

        for &f in &fired {
            let node = tree.iter().find(|n| n.opt == f).expect("fired opt must be in the tree");
            assert_eq!(node.role, OptRole::Fired, "{f:?} fired but is not Fired-role");
        }
        for &(w, l) in &preempted {
            assert!(present.contains(&w), "winner {w:?} missing from tree");
            let node = tree.iter().find(|n| n.opt == l).expect("loser must be in the tree");
            assert!(
                node.preempted_by.contains(&w) || fired.contains(&l),
                "{l:?} was beaten by {w:?} but the node does not record it"
            );
        }
        for n in &tree {
            for &req in &n.requires {
                assert!(present.contains(&req), "orphan: {:?} requires {req:?} which is absent", n.opt);
            }
        }
        // The in-play set, closed under `requires`.
        let mut expected: BTreeSet<Opt> = fired.iter().copied().collect();
        for &(_, l) in &preempted {
            expected.insert(l);
        }
        loop {
            let snapshot: Vec<Opt> = expected.iter().copied().collect();
            let mut grew = false;
            for o in snapshot {
                for &r in o.meta().requires {
                    grew |= expected.insert(r);
                }
            }
            if !grew {
                break;
            }
        }
        assert_eq!(present, expected, "tree must be exactly the in-play set for this program");
    }
    assert!(owned > 0, "relationship_tree shard {shard}/{OPT_GRAPH_SHARDS} owns no programs");
}

macro_rules! relationship_tree_shards {
    ($($name:ident => $idx:expr;)+) => {
        $(
            /// One shard of the relationship-tree derivation lock — see `run_relationship_tree_shard`.
            #[test] fn $name() { run_relationship_tree_shard($idx); }
        )+
    };
}
relationship_tree_shards! {
    relationship_tree_reflects_real_traces_without_orphans_s0 => 0;
    relationship_tree_reflects_real_traces_without_orphans_s1 => 1;
    relationship_tree_reflects_real_traces_without_orphans_s2 => 2;
    relationship_tree_reflects_real_traces_without_orphans_s3 => 3;
    relationship_tree_reflects_real_traces_without_orphans_s4 => 4;
    relationship_tree_reflects_real_traces_without_orphans_s5 => 5;
    relationship_tree_reflects_real_traces_without_orphans_s6 => 6;
    relationship_tree_reflects_real_traces_without_orphans_s7 => 7;
}

/// The dependency mirror of the preempts invariant, and the other half of
/// "evaluate with all optimizations on, see what ends up happening." The preempts
/// differential captures *blockers* (disable X, Y NEWLY fires). This locks the
/// *dependency* extraction: disabling a fired X that makes fired Y STOP means Y
/// depended on X. Stops a declared `requires`-cascade already explains (`normalize`
/// disables Y anyway — e.g. `symmetry`/`specialize`) belong to the static graph;
/// the REMAINDER are emergent, program-specific dependencies (DCE only had work
/// because scalarization produced dead code). `optimization_dependencies` must
/// return EXACTLY that remainder for every witness — so the per-program graph that
/// gets baked into the benchmark data is the canonical one, neither inventing nor
/// dropping an edge. Generic over the corpus.
fn run_optimization_dependencies_shard(shard: usize) {
    use logicaffeine_compile::compile::optimization_dependencies;
    let progs = witness_programs();
    let mut owned = 0usize;
    for i in opt_graph_shard(shard) {
        owned += 1;
        let src = &progs[i];
        // Recompute the canonical per-program dependency set: single-disable stops
        // minus the static `requires`-cascade that `normalize` already explains.
        let base: BTreeSet<&'static str> = optimizations_fired(src).into_iter().collect();
        let mut expected: BTreeSet<(&'static str, &'static str)> = BTreeSet::new();
        for &x in &base {
            let xopt = match by_keyword(x) {
                Some(o) => o,
                None => continue,
            };
            let mut cfg = OptimizationConfig::all_on();
            cfg.set(xopt, false);
            cfg.normalize();
            let off: BTreeSet<&'static str> =
                optimizations_fired(&decorate_source(src, &[x])).into_iter().collect();
            for &y in base.difference(&off) {
                if y == x {
                    continue;
                }
                let yopt = match by_keyword(y) {
                    Some(o) => o,
                    None => continue,
                };
                if cfg.is_on(yopt) {
                    expected.insert((y, x));
                }
            }
        }
        let got: BTreeSet<(&'static str, &'static str)> =
            optimization_dependencies(src).into_iter().collect();
        assert_eq!(
            got, expected,
            "optimization_dependencies must equal the per-program stop-differential \
             (minus the static requires-cascade) — the canonical baked graph"
        );
    }
    assert!(owned > 0, "optimization_dependencies shard {shard}/{OPT_GRAPH_SHARDS} owns no programs");
}

macro_rules! optimization_dependencies_shards {
    ($($name:ident => $idx:expr;)+) => {
        $(
            /// One shard of the per-program dependency-extraction lock — see `run_optimization_dependencies_shard`.
            #[test] fn $name() { run_optimization_dependencies_shard($idx); }
        )+
    };
}
optimization_dependencies_shards! {
    optimization_dependencies_are_exactly_the_unexplained_per_program_stops_s0 => 0;
    optimization_dependencies_are_exactly_the_unexplained_per_program_stops_s1 => 1;
    optimization_dependencies_are_exactly_the_unexplained_per_program_stops_s2 => 2;
    optimization_dependencies_are_exactly_the_unexplained_per_program_stops_s3 => 3;
    optimization_dependencies_are_exactly_the_unexplained_per_program_stops_s4 => 4;
    optimization_dependencies_are_exactly_the_unexplained_per_program_stops_s5 => 5;
    optimization_dependencies_are_exactly_the_unexplained_per_program_stops_s6 => 6;
    optimization_dependencies_are_exactly_the_unexplained_per_program_stops_s7 => 7;
}
