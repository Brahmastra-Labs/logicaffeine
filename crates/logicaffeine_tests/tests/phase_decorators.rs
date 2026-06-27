//! File-level optimization decorators: a `## No <X>` placed before `## Main`
//! (not attached to a `## To` function) disables that optimization program-wide,
//! so the generated Rust visibly changes. This is the mechanism the benchmarks
//! page drives: toggle an optimization → a `## No <X>` decorator appears on the
//! Logos source → the generated Rust changes.
//!
//! Each test pairs a CONTROL (the optimization fires by default) with the
//! decorated form (the optimization is gone), so it proves the toggle both ways.

use logicaffeine_compile::compile::compile_to_rust;
use logicaffeine_compile::optimization::REGISTRY;

fn bench_src(name: &str) -> String {
    let path = format!(
        "{}/../../benchmarks/programs/{}/main.lg",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"))
}

/// Insert a FILE-LEVEL `## No <name>` decorator: just before `## Main`, where the
/// parser folds it into the program-wide config (not a per-function annotation).
fn with_file_decorator(src: &str, name: &str) -> String {
    src.replace("## Main", &format!("## No {name}\n## Main"))
}

/// A quicksort whose partition accesses the oracle proves in range, so by default
/// codegen lowers them to `get_unchecked`/`get_unchecked_mut`.
const QUICKSORT: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To qs (arr: Seq of Int, lo: Int, hi: Int) -> Seq of Int:
    If lo is at least hi:
        Return arr.
    Let pivot be item hi of arr.
    Let mutable result be arr.
    Let mutable i be lo.
    Let mutable j be lo.
    While j is less than hi:
        If item j of result is at most pivot:
            Let tmp be item i of result.
            Set item i of result to item j of result.
            Set item j of result to tmp.
            Set i to i + 1.
        Set j to j + 1.
    Let tmp be item i of result.
    Set item i of result to item hi of result.
    Set item hi of result to tmp.
    Set result to qs(result, lo, i - 1).
    Set result to qs(result, i + 1, hi).
    Return result.

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable arr be a new Seq of Int.
Let mutable seed be 42.
Let mutable i be 0.
While i is less than n:
    Set seed to (seed * 1103515245 + 12345) % 2147483648.
    Push (seed / 65536) % 32768 to arr.
    Set i to i + 1.
Set arr to qs(arr, 1, n).
Let mutable checksum be 0.
Set i to 1.
While i is at most n:
    Set checksum to (checksum + item i of arr) % 1000000007.
    Set i to i + 1.
Show "" + item 1 of arr + " " + item n of arr + " " + checksum.
"#;

/// Two co-indexed `Seq of Float` arrays; by default they fuse into one
/// `[[f64; 2]; 3]` array-of-structs backing.
const AOS: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let seed be parseInt(item 2 of arguments).
Let mutable px be a new Seq of Float.
Let mutable py be a new Seq of Float.
Push 1.0 to px. Push 2.0 to py.
Push 3.0 to px. Push 4.0 to py.
Push 5.0 to px. Push 6.0 to py.
Let mutable total be 0.0.
Let mutable i be 1.
While i is at most seed:
    Set total to total + item i of px + item i of py.
    Set i to i + 1.
Show "{total:.1}".
"#;

#[test]
fn file_level_no_unchecked_keeps_bounds_checks() {
    // Control: by default the partition accesses elide to unchecked.
    let base = compile_to_rust(QUICKSORT).unwrap();
    assert!(
        base.contains(".get_unchecked("),
        "control: default quicksort should elide to get_unchecked"
    );

    // File-level `## No Unchecked` keeps every bounds check.
    let rust = compile_to_rust(&with_file_decorator(QUICKSORT, "Unchecked")).unwrap();
    assert!(
        !rust.contains(".get_unchecked("),
        "file-level `## No Unchecked` must keep reads checked. Got:\n{rust}"
    );
    assert!(
        !rust.contains(".get_unchecked_mut("),
        "file-level `## No Unchecked` must keep stores checked. Got:\n{rust}"
    );
}

#[test]
fn file_level_no_optimize_is_boring() {
    // The master switch disables every optimization program-wide.
    let rust = compile_to_rust(&with_file_decorator(QUICKSORT, "Optimize")).unwrap();
    assert!(
        !rust.contains(".get_unchecked("),
        "`## No Optimize` → boring Rust, no oracle-elided unchecked. Got:\n{rust}"
    );
}

#[test]
fn file_level_no_interleave_disables_aos() {
    // Control: by default the co-indexed float arrays fuse into an AoS backing.
    let base = compile_to_rust(AOS).unwrap();
    assert!(
        base.contains("[[f64; 2]; 3]"),
        "control: default should fuse co-indexed float arrays into [[f64; 2]; 3]. Got:\n{base}"
    );

    // File-level `## No Interleave` leaves them as separate arrays.
    let rust = compile_to_rust(&with_file_decorator(AOS, "Interleave")).unwrap();
    assert!(
        !rust.contains("[[f64; 2]; 3]"),
        "file-level `## No Interleave` must not fuse into an AoS backing. Got:\n{rust}"
    );
}

/// The user's #1 worry, on the CODEGEN side: disabling ONE optimization while the
/// rest stay on must never break codegen (panic, or emit empty/invalid Rust).
/// Every single-opt-disabled config — and the all-off config — must still produce
/// a non-empty Rust program for a spread of real benchmark shapes.
#[test]
fn codegen_survives_every_single_disable() {
    let mut programs: Vec<String> = ["nbody", "knapsack", "graph_bfs", "two_sum", "mergesort", "histogram", "collect"]
        .iter()
        .map(|n| bench_src(n))
        .collect();
    programs.push(QUICKSORT.to_string());
    programs.push(AOS.to_string());

    for src in &programs {
        // Control: it compiles at all-on.
        std::env::remove_var("LOGOS_OPT_OFF");
        assert!(
            compile_to_rust(src).map(|r| !r.is_empty()).unwrap_or(false),
            "program does not compile at the default config:\n{}",
            &src[..src.len().min(120)]
        );
        // Leave-one-out: disabling any single optimization keeps codegen valid.
        for m in REGISTRY {
            std::env::set_var("LOGOS_OPT_OFF", m.keyword);
            let r = compile_to_rust(src);
            std::env::remove_var("LOGOS_OPT_OFF");
            assert!(
                r.as_ref().map(|s| !s.is_empty()).unwrap_or(false),
                "disabling `{}` alone broke codegen ({:?}). Program head:\n{}",
                m.keyword,
                r.as_ref().err(),
                &src[..src.len().min(120)]
            );
        }
        // The boring all-off build must also be valid Rust.
        std::env::set_var("LOGOS_OPT", "off");
        let r = compile_to_rust(src);
        std::env::remove_var("LOGOS_OPT");
        assert!(
            r.map(|s| !s.is_empty()).unwrap_or(false),
            "all-off (boring) codegen failed for:\n{}",
            &src[..src.len().min(120)]
        );
    }
}

#[test]
fn optimizations_used_reports_firing_opts() {
    use logicaffeine_compile::compile::optimizations_used;
    // QUICKSORT's partition elides to unchecked indexing → `unchecked` fires.
    let used = optimizations_used(QUICKSORT);
    assert!(
        used.contains(&"unchecked"),
        "quicksort uses oracle-unchecked indexing; reported used: {used:?}"
    );
    // The AoS program fuses co-indexed arrays → `interleave` fires.
    let aos_used = optimizations_used(AOS);
    assert!(
        aos_used.contains(&"interleave"),
        "AoS program uses interleave; reported used: {aos_used:?}"
    );
}

#[test]
fn file_level_decorator_value_is_preserved() {
    // Disabling an optimization changes the SHAPE of the Rust, never breaks
    // compilation — both forms produce valid Rust.
    for decorated in [
        with_file_decorator(QUICKSORT, "Unchecked"),
        with_file_decorator(QUICKSORT, "Optimize"),
        with_file_decorator(AOS, "Interleave"),
    ] {
        assert!(compile_to_rust(&decorated).is_ok(), "decorated source must still compile:\n{decorated}");
    }
}
