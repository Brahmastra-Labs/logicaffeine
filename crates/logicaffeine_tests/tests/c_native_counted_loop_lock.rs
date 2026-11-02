//! Direct C must consume the same counted-loop proof as Rust: a hot exact-`Int`
//! induction variable runs as a native `int64_t`, while values outside that proof
//! execute the original exact `LInt` loop.  This locks both halves so speed never
//! comes from wrapping or truncating LOGOS arbitrary-precision integers.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::{
    compile_to_c, compile_to_c_traced, vm_outcome, vm_outcome_with_args,
};
use logicaffeine_compile::optimization::Opt;

const FAST: &str = r#"
## Main
Let mutable xs be a new Seq of Int.
Let mutable i be 0.
While i is less than 1000:
    Push (i * 17 + 3) % 101 to xs.
    Set i to i + 1.
Let mutable fold be 0.
Set i to 1.
While i is less than or equal to length of xs:
    Set fold to (fold * 31 + item i of xs) & 268435455.
    Set i to i + 1.
Show fold.
Show i.
"#;

const BIG_COUNTER_SKIPS: &str = r#"
## Main
Let mutable i be 2 ** 100.
Let mutable trips be 0.
While i is less than 3:
    Set trips to trips + 1.
    Set i to i + 1.
Show i.
Show trips.
"#;

const INCLUSIVE_MAX_PROMOTES: &str = r#"
## Main
Let mutable i be 9223372036854775807.
Let mutable trips be 0.
While i is less than or equal to 9223372036854775807:
    Set trips to trips + 1.
    Set i to i + 1.
Show i.
Show trips.
"#;

const EXACT_BODY_STAYS_EXACT: &str = r#"
## Main
Let mutable i be 2 ** 100.
Set i to 2.
While i is less than 4:
    Show i * 5000000000000000000.
    Set i to i + 1.
Show i.
"#;

const EXACT_LIMIT_HAS_WORD_FAST_PATH: &str = r#"
## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let limit be n * 5.
Let mutable i be 2 ** 100.
Set i to 0.
Let mutable total be 0.
While i is less than limit:
    Set total to total + i.
    Set i to i + 1.
Show total.
Show i.
"#;

const BIG_LIMIT_SKIPS_EXACTLY: &str = r#"
## Main
Let limit be 0 - (2 ** 100).
Let mutable i be 0.
Let mutable trips be 0.
While i is less than limit:
    Set trips to trips + 1.
    Set i to i + 1.
Show i.
Show trips.
"#;

fn c_output(source: &str, args: &[&str]) -> String {
    let c = compile_to_c(source).expect("source compiles to direct C");
    let dir = tempfile::Builder::new()
        .prefix("logos_c_native_counted_")
        .tempdir()
        .expect("temporary C directory");
    let c_path = dir.path().join("main.c");
    let bin_path = dir.path().join("main");
    std::fs::write(&c_path, c).expect("write generated C");
    let build = common::build_c_release(&c_path, &bin_path).expect("C compiler starts");
    assert!(
        build.status.success(),
        "generated C builds:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new(&bin_path)
        .args(args)
        .output()
        .expect("generated C starts");
    assert!(
        run.status.success(),
        "generated C exits cleanly:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8_lossy(&run.stdout).into_owned()
}

#[test]
fn direct_c_emits_the_native_counted_loop_fast_path() {
    let (c, trace) =
        compile_to_c_traced(EXACT_BODY_STAYS_EXACT).expect("source compiles to traced direct C");
    assert!(
        c.contains("exact counted loop: native-word fast path, exact fallback"),
        "a proven counted exact-Int loop silently lost its native C fast path"
    );
    assert!(
        c.contains("__builtin_expect(!i.big") || c.contains("__builtin_expect(!__logos_i_"),
        "the fast path must be guarded by the exact counter's native representation:\n{c}"
    );
    assert!(
        trace.lowering.fired(Opt::Peephole),
        "the native counted-loop lowering silently disappeared from the C backend trace: {:?}",
        trace.lowering.keywords()
    );
}

#[test]
fn direct_c_guards_an_exact_limit_before_using_its_word_value() {
    let (c, trace) = compile_to_c_traced(EXACT_LIMIT_HAS_WORD_FAST_PATH)
        .expect("source compiles to traced direct C");
    assert!(
        c.contains("exact counted loop: native-word fast path, exact fallback"),
        "an exact limit with a small runtime value silently lost its native C fast path:\n{c}"
    );
    assert!(
        c.contains("__count_exact_hi") && c.contains(".big ? 0LL"),
        "the exact limit must be representation-guarded before its word value is used:\n{c}"
    );
    assert!(
        trace.lowering.fired(Opt::Peephole),
        "the guarded exact-limit lowering must remain visible in the C trace"
    );
    if common::require_tool(&common::c_compiler()) {
        let vm_args = vec!["bench".to_string(), "200".to_string()];
        let vm = vm_outcome_with_args(EXACT_LIMIT_HAS_WORD_FAST_PATH, &vm_args, None);
        assert!(vm.error.is_none(), "VM failed: {:?}", vm.error);
        assert_eq!(
            c_output(EXACT_LIMIT_HAS_WORD_FAST_PATH, &["200"]),
            vm.output,
            "the guarded exact-limit C fast path diverged from the exact VM"
        );
    }
}

#[test]
fn native_and_exact_fallbacks_both_match_the_vm() {
    if !common::require_tool(&common::c_compiler()) {
        return;
    }
    for (name, source) in [
        ("native", FAST),
        ("big-counter", BIG_COUNTER_SKIPS),
        ("inclusive-max", INCLUSIVE_MAX_PROMOTES),
        ("exact-body", EXACT_BODY_STAYS_EXACT),
        ("big-limit", BIG_LIMIT_SKIPS_EXACTLY),
    ] {
        let vm = vm_outcome(source);
        assert!(vm.error.is_none(), "{name}: VM failed: {:?}", vm.error);
        assert_eq!(
            c_output(source, &[]),
            vm.output,
            "{name}: direct C diverged from the exact VM"
        );
    }
}
