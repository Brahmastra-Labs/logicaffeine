//! Optimization firing-trace: the compiler records WHICH optimizations actually
//! fired for a given compile. These tests pin the contract:
//!   1. tracing never changes the generated code (soundness),
//!   2. a fired optimization was enabled (subset invariant),
//!   3. programs with known wins report exactly those optimizations,
//!   4. re-tracing under a different toggle state reflects the change,
//!   5. tracing is opt-in — the plain path records nothing.

use logicaffeine_compile::compile::{
    compile_program_backend_traced, compile_program_full, compile_program_traced, compile_to_c,
    compile_to_c_traced, compile_to_c_via_bytecode, compile_to_c_via_bytecode_traced,
    compile_to_rust, optimization_trace_run, optimization_trace_vm, optimizations_fired,
    optimizations_fired_run, optimizations_fired_vm,
};
use logicaffeine_compile::optimization::{
    decorate_source, Opt, OptimizationBackend, OptimizationConfig, REGISTRY,
};

/// A spread of benchmark programs, exercising the recursion, float, sort, array,
/// and hash-map clusters so the invariants are checked across optimization kinds.
/// Triggers `accumbound` (bounded-accumulator overflow-guard elision): isqrt's
/// Newton loop over a RUNTIME input (kept non-constant so it is not const-folded
/// away before the pass runs). The pass proves the loop stays i64-safe under a
/// compile-time input bound and versions it.
const ISQRT_ACCUM: &str = "## To native args () -> Seq of Text\n\
## To native parseInt (s: Text) -> Int\n\
## To isqrt (n: Int) -> Int:\n    \
    If n is less than 2:\n        \
        Return n.\n    \
    Let mutable x be n.\n    \
    Let mutable y be (x + 1) / 2.\n    \
    While y is less than x:\n        \
        Set x to y.\n        \
        Set y to (x + (n / x)) / 2.\n    \
    Return x.\n\
## Main\n\
Let arguments be args().\n\
Let n be parseInt(item 2 of arguments).\n\
Show isqrt(n).\n";

/// Shapes the benchmark programs no longer contain since they were made fair, kept so each optimization they
/// exercise keeps a trigger: a CSR offset array `start = item v of adjStarts` filled with `i * 5` (affine), a search
/// that only COUNTS matches in periodic text (simd, indexstring, cascade), and a map filled at the dense keys `1..=n`
/// (densemap).
const CSR_OFFSETS: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable adj be a new Seq of Int.
Let mutable adjStarts be a new Seq of Int.
Let mutable adjCounts be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push i * 5 to adjStarts.
    Push 0 to adjCounts.
    Push 0 to adj. Push 0 to adj. Push 0 to adj. Push 0 to adj. Push 0 to adj.
    Set i to i + 1.
Let mutable primes be a new Seq of Int.
Push 31 to primes. Push 37 to primes. Push 41 to primes. Push 43 to primes. Push 47 to primes.
Let mutable offsets be a new Seq of Int.
Push 7 to offsets. Push 13 to offsets. Push 17 to offsets. Push 23 to offsets. Push 29 to offsets.
Let mutable p be 1.
While p is at most 5:
    Set i to 0.
    While i is less than n:
        Let neighbor be (i * item p of primes + item p of offsets) % n.
        If neighbor is not i:
            Let start be item (i + 1) of adjStarts.
            Let cnt be item (i + 1) of adjCounts.
            Set item (start + cnt + 1) of adj to neighbor.
            Set item (i + 1) of adjCounts to cnt + 1.
        Set i to i + 1.
    Set p to p + 1.
Let mutable queue be a new Seq of Int.
Let mutable dist be a new Seq of Int.
Set i to 0.
While i is less than n:
    Push 0 - 1 to dist.
    Set i to i + 1.
Push 0 to queue.
Set item 1 of dist to 0.
Let mutable front be 1.
While front is at most length of queue:
    Let v be item front of queue.
    Let start be item (v + 1) of adjStarts.
    Let cnt be item (v + 1) of adjCounts.
    Let mutable e be 0.
    While e is less than cnt:
        Let u be item (start + e + 1) of adj.
        If item (u + 1) of dist equals 0 - 1:
            Set item (u + 1) of dist to item (v + 1) of dist + 1.
            Push u to queue.
        Set e to e + 1.
    Set front to front + 1.
Let mutable reachable be 0.
Let mutable totalDist be 0.
Set i to 0.
While i is less than n:
    If item (i + 1) of dist is at least 0:
        Set reachable to reachable + 1.
        Set totalDist to totalDist + item (i + 1) of dist.
    Set i to i + 1.
Show "" + reachable + " " + totalDist.
"#;
const COUNTING_SEARCH: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable text be "".
Let mutable pos be 0.
While pos is less than n:
    If pos is greater than 0:
        If pos % 1000 equals 0:
            If pos + 5 is at most n:
                Set text to text + "XXXXX".
                Set pos to pos + 5.
    If pos is less than n:
        Let mutable ch be "a".
        If pos % 5 equals 1:
            Set ch to "b".
        If pos % 5 equals 2:
            Set ch to "c".
        If pos % 5 equals 3:
            Set ch to "d".
        If pos % 5 equals 4:
            Set ch to "e".
        Set text to text + ch.
        Set pos to pos + 1.
Let needle be "XXXXX".
Let needleLen be 5.
Let textLen be length of text.
Let mutable count be 0.
Let mutable i be 1.
While i is at most textLen - needleLen + 1:
    Let mutable match be 1.
    Let mutable j be 0.
    While j is less than needleLen:
        If item (i + j) of text is not item (j + 1) of needle:
            Set match to 0.
            Set j to needleLen.
        Set j to j + 1.
    If match equals 1:
        Set count to count + 1.
    Set i to i + 1.
Show count.
"#;
const DENSE_KEYS_MAP: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let m be a new Map of Int to Int with capacity n.
Let i be 1.
While i is less than n + 1:
    Set item i of m to i * 2.
    Set i to i + 1.
Let found be 0.
Set i to 1.
While i is less than n + 1:
    If item i of m equals i * 2:
        Set found to found + 1.
    Set i to i + 1.
Show found.
"#;

const PROGRAMS: &[(&str, &str)] = &[
    ("accumbound_isqrt", ISQRT_ACCUM),
    (
        "nqueens",
        include_str!("../../../benchmarks/programs/nqueens/main.lg"),
    ),
    (
        "fib",
        include_str!("../../../benchmarks/programs/fib/main.lg"),
    ),
    (
        "mandelbrot",
        include_str!("../../../benchmarks/programs/mandelbrot/main.lg"),
    ),
    (
        "nbody",
        include_str!("../../../benchmarks/programs/nbody/main.lg"),
    ),
    (
        "quicksort",
        include_str!("../../../benchmarks/programs/quicksort/main.lg"),
    ),
    (
        "two_sum",
        include_str!("../../../benchmarks/programs/two_sum/main.lg"),
    ),
    (
        "gcd",
        include_str!("../../../benchmarks/programs/gcd/main.lg"),
    ),
    (
        "collatz",
        include_str!("../../../benchmarks/programs/collatz/main.lg"),
    ),
    (
        "mergesort",
        include_str!("../../../benchmarks/programs/mergesort/main.lg"),
    ),
    (
        "string_search",
        include_str!("../../../benchmarks/programs/string_search/main.lg"),
    ),
    (
        "pi_leibniz",
        include_str!("../../../benchmarks/programs/pi_leibniz/main.lg"),
    ),
    ("csr_offsets", CSR_OFFSETS),
    ("counting_search", COUNTING_SEARCH),
    ("dense_keys_map", DENSE_KEYS_MAP),
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
        let plain =
            compile_to_rust(src).unwrap_or_else(|e| panic!("{name}: plain compile failed: {e:?}"));
        let (out, _fired) = compile_program_traced(src)
            .unwrap_or_else(|e| panic!("{name}: traced compile failed: {e:?}"));
        if plain != out.rust_code {
            // A mismatch must come from tracing, not from inherent codegen
            // non-determinism. If two plain compiles already differ, the program
            // is non-deterministic (a separate concern, owned by
            // phase_codegen_determinism) and this comparison can't isolate
            // tracing — so only fail when codegen is deterministic here.
            let plain2 = compile_to_rust(src).unwrap();
            assert_eq!(
                plain, plain2,
                "{name}: codegen is non-deterministic (separate from tracing)"
            );
            assert_eq!(
                plain, out.rust_code,
                "{name}: tracing changed the generated Rust"
            );
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
        assert!(
            fired.bits().difference(enabled.bits()).is_empty(),
            "{name}: fired {:?} includes a disabled optimization",
            fired.keywords()
        );
    }
}

/// An optimized program fires *something* — the trace is not silently empty.
#[test]
fn optimized_program_fires_at_least_one() {
    let (_out, fired) = compile_program_traced(nqueens()).unwrap();
    assert!(
        !fired.is_empty(),
        "nqueens fired nothing — instrumentation is not recording"
    );
}

/// The old fired bitset remains the exact compatibility union. Existing UI,
/// decorators, and Rust benchmark metadata cannot lose an optimization merely
/// because the trace now identifies the stage that fired it.
#[test]
fn staged_rust_trace_preserves_the_legacy_union() {
    let src = program("mergesort");
    let (_, legacy) = compile_program_traced(src).unwrap();
    let (_, staged) = compile_program_backend_traced(src).unwrap();
    assert_eq!(legacy, staged.all_fired());
}

/// Every benchmark-page backend exposes the same two-stage contract: shared
/// optimizer transformations are kept separate from backend lowering choices.
/// The trace is observational only: Rust, direct C, and bytecode C stay
/// byte-identical to their ordinary compile entry points.
#[test]
fn every_benchmark_backend_has_a_separated_trace() {
    let src = nqueens();

    let rust_plain = compile_to_rust(src).unwrap();
    let (rust_out, rust) = compile_program_backend_traced(src).unwrap();
    assert_eq!(
        rust_plain, rust_out.rust_code,
        "Rust tracing changed generated code"
    );
    assert_eq!(rust.backend, OptimizationBackend::RustAot);
    assert!(
        !rust.optimizer.is_empty(),
        "Rust optimizer stage was not traced"
    );
    assert!(
        !rust.lowering.is_empty(),
        "Rust lowering stage was not traced"
    );

    let c_plain = compile_to_c(src).unwrap();
    let (c_out, c) = compile_to_c_traced(src).unwrap();
    assert_eq!(c_plain, c_out, "direct-C tracing changed generated code");
    assert_eq!(c.backend, OptimizationBackend::CAst);
    assert!(
        !c.optimizer.is_empty(),
        "direct C silently skipped the optimizer trace"
    );

    let bc_plain = compile_to_c_via_bytecode(src).unwrap();
    let (bc_out, bc) = compile_to_c_via_bytecode_traced(src).unwrap();
    assert_eq!(
        bc_plain, bc_out,
        "bytecode-C tracing changed generated code"
    );
    assert_eq!(bc.backend, OptimizationBackend::CBytecode);
    assert!(
        !bc.optimizer.is_empty(),
        "bytecode C silently skipped the optimizer trace"
    );

    let run = optimization_trace_run(src);
    assert_eq!(run.backend, OptimizationBackend::TreeWalker);
    assert!(
        !run.optimizer.is_empty(),
        "tree walker silently skipped its optimizer trace"
    );

    let vm = optimization_trace_vm(src);
    assert_eq!(vm.backend, OptimizationBackend::Vm);
    assert!(
        !vm.optimizer.is_empty(),
        "VM silently skipped its optimizer trace"
    );
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
        // Under value semantics the recursive partition threads a copy-on-write
        // `LogosSeq` and pair-TCEs, so its borrow-family win is borrow HOISTING
        // (`.borrow()` lifted out of the partition loop) rather than by-borrow
        // parameter inference.
        ("quicksort", "hoistborrows"),
        ("dense_keys_map", "densemap"),
        ("counting_search", "simd"),
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
    assert!(
        on.contains(&"symmetry"),
        "symmetry should fire by default; got {on:?}"
    );

    let off_src = decorate_source(nqueens(), &["symmetry"]);
    let off = optimizations_fired(&off_src);
    assert!(
        !off.contains(&"symmetry"),
        "symmetry must not fire when disabled; got {off:?}"
    );

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
/// DivModFuse trigger: a hot accumulator loop computing BOTH `i / 7` and `i % 7` in one
/// statement — the shared quotient is bound once and the modulo derived as `i - q*7`.
fn snippet_divmodfuse() -> String {
    format!("{HDR}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\nLet mutable sum be 0.\nLet mutable i be 1.\nWhile i is at most n:\n    Set sum to sum + (i / 7) + (i % 7).\n    Set i to i + 1.\nShow sum.\n")
}
/// SpecializeLoop trigger: a counted-accumulator loop calling a pure helper that branches
/// on its argument's sign. Under the counter's proven `>= 0` range the sign branch is dead,
/// so the call is symbolically inlined + folded to bare `i / 3`.
fn snippet_specializeloop() -> String {
    format!("{HDR}## To sdiv (x: Int) -> Int:\n    If x is less than 0:\n        Return 0 - ((0 - x) / 3).\n    Return x / 3.\n\n## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\nLet mutable sum be 0.\nLet mutable i be 1.\nWhile i is at most n:\n    Set sum to sum + sdiv(i).\n    Set i to i + 1.\nShow sum.\n")
}

#[test]
fn snippet_triggered_optimizations_fire_and_gate() {
    for (kw, src) in [
        ("cse", snippet_cse()),
        ("oracle", snippet_oracle()),
        ("narrowmap", snippet_narrowmap()),
        ("divmodfuse", snippet_divmodfuse()),
        ("specializeloop", snippet_specializeloop()),
    ] {
        let fired = optimizations_fired(&src);
        assert!(
            fired.contains(&kw),
            "snippet for `{kw}` must fire it (AOT); got {fired:?}"
        );
        let off = optimizations_fired(&decorate_source(&src, &[kw]));
        assert!(
            !off.contains(&kw),
            "disabling `{kw}` must remove it; got {off:?}"
        );
    }
}

/// The VM-bytecode-compile-time optimizations (constant-divisor magic division,
/// VM `i32` narrowing) fire and gate via the VM trace — the third path, which the
/// AOT and run-path traces cannot see.
#[test]
fn vm_compile_optimizations_fire_and_gate() {
    let fastdiv = format!("{HDR}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\nLet mutable acc be 0.\nLet mutable i be 1.\nWhile i is at most n:\n    Set acc to acc + i / 7.\n    Set i to i + 1.\nShow acc.\n");
    let fd = optimizations_fired_vm(&fastdiv);
    assert!(
        fd.contains(&"fastdiv"),
        "VM trace should fire fastdiv; got {fd:?}"
    );
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
    assert!(
        nv.contains(&"narrowvm"),
        "VM trace should fire narrowvm; got {nv:?}"
    );
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
/// SCCP trigger: `threshold` is a `Set` target (so `propagate` refuses it) whose
/// value is nonetheless provably 10 at the guard, letting SCCP take the then arm
/// — a prune neither `fold`/`propagate` nor `deadcode` can perform.
fn snippet_sccp() -> String {
    "## Main\nLet mutable threshold be 0.\nSet threshold to 10.\nIf threshold is greater than 5:\n    Show 100.\nOtherwise:\n    Show 200.\n".to_string()
}
/// RegPromote trigger: a reduction into `acc[1]` of a runtime-size array — an
/// invariant-indexed read-modify-write forwarded through a scalar, which no
/// benchmark exercises.
fn snippet_regpromote() -> String {
    "## Main\nLet n be 50.\nLet mutable acc be a new Seq of Int.\nLet mutable p be 0.\nWhile p is less than n:\n    Push 0 to acc.\n    Set p to p + 1.\nLet mutable i be 0.\nWhile i is less than n:\n    Set item 1 of acc to item 1 of acc + i.\n    Set i to i + 1.\nShow item 1 of acc.\n".to_string()
}
/// Interchange trigger: a matmul in SUBOPTIMAL `<i, j, k>` order (k innermost).
/// No benchmark exercises it (the matmul benchmark is already `<i, k, j>`), so
/// the pass needs this synthetic ijk nest to fire.
fn snippet_interchange() -> String {
    "## Main\nLet n be 8.\nLet a be a new Seq of Int.\nLet b be a new Seq of Int.\nLet c be a new Seq of Int.\nLet i be 0.\nWhile i is less than n:\n    Let j be 0.\n    While j is less than n:\n        Push (i * n + j) % 100 to a.\n        Push (j * n + i) % 100 to b.\n        Push 0 to c.\n        Set j to j + 1.\n    Set i to i + 1.\nSet i to 0.\nWhile i is less than n:\n    Let j be 0.\n    While j is less than n:\n        Let k be 0.\n        While k is less than n:\n            Set item (i * n + j + 1) of c to (item (i * n + j + 1) of c + item (i * n + k + 1) of a * item (k * n + j + 1) of b) % 1000000007.\n            Set k to k + 1.\n        Set j to j + 1.\n    Set i to i + 1.\nShow item 1 of c.\n".to_string()
}

/// BignumInPlace trigger: an iterative factorial whose accumulator overflows i64 and is
/// multiplied by the loop counter — a promoted scalar accumulator mutated in place.
fn snippet_bignuminplace() -> String {
    format!("{HDR}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\nLet mutable p be 1.\nLet mutable i be 1.\nWhile i is at most n:\n    Set p to p * i.\n    Set i to i + 1.\nShow p.\n")
}
/// BignumByRef trigger: exponentiation by squaring reads the promoted operand `b` in `acc * b`
/// and again reassigns it with `b * b`, so the by-ref lowering borrows it instead of cloning.
fn snippet_bignumbyref() -> String {
    format!("{HDR}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\nLet mutable acc be 1.\nLet mutable b be 3.\nLet mutable e be n.\nWhile e is greater than 0:\n    Set acc to acc * b.\n    Set b to b * b.\n    Set e to e - 1.\nShow acc.\n")
}
/// BitIntrinsics trigger: a native-width rotate and bit-reverse of a runtime value (from argv,
/// so it is not constant-folded away) lower to the CPU intrinsics.
fn snippet_bitintrinsics() -> String {
    format!("{HDR}## Main\nLet arguments be args().\nLet n be parseInt(item 2 of arguments).\nLet mutable sum be 0.\nLet mutable i be 1.\nWhile i is at most n:\n    Set sum to sum + rotateLeft(i, 7, 32) + reverseBits(i, 32).\n    Set i to i + 1.\nShow sum.\n")
}
/// TextBytesView trigger: `text_bytes(s)` on a Text PARAMETER (not const-inlinable) used ONLY for
/// reads (a byte-count loop) lowers to a zero-copy `s.as_bytes()` view — `s` is dead after the
/// binding, so borrowing it is lifetime-safe. Mirrors the stdlib text-fn shape.
fn snippet_textbytesview() -> String {
    "## To countLs (s: Text) -> Int:\n    Let sb be text_bytes(s).\n    Let mutable c be 0.\n    Repeat for b in sb:\n        If b is equal to 108:\n            Set c to c + 1.\n    Return c.\n\n## Main\nShow countLs(\"hello world\").\n".to_string()
}

/// StringAccumReuse trigger: a foreach accumulator that appends a VARYING chunk (`out += p`, p from
/// the iterator) — `out` is a mutable owned String, so each concat appends in place via `push_str`
/// instead of reallocating the whole accumulator. This is accumreuse's true domain: the invariant
/// counted-loop shape (`out += s`) is claimed by the strictly-better StringRepeatLoop (which declares
/// `preempts: StringAccumReuse`), so the trigger uses a foreach + per-item chunk it cannot lower to
/// `.repeat(n)`. See snippet_stringrepeatloop for the preempting shape.
fn snippet_stringaccumreuse() -> String {
    "## To joinAll (parts: Seq of Text) -> Text:\n    Let mutable out be \"\".\n    Repeat for p in parts:\n        Set out to out combined with p.\n    Return out.\n\n## Main\nLet parts be a new Seq of Text.\nPush \"a\" to parts.\nPush \"bb\" to parts.\nPush \"ccc\" to parts.\nShow joinAll(parts).\n".to_string()
}

/// StringRepeatLoop trigger: a counted loop appending a LOOP-INVARIANT chunk (`out += c`, c a param)
/// — the specialization of the accumulator that lowers to `c.repeat(n)` (one presized alloc). This is
/// the shape StringRepeatLoop `preempts: StringAccumReuse` on.
fn snippet_stringrepeatloop() -> String {
    "## To rep (c: Text, n: Int) -> Text:\n    Let mutable out be \"\".\n    Let mutable i be 1.\n    While i is at most n:\n        Set out to out combined with c.\n        Set i to i + 1.\n    Return out.\n\n## Main\nShow rep(\"ab\", 3).\n".to_string()
}

/// StringConcatReuse trigger: `Let s be rep(r) + base` where the leftmost leaf is a call returning an
/// owned String — its buffer is moved and `push_str`ed instead of a fresh `format!` alloc. The call
/// arg is the RUNTIME counter `r` (not a constant) so comptime can't fold `rep(..)` to a literal,
/// which would make the leftmost leaf a literal and defeat the pass.
fn snippet_stringconcatreuse() -> String {
    "## To rep (c: Text, n: Int) -> Text:\n    Let mutable out be \"\".\n    Let mutable i be 0.\n    While i is less than n:\n        Set out to out combined with c.\n        Set i to i + 1.\n    Return out.\n\n## Main\nLet base be \"hello world\".\nLet mutable acc be 0.\nLet mutable r be 0.\nWhile r is less than 5:\n    Let s be rep(\"a\", r) + base.\n    Set acc to acc + length of s.\n    Set r to r + 1.\nShow acc.\n".to_string()
}

/// TextFromBytesBuilder trigger: a `text_bytes` → transform → `Push` → `text_from_bytes(out)` write
/// loop lowers onto a `Vec<u8>` builder (no `Vec<i64>` detour, no UTF-8 revalidation).
fn snippet_textfrombytesbuilder() -> String {
    "## To upperOne (s: Text) -> Text:\n    Let sb be text_bytes(s).\n    Let out be a new Seq of Int.\n    Repeat for bt in sb:\n        Let mutable cv be bt.\n        If bt is at least 97:\n            If bt is at most 122:\n                Set cv to bt - 32.\n        Push cv to out.\n    Return text_from_bytes(out).\n\n## Main\nShow upperOne(\"hello\").\n".to_string()
}

/// TextSliceReturn trigger: a fn that scans then rebuilds a CONTIGUOUS sub-range and returns
/// `text_from_bytes(out)` lowers to a zero-copy `&str` slice of its source param.
fn snippet_textslicereturn() -> String {
    "## To trimStartOne (s: Text) -> Text:\n    Let sb be text_bytes(s).\n    Let n be length of sb.\n    Let mutable lo be 1.\n    Let mutable go be true.\n    While go is equal to true:\n        If lo is greater than n:\n            Set go to false.\n        Otherwise:\n            If (item lo of sb) is equal to 32:\n                Set lo to lo + 1.\n            Otherwise:\n                Set go to false.\n    Let out be a new Seq of Int.\n    Let mutable i be lo.\n    While i is at most n:\n        Push item i of sb to out.\n        Set i to i + 1.\n    Return text_from_bytes(out).\n\n## Main\nShow trimStartOne(\"  hi\").\n".to_string()
}

/// AsciiClassMask trigger: a runtime loop classifying bytes with `charIsDigit` — a pure Int->Bool
/// predicate whose true-set is in [0,256), so its body lowers to a branchless 256-bit mask test.
fn snippet_asciiclassmask() -> String {
    "## Main\nLet mutable s be 0.\nLet mutable i be 0.\nWhile i is less than 3000:\n    If charIsDigit(i % 300):\n        Set s to s + 1.\n    Set i to i + 1.\nShow s.\n".to_string()
}

/// StrengthReducePow2 trigger: a non-negative loop counter modulo/divided by a power of two lowers to
/// a mask/shift (`i % 128` → `i & 127`).
fn snippet_strengthreducepow2() -> String {
    "## Main\nLet mutable acc be 0.\nLet mutable i be 0.\nWhile i is less than 100000:\n    Let c be i % 128.\n    Set acc to acc + c.\n    Set i to i + 1.\nShow acc.\n".to_string()
}

/// MaskNarrow trigger: the masked LCG the benchmarks draw their inputs from (`seed` stays below 2^31, so every
/// node of `seed * 1103515245 + 12345` is proven within i64) computes in 32-bit wrapping arithmetic.
fn snippet_masknarrow() -> String {
    "## Main\nLet mutable seed be 42.\nLet mutable total be 0.\nLet mutable i be 0.\nWhile i is less than 100000:\n    Set seed to (seed * 1103515245 + 12345) % 2147483648.\n    Set total to (total + seed) % 1000000007.\n    Set i to i + 1.\nShow total.\n".to_string()
}

/// ModWrap trigger: a residue carried around a loop (`b = (a + b) % M` with `a` and `b` below `M`) reduces with one
/// conditional subtraction.
fn snippet_modwrap() -> String {
    "## Main\nLet mutable a be 0.\nLet mutable b be 1.\nLet mutable i be 0.\nWhile i is less than 100000:\n    Let t be b.\n    Set b to (a + b) % 1000000007.\n    Set a to t.\n    Set i to i + 1.\nShow a.\n".to_string()
}

/// BranchlessSelect trigger: collatz's parity step, whose odd arm keeps `3 * k + 1` exact behind a guarded dual,
/// selects between both arms.
fn snippet_branchlessselect() -> String {
    "## Main\nLet mutable total be 0.\nLet mutable i be 1.\nWhile i is at most 3000:\n    Let mutable k be i.\n    While k is not 1:\n        If k % 2 equals 0:\n            Set k to k / 2.\n        Otherwise:\n            Set k to 3 * k + 1.\n        Set total to total + 1.\n    Set i to i + 1.\nShow total.\n".to_string()
}

/// VariantBox trigger: a recursive enum whose `Node` holds two `Tree`s boxes the variant whole.
fn snippet_variantbox() -> String {
    "## A Tree is either:\n    A Leaf.\n    A Node with left Tree and right Tree.\n\n## To count (t: Tree) -> Int:\n    Inspect t:\n        When Leaf: Return 1.\n        When Node (l, r): Return count(l) + count(r).\n\n## Main\nShow count(a new Node with left (a new Leaf) and right (a new Leaf)).\n".to_string()
}

/// BitPack trigger: a local table of flags, written and read by index, is held one bit per item.
fn snippet_bitpack() -> String {
    "## Main\nLet mutable flags be a new Seq of Bool.\nLet mutable i be 0.\nWhile i is less than 100:\n    Push false to flags.\n    Set i to i + 1.\nSet i to 2.\nWhile i is at most 100:\n    Set item i of flags to true.\n    Set i to i + 3.\nLet mutable count be 0.\nSet i to 1.\nWhile i is at most 100:\n    If item i of flags equals true:\n        Set count to count + 1.\n    Set i to i + 1.\nShow count.\n".to_string()
}

/// SortNest trigger: a bubble sort's nest of adjacent compare-exchanges sorts its window by one heap sort.
fn snippet_sortnest() -> String {
    "## Main\nLet n be 50.\nLet arr be a new Seq of Int.\nLet mutable k be 0.\nWhile k is less than n:\n    Push (k * 37) % 50 to arr.\n    Set k to k + 1.\nLet mutable i be 0.\nWhile i is less than n - 1:\n    Let mutable j be 1.\n    While j is at most n - 1 - i:\n        Let a be item j of arr.\n        Let b be item (j + 1) of arr.\n        If a is greater than b:\n            Set item j of arr to b.\n            Set item (j + 1) of arr to a.\n        Set j to j + 1.\n    Set i to i + 1.\nShow item 1 of arr.\nShow item n of arr.\n".to_string()
}

/// BranchlessAccumulate trigger: a predicate-guarded `+1` on a plain accumulator lowers to a
/// branchless `sum = sum + (cond as i64)`.
fn snippet_branchlessaccumulate() -> String {
    "## Main\nLet mutable sum be 0.\nLet mutable i be 0.\nWhile i is less than 100000:\n    If i is less than 50000:\n        Set sum to sum + 1.\n    Set i to i + 1.\nShow sum.\n".to_string()
}

/// WideAccumulator trigger: a `Let 0` counting accumulator (`+1` under a guard, single counted loop,
/// read only by `Show`) widens to i128 for the loop.
fn snippet_wideaccumulator() -> String {
    "## Main\nLet mutable sum be 0.\nLet mutable i be 0.\nWhile i is less than 100000:\n    If (i % 7) is equal to 0:\n        Set sum to sum + 1.\n    Set i to i + 1.\nShow sum.\n".to_string()
}

/// RadixDigitBuffer trigger: the base-N itoa idiom (generate digits into a Seq, reverse into a
/// second Seq, `text_from_bytes`) lowers to a stack `[u8; 64]` written back-to-front.
fn snippet_radixdigitbuffer() -> String {
    "## To toBase (n: Int) and (base: Int) -> Text:\n    If n is equal to 0:\n        Return \"0\".\n    Let mutable m be n.\n    Let digits be a new Seq of Int.\n    While m is greater than 0:\n        Let d be m % base.\n        Let mutable code be 48 + d.\n        If d is at least 10:\n            Set code to 97 + (d - 10).\n        Push code to digits.\n        Set m to m / base.\n    Let out be a new Seq of Int.\n    Let k be length of digits.\n    Let mutable i be k.\n    While i is at least 1:\n        Push item i of digits to out.\n        Set i to i - 1.\n    Return text_from_bytes(out).\n\n## Main\nLet mutable n be 0.\nWhile n is at most 50:\n    Show toBase(n, 16).\n    Set n to n + 1.\n".to_string()
}

/// ByteSliceReturn trigger: a fn returning a contiguous sub-slice of a `Seq of Int` param (build
/// `out` then `Return out`) lowers to a borrowed `&'a [i64]`; the inline `length of` caller keeps it.
fn snippet_byteslicereturn() -> String {
    "## To trimByte (xs: Seq of Int) and (c: Int) -> Seq of Int:\n    Let n be length of xs.\n    Let mutable lo be 1.\n    Let mutable hi be n.\n    Let out be a new Seq of Int.\n    Let mutable i be lo.\n    While i is at most hi:\n        Push item i of xs to out.\n        Set i to i + 1.\n    Return out.\n\n## Main\nLet mutable xs be a new Seq of Int.\nPush 1 to xs.\nPush 2 to xs.\nShow length of trimByte(xs, 0).\n".to_string()
}

/// SimdReduce trigger: an integer accumulator summing `trunc(sin(toReal(i)/…)·…)` over a counted
/// loop — `sin` (via `sinCore`/`cosCore`/`roundR`/`floorR`) is SIMD-liftable, so the loop lowers to
/// an AVX2 four-lane reduction. The math.<transcendental> benchmark shape.
fn snippet_simdreduce() -> String {
    "## Main\nLet mutable sum be 0.\nLet mutable i be 1.\nWhile i is at most 1000:\n    Set sum to sum + trunc(sin(toReal(i) / 1000.0) * 1000000.0).\n    Set i to i + 1.\nShow sum.\n".to_string()
}

/// SimdReduce also lifts `atan`: the sqrt-free Cephes core (`atanPos`/`ratAtan`) is a fixed
/// straight-line rational, so its accumulator loop vectorizes the same way `sin` does.
fn snippet_simdreduce_atan() -> String {
    "## Main\nLet mutable sum be 0.\nLet mutable i be 1.\nWhile i is at most 1000:\n    Set sum to sum + trunc(atan(toReal(i) / 1000.0) * 1000000.0).\n    Set i to i + 1.\nShow sum.\n".to_string()
}

/// SimdReduce lifts `asin`/`acos` too: `asin = atan(x / sqrt(1 - x²))` needs the lifter's `sqrt`
/// intrinsic (`_mm256_sqrt_pd`), and `acos = π/2 - asin`; both reduce to `atan` + arithmetic + sqrt.
fn snippet_simdreduce_asin() -> String {
    "## Main\nLet mutable sum be 0.\nLet mutable i be 1.\nWhile i is at most 1000:\n    Set sum to sum + trunc(asin(toReal(i mod 1000) / 1000.0) * 1000000.0).\n    Set i to i + 1.\nShow sum.\n".to_string()
}

fn snippet_simdreduce_acos() -> String {
    "## Main\nLet mutable sum be 0.\nLet mutable i be 1.\nWhile i is at most 1000:\n    Set sum to sum + trunc(acos(toReal(i mod 1000) / 1000.0) * 1000000.0).\n    Set i to i + 1.\nShow sum.\n".to_string()
}

/// SimdReduce lifts `atan2` once flattened to a guarded-return chain: the `x<0` quadrant fold
/// uses `copysign` (a lifter intrinsic), and the whole thing rides the sqrt-free `atan`.
fn snippet_simdreduce_atan2() -> String {
    "## Main\nLet mutable sum be 0.\nLet mutable i be 1.\nWhile i is at most 1000:\n    Set sum to sum + trunc(atan2(toReal(i) / 1000.0, 100.0) * 1000000.0).\n    Set i to i + 1.\nShow sum.\n".to_string()
}

/// SimdReduce lifts `dist`/`hypot` via the `sqrt` intrinsic (`sqrt(dx²+dy²)`).
fn snippet_simdreduce_dist() -> String {
    "## Main\nLet mutable sum be 0.\nLet mutable i be 1.\nWhile i is at most 1000:\n    Set sum to sum + trunc(hypot(toReal(i) / 1000.0, toReal(i) / 700.0) * 1000000.0).\n    Set i to i + 1.\nShow sum.\n".to_string()
}

/// The bignum/bit codegen optimizations fire on their trigger programs and gate off when disabled.
#[test]
fn bignum_and_bit_optimizations_fire_and_gate() {
    for (kw, src) in [
        ("bignuminplace", snippet_bignuminplace()),
        ("bignumbyref", snippet_bignumbyref()),
        ("bitintrinsics", snippet_bitintrinsics()),
    ] {
        let fired = optimizations_fired(&src);
        assert!(
            fired.contains(&kw),
            "`{kw}` must fire on its trigger (AOT); got {fired:?}"
        );
        assert!(
            !optimizations_fired(&decorate_source(&src, &[kw])).contains(&kw),
            "disabling `{kw}` must remove it; got fired set for the decorated source"
        );
    }
}

/// TextBytesView fires on a read-only `text_bytes(s)` binding and gates off when disabled.
#[test]
fn text_bytes_view_fires_and_gates() {
    let src = snippet_textbytesview();
    let fired = optimizations_fired(&src);
    assert!(
        fired.contains(&"textbytesview"),
        "`textbytesview` must fire on a read-only text_bytes binding; got {fired:?}"
    );
    assert!(
        !optimizations_fired(&decorate_source(&src, &["textbytesview"])).contains(&"textbytesview"),
        "disabling `textbytesview` must remove it"
    );
}

/// StringAccumReuse fires on a `Set X to X combined with Y` accumulator loop and gates off.
#[test]
fn string_accum_reuse_fires_and_gates() {
    let src = snippet_stringaccumreuse();
    let fired = optimizations_fired(&src);
    assert!(
        fired.contains(&"stringaccumreuse"),
        "`stringaccumreuse` must fire on a self-append concat accumulator; got {fired:?}"
    );
    assert!(
        !optimizations_fired(&decorate_source(&src, &["stringaccumreuse"]))
            .contains(&"stringaccumreuse"),
        "disabling `stringaccumreuse` must remove it"
    );
}

/// AsciiClassMask fires on a `charIsDigit` classification loop and gates off when disabled.
#[test]
fn ascii_class_mask_fires_and_gates() {
    let src = snippet_asciiclassmask();
    let fired = optimizations_fired(&src);
    assert!(
        fired.contains(&"asciiclassmask"),
        "`asciiclassmask` must fire on a charIs* classification loop; got {fired:?}"
    );
    assert!(
        !optimizations_fired(&decorate_source(&src, &["asciiclassmask"]))
            .contains(&"asciiclassmask"),
        "disabling `asciiclassmask` must remove it"
    );
}

/// StrengthReducePow2 fires on a `counter % 2^k` and gates off when disabled.
#[test]
fn strength_reduce_pow2_fires_and_gates() {
    let src = snippet_strengthreducepow2();
    let fired = optimizations_fired(&src);
    assert!(
        fired.contains(&"strengthreducepow2"),
        "`strengthreducepow2` must fire on a non-negative `counter % 2^k`; got {fired:?}"
    );
    assert!(
        !optimizations_fired(&decorate_source(&src, &["strengthreducepow2"]))
            .contains(&"strengthreducepow2"),
        "disabling `strengthreducepow2` must remove it"
    );
}

/// BranchlessAccumulate fires on a predicate-guarded increment and gates off when disabled.
#[test]
fn branchless_accumulate_fires_and_gates() {
    let src = snippet_branchlessaccumulate();
    let fired = optimizations_fired(&src);
    assert!(
        fired.contains(&"branchlessaccumulate"),
        "`branchlessaccumulate` must fire on a predicate-guarded increment; got {fired:?}"
    );
    assert!(
        !optimizations_fired(&decorate_source(&src, &["branchlessaccumulate"]))
            .contains(&"branchlessaccumulate"),
        "disabling `branchlessaccumulate` must remove it"
    );
}

/// WideAccumulator fires on a bounded counting accumulator and gates off when disabled.
#[test]
fn wide_accumulator_fires_and_gates() {
    let src = snippet_wideaccumulator();
    let fired = optimizations_fired(&src);
    assert!(
        fired.contains(&"wideaccumulator"),
        "`wideaccumulator` must fire on a bounded counting accumulator; got {fired:?}"
    );
    assert!(
        !optimizations_fired(&decorate_source(&src, &["wideaccumulator"]))
            .contains(&"wideaccumulator"),
        "disabling `wideaccumulator` must remove it"
    );
}

/// ByteSliceReturn fires on the bytes-trim slice-return idiom and gates off when disabled.
#[test]
fn byte_slice_return_fires_and_gates() {
    let src = snippet_byteslicereturn();
    let fired = optimizations_fired(&src);
    assert!(
        fired.contains(&"byteslicereturn"),
        "`byteslicereturn` must fire on a Seq-of-Int contiguous slice return; got {fired:?}"
    );
    assert!(
        !optimizations_fired(&decorate_source(&src, &["byteslicereturn"]))
            .contains(&"byteslicereturn"),
        "disabling `byteslicereturn` must remove it"
    );
}

/// RadixDigitBuffer fires on the base-N itoa idiom and gates off when disabled.
#[test]
fn radix_digit_buffer_fires_and_gates() {
    let src = snippet_radixdigitbuffer();
    let fired = optimizations_fired(&src);
    assert!(
        fired.contains(&"radixdigitbuffer"),
        "`radixdigitbuffer` must fire on the base-N itoa idiom; got {fired:?}"
    );
    assert!(
        !optimizations_fired(&decorate_source(&src, &["radixdigitbuffer"]))
            .contains(&"radixdigitbuffer"),
        "disabling `radixdigitbuffer` must remove it"
    );
}

/// SimdReduce fires on a transcendental accumulator loop and gates off when disabled.
#[test]
fn simd_reduce_fires_and_gates() {
    let src = snippet_simdreduce();
    let fired = optimizations_fired(&src);
    assert!(
        fired.contains(&"simdreduce"),
        "`simdreduce` must fire on a trunc(sin(...)) accumulator loop; got {fired:?}"
    );
    assert!(
        !optimizations_fired(&decorate_source(&src, &["simdreduce"])).contains(&"simdreduce"),
        "disabling `simdreduce` must remove it"
    );
}

/// SimdReduce lifts the sqrt-free Cephes `atan` accumulator loop (via `atanPos`/`ratAtan`).
#[test]
fn simd_reduce_fires_on_atan() {
    let src = snippet_simdreduce_atan();
    let fired = optimizations_fired(&src);
    assert!(
        fired.contains(&"simdreduce"),
        "`simdreduce` must fire on a trunc(atan(...)) accumulator loop; got {fired:?}"
    );
    assert!(
        !optimizations_fired(&decorate_source(&src, &["simdreduce"])).contains(&"simdreduce"),
        "disabling `simdreduce` must remove it"
    );
}

/// SimdReduce lifts `asin`/`acos`/`atan2`/`dist` via the `sqrt`/`copysign` intrinsics + `atan` core.
#[test]
fn simd_reduce_fires_on_asin_acos() {
    for (name, src) in [
        ("asin", snippet_simdreduce_asin()),
        ("acos", snippet_simdreduce_acos()),
        ("atan2", snippet_simdreduce_atan2()),
        ("dist", snippet_simdreduce_dist()),
    ] {
        let fired = optimizations_fired(&src);
        assert!(
            fired.contains(&"simdreduce"),
            "`simdreduce` must fire on a trunc({name}(...)) accumulator loop; got {fired:?}"
        );
        assert!(
            !optimizations_fired(&decorate_source(&src, &["simdreduce"])).contains(&"simdreduce"),
            "disabling `simdreduce` must remove it for {name}"
        );
    }
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
        assert!(
            fired.contains(&kw),
            "`{kw}` must fire on its trigger; got {fired:?}"
        );
        assert!(
            !optimizations_fired(&decorate_source(&src, &[kw])).contains(&kw),
            "disabling `{kw}` must remove it; trigger still fired it"
        );
    }
    // supercompile: fires when its subsumer (comptime) is off.
    let sc = decorate_source(&snippet_factorial(), &["comptime"]);
    assert!(
        optimizations_fired(&sc).contains(&"supercompile"),
        "supercompile must fire with comptime off"
    );
    assert!(
        !optimizations_fired(&decorate_source(
            &snippet_factorial(),
            &["comptime", "supercompile"]
        ))
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
    let bench = |n: &str| {
        std::fs::read_to_string(format!(
            "{}/../../benchmarks/programs/{n}/main.lg",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    };
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
            let off: BTreeSet<&'static str> = optimizations_fired(&decorate_source(src, &[x]))
                .into_iter()
                .collect();
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
        "fib",
        "ackermann",
        "nqueens",
        "gcd",
        "collatz",
        "primes",
        "mandelbrot",
        "pi_leibniz",
        "nbody",
        "binary_trees",
        "loop_sum",
        "matrix_mult",
        "two_sum",
        "collect",
        "coins",
        "knapsack",
        "array_fill",
        "graph_bfs",
        "counting_sort",
        "string_search",
        "mergesort",
        "quicksort",
        "heap_sort",
    ]
    .iter()
    .map(|n| bench(n))
    .chain([CSR_OFFSETS, COUNTING_SEARCH, DENSE_KEYS_MAP].map(String::from))
    .chain([
        snippet_cse(),
        snippet_oracle(),
        snippet_narrowmap(),
        snippet_divmodfuse(),
        snippet_specializeloop(),
        snippet_fuse(),
        snippet_saturate(),
        snippet_interleave(),
        snippet_defunctionalize(),
        snippet_sccp(),
        snippet_regpromote(),
        snippet_interchange(),
        snippet_bignuminplace(),
        snippet_bignumbyref(),
        snippet_bitintrinsics(),
        snippet_textbytesview(),
        snippet_stringaccumreuse(),
        snippet_stringrepeatloop(),
        snippet_stringconcatreuse(),
        snippet_textfrombytesbuilder(),
        snippet_textslicereturn(),
        snippet_asciiclassmask(),
        snippet_strengthreducepow2(),
        snippet_masknarrow(),
        snippet_modwrap(),
        snippet_branchlessselect(),
        snippet_variantbox(),
        snippet_bitpack(),
        snippet_sortnest(),
        snippet_branchlessaccumulate(),
        snippet_wideaccumulator(),
        snippet_radixdigitbuffer(),
        snippet_byteslicereturn(),
        snippet_simdreduce(),
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
    // the trace paths — every registry entry, nothing forgotten, nothing merely documented. If a
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
    assert!(
        optimizations_fired_run(program("gcd")).contains(&"inline"),
        "gcd should fire run-path inline"
    );
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
fn disabling_a_fired_optimization_removes_it_nqueens() {
    run_disabling_removes("nqueens");
}
#[test]
fn disabling_a_fired_optimization_removes_it_mergesort() {
    run_disabling_removes("mergesort");
}
#[test]
fn disabling_a_fired_optimization_removes_it_two_sum() {
    run_disabling_removes("two_sum");
}
#[test]
fn disabling_a_fired_optimization_removes_it_string_search() {
    run_disabling_removes("string_search");
}

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

/// The Rust emitter has long recognized the benchmark's ping-pong dynamic-
/// programming buffers and emitted `resize` + `mem::swap`.  C must consume the
/// same peephole instead of allocating a fresh `curr` on every outer
/// iteration.  This is both a code-shape lock and a trace lock: an optimization
/// that is not reported by the concrete backend cannot be ratcheted.
#[test]
fn c_ast_reuses_and_traces_knapsack_double_buffers() {
    let source = include_str!("../../../benchmarks/programs/knapsack/main.lg");
    let (c, trace) = compile_to_c_traced(source).expect("compile knapsack through C AST");

    assert!(
        trace.lowering.fired(Opt::Peephole),
        "the C backend must report the buffer-reuse lowering: {trace:?}"
    );
    assert!(
        c.contains("curr.len = 0;"),
        "the loop-local buffer must be cleared and reused instead of reconstructed:\n{c}"
    );
    assert!(
        c.contains("Seq_i64 __buffer_swap = prev; prev = curr; curr = __buffer_swap;"),
        "the ownership transfer must become a capacity-preserving buffer swap:\n{c}"
    );
    // Item growth (`optimize::item_growth`) runs the rolled row's loop with raw additions under `n <= C` and as
    // written otherwise: each copy of the loop builds its buffer once, before its hot loop clears and refills it.
    let curr_ctor = "Seq_i64 curr = seq_i64_new();";
    let (fast, slow) = guarded_branches(&c, "<= 92233720368547758LL");
    for (branch, code) in [("raw", fast), ("as written", slow)] {
        assert_eq!(
            code.matches(curr_ctor).count(),
            1,
            "the reusable buffer must be constructed exactly once in the {branch} branch:\n{c}"
        );
        assert!(
            code.find(curr_ctor).expect("curr constructor")
                < code.find("curr.len = 0;").expect("curr clear in the hot loop"),
            "curr must be constructed before the hot loop clears and refills it in the {branch} branch"
        );
    }

    // The same registry gate controls every backend. Turning the peephole off
    // restores the ordinary C lowering and removes its lowering evidence.
    let disabled = decorate_source(source, &["peephole"]);
    let (plain, off_trace) =
        compile_to_c_traced(&disabled).expect("compile knapsack with C peepholes disabled");
    assert!(!off_trace.lowering.fired(Opt::Peephole));
    assert!(plain.contains("prev = curr;"));
    assert!(!plain.contains("curr.len = 0;"));
}

/// The two branches of the `if` whose condition contains `guard`: its block, and the block after its `else`, each
/// matched by brace depth.
fn guarded_branches<'s>(code: &'s str, guard: &str) -> (&'s str, &'s str) {
    fn block(code: &str, from: usize) -> (usize, usize) {
        let open = from + code[from..].find('{').expect("a block opens");
        let mut depth = 0usize;
        for (i, c) in code[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return (open, open + i + 1);
                    }
                }
                _ => {}
            }
        }
        panic!("the block opened at {open} closes")
    }
    let at = code
        .find(guard)
        .unwrap_or_else(|| panic!("the guard `{guard}` is emitted:\n{code}"));
    let (fast_open, fast_close) = block(code, at);
    assert!(
        code[fast_close..].trim_start().starts_with("else"),
        "the guarded block has an `else`:\n{code}"
    );
    let (slow_open, slow_close) = block(code, fast_close);
    (&code[fast_open..fast_close], &code[slow_open..slow_close])
}

/// Rust and C share one structural proof for the fixed-window text-compare
/// idiom.  C must emit direct byte loads, not allocate one-character strings
/// and call `strcmp` for every candidate byte.
#[test]
fn c_ast_consumes_shared_fixed_window_matcher() {
    let source = include_str!("../../../benchmarks/programs/string_search/main.lg");
    let (c, trace) = compile_to_c_traced(source).expect("compile string_search through C AST");

    assert!(
        trace.lowering.fired(Opt::Simd),
        "C must trace the shared window lowering: {trace:?}"
    );
    assert!(
        trace.lowering.fired(Opt::Borrow),
        "C must trace borrowed immutable literals: {trace:?}"
    );
    assert!(c.contains("/* shared fixed-window byte compare */"));
    assert!(c.contains("char * ch = \"a\";"));
    assert!(!c.contains("char * ch = strdup(\"a\");"));
    assert!(!c.contains("char * ch = lc_gc_strdup(\"a\");"));
    assert!(!c.contains("logos_text_builder_char_at(text, (i + j))"));

    let disabled = decorate_source(source, &["simd"]);
    let (plain, off_trace) =
        compile_to_c_traced(&disabled).expect("compile string_search with SIMD disabled");
    assert!(!off_trace.lowering.fired(Opt::Simd));
    assert!(!plain.contains("/* shared fixed-window byte compare */"));
    assert!(plain.contains(
        "logos_text_builder_char_at(text, lint_index_i64(logos_lint_add(i, lint_of((int64_t)(j))),"
    ));
    assert!(plain.contains("\"Cannot index Text with Int\""));

    let no_borrow = decorate_source(source, &["borrow"]);
    let (copying, no_borrow_trace) =
        compile_to_c_traced(&no_borrow).expect("compile string_search with Borrow disabled");
    assert!(!no_borrow_trace.lowering.fired(Opt::Borrow));
    assert!(copying.contains("char * ch = lc_gc_strdup(\"a\");"));
}

/// C consumes the same conservative fixed-buffer analyses that give Rust its
/// `[T; N]` SHA lowering.  Proven non-escaping buffers keep the ordinary Seq
/// API but point at block-local storage, so no hot-loop allocation is hidden
/// behind a backend-specific emitter decision.
#[test]
fn c_ast_consumes_shared_fixed_sequence_proofs() {
    let source = include_str!("../../../benchmarks/programs/sha256/main.lg");
    let (c, trace) = compile_to_c_traced(source).expect("compile sha256 through C AST");

    assert!(
        trace.lowering.fired(Opt::Scalarize),
        "C must report consuming the shared fixed-buffer proof: {trace:?}"
    );
    for shape in [
        "int64_t __logos_fixed_sched[64] = {0};",
        "int64_t __logos_fixed_m[16] = {0};",
        "int64_t __logos_fixed_h0[8] = {0};",
    ] {
        assert!(
            c.contains(shape),
            "missing stack-backed fixed sequence `{shape}`:\n{c}"
        );
    }
    // The schedule is in the compression function; both versions of the main loop
    // own separate message and state buffers.
    assert_eq!(c.matches("/* shared fixed Seq */").count(), 5);
    assert_eq!(
        c.matches("int64_t __logos_fixed_sched[64] = {0};").count(),
        1
    );
    assert_eq!(c.matches("int64_t __logos_fixed_m[16] = {0};").count(), 2);
    assert_eq!(c.matches("int64_t __logos_fixed_h0[8] = {0};").count(), 2);

    let disabled = decorate_source(source, &["scalarize"]);
    let (heap, off_trace) =
        compile_to_c_traced(&disabled).expect("compile sha256 with scalarization disabled");
    assert!(!off_trace.lowering.fired(Opt::Scalarize));
    assert!(!heap.contains("/* shared fixed Seq */"));
    assert!(heap.contains("Seq_i64 sched = seq_i64_new();"));
}

/// Bytecode must carry the same ping-pong proof instead of turning the final
/// transfer into an aliasing `Move`.  The register swap leaves the old `prev`
/// allocation solely owned by `curr`, so VM, C-bytecode, and JIT can all clear
/// and refill it on the next iteration.
#[test]
fn bytecode_reuses_and_traces_knapsack_double_buffers() {
    let source = include_str!("../../../benchmarks/programs/knapsack/main.lg");
    let (c, trace) =
        compile_to_c_via_bytecode_traced(source).expect("compile knapsack through bytecode C");

    assert!(
        trace.lowering.fired(Opt::Peephole),
        "the bytecode compiler must report its buffer-reuse lowering: {trace:?}"
    );
    assert!(
        c.contains("/* ping-pong buffer swap */"),
        "the ownership transfer must lower to a register-and-ownership swap"
    );
    assert!(
        c.contains("/* reuse sole-owned list buffer */"),
        "NewEmptyList must mirror the VM's clear-in-place fast path"
    );

    let disabled = decorate_source(source, &["peephole"]);
    let (plain, off_trace) = compile_to_c_via_bytecode_traced(&disabled)
        .expect("compile bytecode C with peepholes disabled");
    assert!(!off_trace.lowering.fired(Opt::Peephole));
    assert!(!plain.contains("/* ping-pong buffer swap */"));

    // Do not "optimize" an ownership transfer when the old inner name stays
    // live afterwards.  A swap there would make this push mutate the previous
    // row rather than the row just assigned to `prev`.
    let escaped = source.replacen(
        "    Set prev to curr.\n    Set i to i + 1.",
        "    Set prev to curr.\n    Push 0 to curr.\n    Set i to i + 1.",
        1,
    );
    assert_ne!(escaped, source, "the knapsack transfer shape changed");
    let (escaped_c, _) = compile_to_c_via_bytecode_traced(&escaped)
        .expect("compile a post-transfer-use counterexample");
    assert!(
        !escaped_c.contains("/* ping-pong buffer swap */"),
        "a post-transfer use must keep the aliasing move semantics"
    );
}

/// An inferred consumed-and-returned sequence borrow (heap sort's
/// `arr = siftDown(arr, ...)`) is not an escaping alias.  Once `EnsureOwned`
/// isolates the caller's value, C-bytecode must transfer that sole-ownership
/// proof through the argument window and back to the call result.  Dropping it
/// makes the next call clone the entire array and turns heap sort quadratic.
#[test]
fn c_bytecode_preserves_ownership_through_consumed_borrow() {
    let source = include_str!("../../../benchmarks/programs/heap_sort/main.lg");
    let (c, trace) =
        compile_to_c_via_bytecode_traced(source).expect("compile heap sort through bytecode C");

    assert!(
        trace.lowering.fired(Opt::Borrow),
        "the C-bytecode ownership transfer must be backend-visible: {trace:?}"
    );
    assert!(
        c.contains("/* preserve sole ownership through consumed borrow */"),
        "a consumed-and-returned borrow must transfer its ownership bit through the call"
    );

    let disabled = decorate_source(source, &["borrow"]);
    let (plain, off_trace) = compile_to_c_via_bytecode_traced(&disabled)
        .expect("compile heap sort with borrow inference disabled");
    assert!(!off_trace.lowering.fired(Opt::Borrow));
    assert!(!plain.contains("/* preserve sole ownership through consumed borrow */"));
}

/// Bytecode-C function calls carry a collection's sole-owner bit back with
/// the return value, and transient argument windows only borrow ordinary value
/// parameters.  Otherwise SHA loses both its returned-list ownership and the
/// reusable `m`/`h0` buffers at every call boundary.
#[test]
fn c_bytecode_preserves_ownership_across_readonly_calls_and_returns() {
    let source = include_str!("../../../benchmarks/programs/sha256/main.lg");
    let (c, trace) =
        compile_to_c_via_bytecode_traced(source).expect("compile sha256 through bytecode C");

    assert!(
        trace.lowering.fired(Opt::Borrow),
        "bytecode-C must trace call ownership: {trace:?}"
    );
    assert!(c.contains("unsigned char *__ret_own"));
    assert!(c.contains("if (__ret_own) *__ret_own = own[28];"));
    assert!(c.contains("fn_7(r[14], r[15], r[16], &own[8])"));
    assert_eq!(
        c.matches("/* retain ownership across read-only call argument */")
            .count(),
        4,
        "m and h0 must remain reusable across the compression call in both word and exact regions"
    );

    let disabled = decorate_source(source, &["borrow"]);
    let (plain, off_trace) =
        compile_to_c_via_bytecode_traced(&disabled).expect("compile sha256 with Borrow disabled");
    assert!(!off_trace.lowering.fired(Opt::Borrow));
    assert!(!plain.contains("/* retain ownership across read-only call argument */"));
    assert!(plain.contains("fn_7(r[14], r[15], r[16], NULL)"));
}

/// Bytecode C consumes the same fixed-sequence proof as Rust and syntax-tree C.
/// SHA's message schedule and its two call-site inputs have exact capacities and
/// do not escape their stack frame, so heap-boxing them is both unnecessary and
/// a large benchmark regression. Returned or dynamically-sized sequences must
/// retain the ordinary heap representation.
#[test]
fn c_bytecode_stack_backs_shared_fixed_sequences_without_escaping_them() {
    let source = include_str!("../../../benchmarks/programs/sha256/main.lg");
    let (c, trace) =
        compile_to_c_via_bytecode_traced(source).expect("compile sha256 through bytecode C");

    assert!(
        trace.lowering.fired(Opt::Scalarize),
        "fixed-sequence lowering must be backend-visible: {trace:?}"
    );
    assert_eq!(
        c.matches("/* shared fixed Seq */").count(),
        6,
        "the schedule, m, and h0 must use the shared fixed-buffer proof in both word and exact regions"
    );
    assert_eq!(c.matches("[64] = {0}; /* shared fixed Seq */").count(), 2);
    assert_eq!(c.matches("[16] = {0}; /* shared fixed Seq */").count(), 2);
    assert_eq!(c.matches("[8] = {0}; /* shared fixed Seq */").count(), 2);

    let disabled = decorate_source(source, &["scalarize"]);
    let (plain, off_trace) = compile_to_c_via_bytecode_traced(&disabled)
        .expect("compile sha256 with Scalarize disabled");
    assert!(!off_trace.lowering.fired(Opt::Scalarize));
    assert!(!plain.contains("/* shared fixed Seq */"));

    let escaping = r#"## To copy (xs: Seq of Int) -> Seq of Int:
    Let mutable out be a new Seq of Int.
    Repeat for x in xs:
        Push x to out.
    Return out.

## Main
Let mutable xs be a new Seq of Int.
Push 1 to xs.
Show copy(xs).
"#;
    let (escaping_c, _) = compile_to_c_via_bytecode_traced(escaping)
        .expect("compile an escaping dynamically-sized sequence");
    let (copy_fn, main_fn) = escaping_c
        .split_once("int main(int argc, char **argv) {")
        .expect("generated C contains main");
    assert!(
        !copy_fn.contains("/* shared fixed Seq */") && copy_fn.contains("seq_i64_new()"),
        "the returned, dynamically-sized sequence must remain heap-backed"
    );
    assert_eq!(
        main_fn.matches("/* shared fixed Seq */").count(),
        1,
        "the separate one-element input is fixed and non-escaping, so it must remain optimized"
    );
}

#[test]
fn shared_fixed_sequence_plan_is_scoped_per_function() {
    let source = r#"## To makeTwo (seed: Int) and (which: Int) -> Int:
    Let mutable work be a new Seq of Int.
    Push seed to work.
    Push seed + 1 to work.
    Return item which of work.

## To makeThree (seed: Int) and (which: Int) -> Int:
    Let mutable work be a new Seq of Int.
    Push seed to work.
    Push seed + 1 to work.
    Push seed + 2 to work.
    Return item which of work.

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Show makeTwo(n, 1).
Show makeThree(n, 2).
"#;

    for (backend, generated) in [
        (
            "C AST",
            compile_to_c_traced(source).expect("compile direct C").0,
        ),
        (
            "C bytecode",
            compile_to_c_via_bytecode_traced(source)
                .expect("compile bytecode C")
                .0,
        ),
    ] {
        let versions = if backend == "C bytecode" { 2 } else { 1 };
        assert_eq!(
            generated.matches("/* shared fixed Seq */").count(),
            2 * versions,
            "{backend} must lower both scope-local fixed sequences in every emitted version"
        );
        assert_eq!(
            generated.matches("[2] = {0};").count(),
            versions,
            "{backend} must retain makeTwo's scope-local capacity in every emitted version"
        );
        assert_eq!(
            generated.matches("[3] = {0};").count(),
            versions,
            "{backend} must retain makeThree's scope-local capacity in every emitted version"
        );
    }
}

/// Integer-heavy text construction must use the same allocation-free radix
/// append in both C tiers and report that backend choice. Keeping the off
/// shape beside it prevents an untraced fast path from silently surviving the
/// optimization decorator.
#[test]
fn c_backends_trace_allocation_free_integer_text_append() {
    let source = "## Main\n\
        Let mutable result be \"\" with capacity 60.\n\
        Let mutable i be 0.\n\
        While i is less than 10:\n\
        \x20   Set result to result + i + \" \".\n\
        \x20   Set i to i + 1.\n\
        Show result.\n";

    let (direct, direct_trace) =
        compile_to_c_traced(source).expect("compile integer text append through direct C");
    assert!(
        direct_trace.lowering.fired(Opt::StringAccumReuse),
        "direct C must report its allocation-free text append: {direct_trace:?}"
    );
    assert!(
        direct.matches("logos_text_append_i64(").count() >= 2,
        "direct C must call the radix append helper:\n{direct}"
    );
    assert!(
        direct.matches("logos_text_push_ascii(").count() >= 2,
        "direct C must call the single-byte separator helper:\n{direct}"
    );

    let (bytecode, bytecode_trace) = compile_to_c_via_bytecode_traced(source)
        .expect("compile integer text append through bytecode C");
    assert!(
        bytecode_trace.lowering.fired(Opt::StringAccumReuse),
        "bytecode C must report its allocation-free text append: {bytecode_trace:?}"
    );
    assert!(
        bytecode.matches("lc_text_append_i64(").count() >= 2,
        "bytecode C must call the radix append helper:\n{bytecode}"
    );

    let disabled = decorate_source(source, &["stringaccumreuse"]);
    let (direct_off, direct_off_trace) =
        compile_to_c_traced(&disabled).expect("compile direct C with string append reuse disabled");
    assert!(
        !direct_off_trace.lowering.fired(Opt::StringAccumReuse),
        "disabled direct C fast path must not be traced"
    );
    assert_eq!(
        direct_off.matches("logos_text_append_i64(").count(),
        1,
        "disabled direct C must retain only the helper definition"
    );
    assert_eq!(
        direct_off.matches("logos_text_push_ascii(").count(),
        1,
        "disabled direct C must retain only the helper definition"
    );

    let (bytecode_off, bytecode_off_trace) = compile_to_c_via_bytecode_traced(&disabled)
        .expect("compile bytecode C with string append reuse disabled");
    assert!(
        !bytecode_off_trace.lowering.fired(Opt::StringAccumReuse),
        "disabled bytecode C fast path must not be traced"
    );
    assert_eq!(
        bytecode_off.matches("lc_text_append_i64(").count(),
        1,
        "disabled bytecode C must retain only the helper definition"
    );
}

/// Both C tiers consume the shared TextBytesView proof for read-only byte
/// iteration. The disabled forms lock the materializing fallback beside the
/// fast shape so a backend cannot silently keep or lose the optimization.
#[test]
fn c_backends_trace_zero_copy_text_bytes_iteration() {
    let source = snippet_textbytesview();

    let (direct, direct_trace) =
        compile_to_c_traced(&source).expect("compile zero-copy byte iteration through direct C");
    assert!(
        direct_trace.lowering.fired(Opt::TextBytesView),
        "direct C must report its borrowed text byte view: {direct_trace:?}"
    );
    assert!(
        direct.contains("const unsigned char *sb = (const unsigned char *)(s);"),
        "direct C must bind text_bytes as a borrowed byte pointer:\n{direct}"
    );
    assert!(
        !direct.contains("Seq_i64 sb = logos_text_bytes(s)"),
        "direct C must not materialize the proved read-only byte sequence"
    );

    let (bytecode, bytecode_trace) = compile_to_c_via_bytecode_traced(&source)
        .expect("compile zero-copy byte iteration through bytecode C");
    assert!(
        bytecode_trace.lowering.fired(Opt::TextBytesView),
        "bytecode C must report its borrowed text byte view: {bytecode_trace:?}"
    );
    assert!(
        bytecode.contains("const unsigned char *_itv"),
        "bytecode C must iterate a borrowed unsigned byte pointer:\n{bytecode}"
    );
    assert!(
        !bytecode.contains("= logos_text_bytes((const char*)(intptr_t)"),
        "bytecode C must not materialize the proved read-only byte sequence"
    );

    let disabled = decorate_source(&source, &["textbytesview"]);
    let (direct_off, direct_off_trace) =
        compile_to_c_traced(&disabled).expect("compile direct C with byte views disabled");
    assert!(
        !direct_off_trace.lowering.fired(Opt::TextBytesView),
        "disabled direct C byte view must not be traced"
    );
    assert!(
        direct_off.contains("logos_text_bytes(s)"),
        "disabled direct C must retain byte-sequence materialization"
    );

    let (bytecode_off, bytecode_off_trace) = compile_to_c_via_bytecode_traced(&disabled)
        .expect("compile bytecode C with byte views disabled");

    assert!(
        !bytecode_off_trace.lowering.fired(Opt::TextBytesView),
        "disabled bytecode C byte view must not be traced"
    );
    assert!(
        bytecode_off.contains("= logos_text_bytes((const char*)(intptr_t)"),
        "disabled bytecode C must retain byte-sequence materialization"
    );
}

/// A byte sequence that is mutated cannot alias immutable Text storage. Both C
/// tiers must retain the materializing fallback and must not claim the view.
#[test]
fn c_backends_reject_text_bytes_view_for_mutation() {
    let source = "## To mutateBytes (s: Text) -> Seq of Int:\n    Let bytes be text_bytes(s).\n    Push 33 to bytes.\n    Return bytes.\n\n## Main\nShow length of mutateBytes(\"hi\").\n";

    let (direct, direct_trace) =
        compile_to_c_traced(source).expect("compile mutable text bytes through direct C");
    assert!(
        !direct_trace.lowering.fired(Opt::TextBytesView),
        "direct C must not borrow a byte sequence that is mutated"
    );
    assert!(
        direct.contains("logos_text_bytes(s)"),
        "direct C must materialize mutable text bytes"
    );

    let (bytecode, bytecode_trace) = compile_to_c_via_bytecode_traced(source)
        .expect("compile mutable text bytes through bytecode C");
    assert!(
        !bytecode_trace.lowering.fired(Opt::TextBytesView),
        "bytecode C must not borrow a byte sequence that is mutated"
    );
    assert!(
        bytecode.contains("= logos_text_bytes((const char*)(intptr_t)"),
        "bytecode C must materialize mutable text bytes"
    );
}

/// Direct C consumes the same bounded-count proof as Rust before removing the branch. The bytecode
/// tier deliberately retains its cheaper predictable branch and must explain that profitability
/// decision in the trace, so neither behavior can disappear silently.
#[test]
fn c_backends_trace_branchless_accumulation_or_refusal() {
    let source = snippet_branchlessaccumulate();

    let (direct, direct_trace) =
        compile_to_c_traced(&source).expect("compile branchless accumulation through direct C");
    assert!(
        direct_trace.lowering.fired(Opt::BranchlessAccumulate),
        "direct C must report its proof-gated branchless add: {direct_trace:?}"
    );
    assert!(
        direct.contains("+= (int64_t)!!("),
        "direct C must emit a plain branchless add after proving it cannot overflow:\n{direct}"
    );
    assert!(
        direct.contains("int64_t sum") && !direct.contains("LInt sum"),
        "the proved bounded counter must stay on direct C's native word path:\n{direct}"
    );

    let (bytecode, bytecode_trace) = compile_to_c_via_bytecode_traced(&source)
        .expect("compile branchless candidate through bytecode C");
    assert!(
        !bytecode_trace.lowering.fired(Opt::BranchlessAccumulate),
        "bytecode C must not claim a transform its cost model rejects"
    );
    assert!(
        bytecode_trace
            .declines
            .iter()
            .any(|d| d.opt == Opt::BranchlessAccumulate
                && d.reason.starts_with("branchlessaccumulate:")),
        "bytecode C must trace its profitability refusal: {bytecode_trace:?}"
    );
    assert!(
        bytecode_trace.fallbacks.is_empty(),
        "a same-backend profitability decision is not a backend fallback: {bytecode_trace:?}"
    );
    assert!(
        bytecode.contains("if (!r["),
        "bytecode C must retain the measured-faster guarded add:\n{bytecode}"
    );

    let disabled = decorate_source(&source, &["branchlessaccumulate"]);
    let (direct_off, direct_off_trace) =
        compile_to_c_traced(&disabled).expect("compile direct C with branchless add disabled");
    assert!(!direct_off_trace.lowering.fired(Opt::BranchlessAccumulate));
    assert!(!direct_off.contains("+= (int64_t)!!("));
    assert!(
        direct_off.contains("int64_t sum") && direct_off.contains("if (((i < 50000LL)))"),
        "disabling branchlessaccumulate must retain the guarded add; the independent range proof still bounds sum:\n{direct_off}"
    );

    let (_, bytecode_off_trace) = compile_to_c_via_bytecode_traced(&disabled)
        .expect("compile bytecode C with branchless add disabled");
    assert!(
        !bytecode_off_trace
            .declines
            .iter()
            .any(|d| d.opt == Opt::BranchlessAccumulate),
        "a disabled optimization must not report a profitability decision"
    );
}

/// An accumulator with an arbitrary starting value can overflow even in a bounded loop. The direct
/// C tier must retain its checked conditional add instead of applying the proof-gated transform.
#[test]
fn direct_c_rejects_unbounded_branchless_accumulator() {
    let source = "## To native args () -> Seq of Text\n\
## To native parseInt (s: Text) -> Int\n\
## Main\n\
Let arguments be args().\n\
Let mutable sum be parseInt(item 2 of arguments).\n\
Let mutable i be 0.\n\
While i is less than 100:\n    \
    If i is less than 50:\n        \
        Set sum to sum + 1.\n    \
    Set i to i + 1.\n\
Show sum.\n";
    let (direct, trace) =
        compile_to_c_traced(source).expect("compile unbounded accumulator through direct C");
    assert!(
        !trace.lowering.fired(Opt::BranchlessAccumulate),
        "an unbounded starting value must not take the unchecked branchless add"
    );
    assert!(!direct.contains("+= (int64_t)!!("));
    assert!(
        direct.contains("LInt sum"),
        "an unbounded accumulator must retain exact storage:\n{direct}"
    );
}

/// The overflow proof also requires real loop progress and at most one accumulator increment per
/// iteration. Merely looking like a counted loop is not sufficient.
#[test]
fn direct_c_rejects_uncertified_counting_loops() {
    let stalled_counter = "## Main\n\
Let mutable sum be 0.\n\
Let mutable i be 0.\n\
While i is less than 100:\n    \
    If i is less than 50:\n        \
        Set sum to sum + 1.\n\
Show sum.\n";
    let double_increment = "## Main\n\
Let mutable sum be 0.\n\
Let mutable i be 0.\n\
While i is less than 100:\n    \
    If i is less than 50:\n        \
        Set sum to sum + 1.\n    \
    If i is less than 75:\n        \
        Set sum to sum + 1.\n    \
    Set i to i + 1.\n\
Show sum.\n";

    for (name, source) in [
        ("stalled loop counter", stalled_counter),
        ("two increment sites", double_increment),
    ] {
        let (direct, trace) = compile_to_c_traced(source)
            .unwrap_or_else(|e| panic!("{name}: direct C compile failed: {e:?}"));
        assert!(
            !trace.lowering.fired(Opt::BranchlessAccumulate),
            "{name}: an uncertified loop must retain checked conditional adds"
        );
        assert!(!direct.contains("+= (int64_t)!!("), "{name}");
    }
}
