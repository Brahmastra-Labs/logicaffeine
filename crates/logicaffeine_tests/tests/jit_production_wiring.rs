//! The EXODIA endgame: the JIT is not a test fixture — it is the PRODUCTION
//! tier. `logicaffeine_jit::install()` registers the forge tier process-wide,
//! and the LIVE engine entry points (`interpret_for_ui_sync`, the path the
//! studio/CLI sync runs ride) must pick it up: hot Main loops and hot
//! functions tier to native inside ordinary program runs, with outcomes
//! identical to pure bytecode (the debug shadow oracle re-checks every run
//! against the tree-walker on top).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::ui_bridge::interpret_for_ui_sync;

/// install() is idempotent and returns the same process-wide tier; its
/// counters only ever grow, so deltas around a run isolate that run's work —
/// PROVIDED runs don't overlap. The tier is process-global and the test
/// harness is concurrent, so a mutex serializes the delta windows.
static TIER_WINDOW: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn window() -> std::sync::MutexGuard<'static, ()> {
    TIER_WINDOW.lock().unwrap_or_else(|p| p.into_inner())
}

fn run_live(src: &str) -> (Vec<String>, Option<String>, (u32, u32), (u32, u32)) {
    let tier = logicaffeine_jit::install();
    let (f0, r0) = (tier.function_counts(), tier.region_counts());
    let result = interpret_for_ui_sync(src);
    let (f1, r1) = (tier.function_counts(), tier.region_counts());
    (
        result.lines,
        result.error,
        (f1.0 - f0.0, f1.1 - f0.1),
        (r1.0 - r0.0, r1.1 - r0.1),
    )
}

#[test]
fn live_engine_tiers_hot_main_loop_as_region() {
    let _window = window();
    let src = "\
## Main
Let mutable i be 0.
Let mutable total be 0.
While i is less than 2000:
    Set total to total + i * i.
    Set i to i + 1.
Show total.
";
    let (lines, error, _fn_counts, region_counts) = run_live(src);
    assert_eq!(error, None);
    assert_eq!(lines, vec!["2664667000".to_string()]);
    assert_eq!(
        region_counts,
        (1, 1),
        "the LIVE engine must region-tier the hot Main loop"
    );
}

#[test]
fn live_engine_tiers_hot_function() {
    let _window = window();
    let src = "\
## To sq (n: Int) -> Int:
    Return n * n.

## Main
Let mutable i be 0.
Let mutable acc be 0.
While i is less than 500:
    Set acc to acc + sq(i).
    Set i to i + 1.
Show acc.
";
    let (lines, error, fn_counts, _region_counts) = run_live(src);
    assert_eq!(error, None);
    assert_eq!(lines, vec!["41541750".to_string()]);
    assert_eq!(
        fn_counts,
        (1, 1),
        "the LIVE engine must tier the hot function exactly once"
    );
}

#[test]
fn live_engine_unsupported_programs_stay_correct() {
    let _window = window();
    // Text accumulation is outside the JIT subset: the tier may attempt and
    // bail, but the program must run bit-identically on bytecode.
    let src = "\
## Main
Let mutable i be 0.
Let mutable out be \"\".
While i is less than 150:
    Set out to \"{out}x\".
    Set i to i + 1.
Show length of out.
";
    let (lines, error, fn_counts, region_counts) = run_live(src);
    assert_eq!(error, None);
    assert_eq!(lines, vec!["150".to_string()]);
    assert_eq!(fn_counts.1, 0, "nothing here can compile as a function");
    assert_eq!(region_counts.1, 0, "a text loop must not compile as a region");
}

#[test]
fn live_engine_outcomes_match_untiered_outcomes() {
    let _window = window();
    // The same programs through vm_outcome (untiered constructor today) and
    // the live tiered engine must agree on every line.
    for src in [
        "## Main\nLet mutable i be 0.\nLet mutable t be 0.\nWhile i is less than 300:\n    Set t to t + i.\n    Set i to i + 1.\nShow t.\n",
        "## Main\nLet a be 9223372036854775807.\nShow a + 1.\n",
        "## Main\nLet mutable n be 20.\nLet mutable acc be 1.\nWhile n is greater than 1:\n    Set acc to acc * n.\n    Set n to n - 1.\nShow acc.\n",
    ] {
        let (lines, error, _, _) = run_live(src);
        let pure = logicaffeine_compile::compile::vm_outcome(src);
        let pure_lines: Vec<String> =
            pure.output.lines().map(|l| l.to_string()).collect();
        assert_eq!((lines, error), (pure_lines, pure.error), "diverged on:\n{src}");
    }
}
