//! Optimization firing-trace: the compiler records WHICH optimizations actually
//! fired for a given compile. These tests pin the contract:
//!   1. tracing never changes the generated code (soundness),
//!   2. a fired optimization was enabled (subset invariant),
//!   3. programs with known wins report exactly those optimizations,
//!   4. re-tracing under a different toggle state reflects the change,
//!   5. tracing is opt-in — the plain path records nothing.

use logicaffeine_compile::compile::{
    compile_program_full, compile_program_traced, compile_to_rust, optimizations_fired,
    optimizations_fired_run, optimizations_fired_vm,
};
use logicaffeine_compile::optimization::{decorate_source, Opt, OptimizationConfig, REGISTRY};

/// A spread of benchmark programs, exercising the recursion, float, sort, array,
/// and hash-map clusters so the invariants are checked across optimization kinds.
const PROGRAMS: &[(&str, &str)] = &[
    ("nqueens", include_str!("../../../benchmarks/programs/nqueens/main.lg")),
    ("fib", include_str!("../../../benchmarks/programs/fib/main.lg")),
    ("mandelbrot", include_str!("../../../benchmarks/programs/mandelbrot/main.lg")),
    ("nbody", include_str!("../../../benchmarks/programs/nbody/main.lg")),
    ("quicksort", include_str!("../../../benchmarks/programs/quicksort/main.lg")),
    ("two_sum", include_str!("../../../benchmarks/programs/two_sum/main.lg")),
    ("gcd", include_str!("../../../benchmarks/programs/gcd/main.lg")),
    ("collatz", include_str!("../../../benchmarks/programs/collatz/main.lg")),
    ("mergesort", include_str!("../../../benchmarks/programs/mergesort/main.lg")),
    ("string_search", include_str!("../../../benchmarks/programs/string_search/main.lg")),
    ("pi_leibniz", include_str!("../../../benchmarks/programs/pi_leibniz/main.lg")),
];

fn program(name: &str) -> &'static str {
    PROGRAMS.iter().find(|(n, _)| *n == name).unwrap().1
}

fn nqueens() -> &'static str {
    program("nqueens")
}

/// Soundness: a traced compile must emit byte-identical Rust to the plain
/// compile. Instrumentation observes; it must never alter the output.
#[test]
fn traced_output_matches_plain() {
    for (name, src) in PROGRAMS {
        let plain = compile_to_rust(src).unwrap_or_else(|e| panic!("{name}: plain compile failed: {e:?}"));
        let (out, _fired) =
            compile_program_traced(src).unwrap_or_else(|e| panic!("{name}: traced compile failed: {e:?}"));
        if plain != out.rust_code {
            // A mismatch must come from tracing, not from inherent codegen
            // non-determinism. If two plain compiles already differ, the program
            // is non-deterministic (a separate concern, owned by
            // phase_codegen_determinism) and this comparison can't isolate
            // tracing — so only fail when codegen is deterministic here.
            let plain2 = compile_to_rust(src).unwrap();
            assert_eq!(plain, plain2, "{name}: codegen is non-deterministic (separate from tracing)");
            assert_eq!(plain, out.rust_code, "{name}: tracing changed the generated Rust");
        }
    }
}

/// Subset invariant: an optimization can only be reported as fired if it was
/// enabled in the effective config. `fired & !enabled == 0`.
#[test]
fn fired_is_subset_of_enabled() {
    let enabled = OptimizationConfig::from_env();
    for (name, src) in PROGRAMS {
        let (_out, fired) = compile_program_traced(src).unwrap();
        assert_eq!(
            fired.bits() & !enabled.bits(),
            0,
            "{name}: fired {:?} includes a disabled optimization",
            fired.keywords()
        );
    }
}

/// An optimized program fires *something* — the trace is not silently empty.
#[test]
fn optimized_program_fires_at_least_one() {
    let (_out, fired) = compile_program_traced(nqueens()).unwrap();
    assert!(!fired.is_empty(), "nqueens fired nothing — instrumentation is not recording");
}

/// nqueens's documented wins must show up: reflection symmetry breaking, the
/// popcount base-case collapse, and recursion unrolling are all visible in its
/// generated Rust, so the trace must report them.
#[test]
fn nqueens_reports_its_signature_optimizations() {
    let (_out, fired) = compile_program_traced(nqueens()).unwrap();
    for opt in [Opt::Symmetry, Opt::Popcount, Opt::Unfold] {
        assert!(
            fired.fired(opt),
            "nqueens must report {:?} as fired; got {:?}",
            opt,
            fired.keywords()
        );
    }
}

/// Codegen-time optimizations (decisions made during Rust emission, not AST
/// passes) must be traced too. Each assertion is anchored to a signature visible
/// in the benchmark's committed generated Rust: borrow inference (`&mut [T]`
/// params), dense maps (`LogosDense*`), SIMD search kernels, and oracle-proven
/// `get_unchecked` indexing.
#[test]
fn codegen_time_optimizations_are_traced() {
    let cases: &[(&str, &str)] = &[
        ("quicksort", "borrow"),
        ("two_sum", "densemap"),
        ("string_search", "simd"),
        ("mergesort", "unchecked"),
    ];
    for (prog, keyword) in cases {
        let fired = optimizations_fired(program(prog));
        assert!(
            fired.contains(keyword),
            "{prog} must report codegen optimization {keyword:?} as fired; got {fired:?}"
        );
    }
}

/// Re-tracing under a different toggle state reflects it: with symmetry on it
/// fires, with `## No symmetry` it does not — and the generated Rust changes.
#[test]
fn retrace_reflects_disabling_an_optimization() {
    let on = optimizations_fired(nqueens());
    assert!(on.contains(&"symmetry"), "symmetry should fire by default; got {on:?}");

    let off_src = decorate_source(nqueens(), &["symmetry"]);
    let off = optimizations_fired(&off_src);
    assert!(!off.contains(&"symmetry"), "symmetry must not fire when disabled; got {off:?}");

    assert_ne!(
        compile_to_rust(nqueens()).unwrap(),
        compile_to_rust(&off_src).unwrap(),
        "disabling symmetry must change the generated Rust"
    );
}

/// Optimizations no benchmark exercises, fired by small purpose-built programs:
/// CSE folds a repeated subexpression, interval analysis (Oracle) folds a
/// provably-true branch, and an `Int`-keyed map narrows (NarrowMap). Each is a
/// reusable triggering program for the coverage matrix.
const HDR: &str = "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n";

fn snippet_cse() -> String {
    format!("{HDR}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\nLet a be n * n + n * n.\nShow a.\n")
}
fn snippet_oracle() -> String {
    format!("{HDR}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\nLet r be n % 5.\nIf r is less than 5:\n    Show r.\nOtherwise:\n    Show 0.\n")
}
fn snippet_narrowmap() -> String {
    format!("{HDR}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\nLet mutable m be a new Map of Int to Int.\nLet mutable i be 1.\nWhile i is at most n:\n    Set m at (i % 100) to (i % 100).\n    Set i to i + 1.\nLet mutable hit be 0.\nIf m contains 5:\n    Set hit to 1.\nShow hit.\n")
}

#[test]
fn snippet_triggered_optimizations_fire_and_gate() {
    for (kw, src) in [("cse", snippet_cse()), ("oracle", snippet_oracle()), ("narrowmap", snippet_narrowmap())] {
        let fired = optimizations_fired(&src);
        assert!(fired.contains(&kw), "snippet for `{kw}` must fire it (AOT); got {fired:?}");
        let off = optimizations_fired(&decorate_source(&src, &[kw]));
        assert!(!off.contains(&kw), "disabling `{kw}` must remove it; got {off:?}");
    }
}

/// The VM-bytecode-compile-time optimizations (constant-divisor magic division,
/// VM `i32` narrowing) fire and gate via the VM trace — the third path, which the
/// AOT and run-path traces cannot see.
#[test]
fn vm_compile_optimizations_fire_and_gate() {
    let fastdiv = format!("{HDR}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most n:\n    Set acc to acc + i / 7.\n    Set i to i + 1.\nShow acc.\n");
    let fd = optimizations_fired_vm(&fastdiv);
    assert!(fd.contains(&"fastdiv"), "VM trace should fire fastdiv; got {fd:?}");
    assert!(
        !optimizations_fired_vm(&decorate_source(&fastdiv, &["fastdiv"])).contains(&"fastdiv"),
        "disabling fastdiv must remove it from the VM trace"
    );

    let array_fill = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../benchmarks/programs/array_fill/main.lg"
    ))
    .unwrap();
    let nv = optimizations_fired_vm(&array_fill);
    assert!(nv.contains(&"narrowvm"), "VM trace should fire narrowvm; got {nv:?}");
    assert!(
        !optimizations_fired_vm(&decorate_source(&array_fill, &["narrowvm"])).contains(&"narrowvm"),
        "disabling narrowvm must remove it from the VM trace"
    );
}

// The five niche/advanced optimizations no benchmark exercises. Each has a
// purpose-built trigger program: deforestation fuses a producer/consumer loop;
// the e-graph collapses `length of (copy of xs)`; AoS interleaving fuses two
// co-indexed arrays read by variable index; a dynamic-arg closure defunctionalizes.
fn snippet_fuse() -> String {
    "## Main\nLet mutable items be a new Seq of Int.\nPush 1 to items.\nPush 2 to items.\nPush 3 to items.\nLet mutable doubled be a new Seq of Int.\nRepeat for x in items:\n    Push x * 2 to doubled.\nLet mutable total be 0.\nRepeat for y in doubled:\n    Set total to total + y.\nShow total.\n".to_string()
}
fn snippet_saturate() -> String {
    "## Main\nLet mutable seed be 42.\nLet mutable failures be 0.\nLet mutable t be 0.\nWhile t is less than 40:\n    Let mutable xs be a new Seq of Int.\n    Set seed to (seed * 1103515245 + 12345) % 2147483648.\n    Let n be seed % 13.\n    Let mutable i be 0.\n    While i is less than n:\n        Set seed to (seed * 1103515245 + 12345) % 2147483648.\n        Push seed % 100 to xs.\n        Set i to i + 1.\n    If length of (copy of xs) is not length of xs:\n        Set failures to failures + 1.\n    Set t to t + 1.\nShow failures.\n".to_string()
}
fn snippet_interleave() -> String {
    format!("{HDR}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\nLet mutable xs be a new Seq of Int.\nLet mutable ys be a new Seq of Int.\nPush 1 to xs.\nPush 2 to ys.\nPush 3 to xs.\nPush 4 to ys.\nPush 5 to xs.\nPush 6 to ys.\nPush 7 to xs.\nPush 8 to ys.\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most n:\n    Set acc to acc + item ((i % 4) + 1) of xs + item ((i % 4) + 1) of ys.\n    Set i to i + 1.\nShow acc.\n")
}
fn snippet_defunctionalize() -> String {
    format!("{HDR}## Main\nLet arguments be args().\nLet d be parseInt(item 2 of arguments).\nLet doubler be (x: Int) -> x * 2.\nShow doubler(d).\nShow doubler(d + 1).\n")
}
fn snippet_factorial() -> String {
    "## To factorial (n: Int) -> Int:\n    If n is at most 1:\n        Return 1.\n    Return n * factorial(n - 1).\n\n## Main\nLet result be factorial(10).\nShow result.\n".to_string()
}

/// The niche/advanced optimizations fire on their purpose-built programs and gate
/// off when disabled. `fuse`/`saturate`/`interleave`/`defunctionalize` fire with
/// all optimizations on; `supercompile` (a unified pass subsumed by the always-on
/// individual passes) fires once its subsumer `comptime` is disabled — that is its
/// genuine niche.
#[test]
fn niche_optimizations_fire_and_gate() {
    for (kw, src) in [
        ("fuse", snippet_fuse()),
        ("saturate", snippet_saturate()),
        ("interleave", snippet_interleave()),
        ("defunctionalize", snippet_defunctionalize()),
    ] {
        let fired = optimizations_fired(&src);
        assert!(fired.contains(&kw), "`{kw}` must fire on its trigger; got {fired:?}");
        assert!(
            !optimizations_fired(&decorate_source(&src, &[kw])).contains(&kw),
            "disabling `{kw}` must remove it; trigger still fired it"
        );
    }
    // supercompile: fires when its subsumer (comptime) is off.
    let sc = decorate_source(&snippet_factorial(), &["comptime"]);
    assert!(optimizations_fired(&sc).contains(&"supercompile"), "supercompile must fire with comptime off");
    assert!(
        !optimizations_fired(&decorate_source(&snippet_factorial(), &["comptime", "supercompile"]))
            .contains(&"supercompile"),
        "disabling supercompile must remove it even with comptime off"
    );
}

/// TEMP discovery: find PRECEDENCE edges empirically. For each program, for each
/// optimization X that fires with all-on, disable X and re-trace; any opt that
/// NEWLY fires was preempted by X (X took precedence; disabling X unlocked it).
/// Run with `--nocapture`.
#[test]
fn diagnostic_discover_preempts() {
    use std::collections::BTreeSet;
    let bench = |n: &str| std::fs::read_to_string(format!("{}/../../benchmarks/programs/{n}/main.lg", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let programs: Vec<(String, String)> = vec![
        ("two_sum".into(), bench("two_sum")),
        ("graph_bfs".into(), bench("graph_bfs")),
        ("nbody".into(), bench("nbody")),
        ("coins".into(), bench("coins")),
        ("counting_sort".into(), bench("counting_sort")),
        ("matrix_mult".into(), bench("matrix_mult")),
        ("heap_sort".into(), bench("heap_sort")),
        ("nqueens".into(), bench("nqueens")),
        ("fib".into(), bench("fib")),
        ("interleave".into(), snippet_interleave()),
        ("factorial".into(), snippet_factorial()),
    ];
    let mut edges: BTreeSet<(String, String)> = BTreeSet::new();
    for (_name, src) in &programs {
        let base: BTreeSet<&'static str> = optimizations_fired(src).into_iter().collect();
        for &x in &base {
            let off: BTreeSet<&'static str> = optimizations_fired(&decorate_source(src, &[x])).into_iter().collect();
            for &y in off.difference(&base) {
                if y != x {
                    edges.insert((x.to_string(), y.to_string()));
                }
            }
        }
    }
    eprintln!("PREEMPTS EDGES (winner -> loser):");
    for (x, y) in &edges {
        eprintln!("  {x} -> {y}");
    }
}

/// The completeness lock: EVERY optimization in the registry is accounted for.
/// 35 of the 40 are proven to fire (the union of the AOT, run-path, and VM-compile
/// traces over a representative program set covers them); the remaining 5 are
/// niche/advanced passes with no known minimal trigger (no benchmark and no simple
/// program exercises them — documented, not hidden). If a future change makes one
/// of the 5 fire, or stops one of the 35 from firing, this test fails — so the
/// coverage can never silently drift, and a newly-added optimization must declare
/// its status here.
#[test]
fn every_optimization_is_accounted_for() {
    use std::collections::BTreeSet;

    // Representative programs spanning every cluster; small first so the common
    // case is fast. Benchmarks read from disk; snippets cover the gaps.
    let bench = |name: &str| {
        std::fs::read_to_string(format!(
            "{}/../../benchmarks/programs/{name}/main.lg",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    };
    let programs: Vec<String> = [
        "fib", "ackermann", "nqueens", "gcd", "collatz", "primes", "mandelbrot",
        "pi_leibniz", "nbody", "binary_trees", "loop_sum", "matrix_mult",
        "two_sum", "collect", "coins", "knapsack", "array_fill", "graph_bfs",
        "counting_sort", "string_search", "mergesort", "quicksort", "heap_sort",
    ]
    .iter()
    .map(|n| bench(n))
    .chain([
        snippet_cse(),
        snippet_oracle(),
        snippet_narrowmap(),
        snippet_fuse(),
        snippet_saturate(),
        snippet_interleave(),
        snippet_defunctionalize(),
    ])
    .collect();

    // Union of everything that fires, across all three trace paths.
    let mut fired: BTreeSet<&'static str> = BTreeSet::new();
    for src in &programs {
        for kw in optimizations_fired(src) {
            fired.insert(kw);
        }
        for kw in optimizations_fired_run(src) {
            fired.insert(kw);
        }
        for kw in optimizations_fired_vm(src) {
            fired.insert(kw);
        }
    }
    // `supercompile` is a unified pass subsumed by the always-on individual passes;
    // its genuine niche is doing their work when they are off, so it fires once its
    // subsumer `comptime` is disabled.
    for kw in optimizations_fired(&decorate_source(&snippet_factorial(), &["comptime"])) {
        fired.insert(kw);
    }

    // Completeness: EVERY optimization in the registry is observed firing across
    // the trace paths — all 40, nothing forgotten, nothing merely documented. If a
    // new optimization is added it must come with a trigger that makes it fire here,
    // and if a change silently stops one firing, this fails.
    let registry: BTreeSet<&'static str> = REGISTRY.iter().map(|m| m.keyword).collect();
    let missing: Vec<&str> = registry.difference(&fired).copied().collect();
    assert!(
        missing.is_empty(),
        "these optimizations were never observed firing: {missing:?} — every one must have a trigger"
    );
    assert_eq!(fired.len(), REGISTRY.len(), "observed {:?}", fired);
}

/// Run-path tracing works end to end: the run-path-only optimizations surface
/// where the AOT trace cannot see them — gcd inlines its helper, mandelbrot's
/// loop-carried CSE fires, pi_leibniz's float strength reduction fires.
#[test]
fn run_path_only_optimizations_are_traced() {
    assert!(optimizations_fired_run(program("gcd")).contains(&"inline"), "gcd should fire run-path inline");
    assert!(
        optimizations_fired_run(program("mandelbrot")).contains(&"loopcse"),
        "mandelbrot should fire run-path loop-carried CSE"
    );
    assert!(
        optimizations_fired_run(program("pi_leibniz")).contains(&"floatstrength"),
        "pi_leibniz should fire run-path float strength reduction"
    );
}

/// The soundness check for the marks: every optimization that fires for a
/// program must STOP firing when it is disabled. A mark that ignores its gate
/// (the classic bug — an un-gated codegen emission) would keep reporting a
/// disabled opt as fired. Iterating only the opts that actually fire keeps this
/// fast while covering exactly the marks under test, across programs that
/// together exercise the codegen + AST mark surface.
// Sharded one program per `#[test]` so nextest runs the four in parallel (each
// program's marks are checked independently). Looked up BY NAME, not list index,
// so the shards are robust to reordering. `disabling_removes_covers_the_mark_surface`
// pins the four-program set so a shard can never silently drop to a no-op.
fn run_disabling_removes(name: &str) {
    let src = program(name);
    let fired = optimizations_fired(src);
    assert!(
        !fired.is_empty(),
        "{name}: expected to fire optimizations to exercise the mark gate, but fired nothing"
    );
    for kw in fired {
        let off = optimizations_fired(&decorate_source(src, &[kw]));
        assert!(
            !off.contains(&kw),
            "{name}: disabling `{kw}` did not remove it from the trace (its mark is not gated on its opt); still fired {off:?}"
        );
    }
}

#[test]
fn disabling_a_fired_optimization_removes_it_nqueens() { run_disabling_removes("nqueens"); }
#[test]
fn disabling_a_fired_optimization_removes_it_mergesort() { run_disabling_removes("mergesort"); }
#[test]
fn disabling_a_fired_optimization_removes_it_two_sum() { run_disabling_removes("two_sum"); }
#[test]
fn disabling_a_fired_optimization_removes_it_string_search() { run_disabling_removes("string_search"); }

/// Coverage guard: the four programs the shards cover are exactly the mark-surface
/// set the original single test iterated — all present in `PROGRAMS` and all
/// firing, so none of the four shards is silently a no-op.
#[test]
fn disabling_removes_covers_the_mark_surface() {
    for name in ["nqueens", "mergesort", "two_sum", "string_search"] {
        assert!(
            !optimizations_fired(program(name)).is_empty(),
            "{name} fires nothing — the disabling-removes shard for it would be a no-op"
        );
    }
}

/// Tracing is opt-in: the plain entry point compiles correctly and is not
/// polluted by a preceding traced compile (the thread-local is reset/taken).
#[test]
fn plain_path_is_unaffected_by_tracing() {
    let _ = compile_program_traced(nqueens()).unwrap();
    // A subsequent plain compile still succeeds and matches a fresh traced one.
    let plain = compile_to_rust(nqueens()).unwrap();
    let _ = compile_program_full(nqueens()).unwrap();
    let (out, _) = compile_program_traced(nqueens()).unwrap();
    assert_eq!(plain, out.rust_code);
}
