//! Direct C lowers proven descending exact counters through the same native
//! word representation Rust uses, retaining the original exact loop whenever
//! an endpoint or body offset can cross `i64`.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::{compile_to_c, compile_to_c_traced, vm_outcome};
use logicaffeine_compile::optimization::Opt;

/// Heap sort as the benchmark ships it: `n` comes from the command line, so neither down counter
/// has a bound the optimizer can see and both stay exact `Int`s.
const HEAP_FROM_ARGS: &str = include_str!("../../../benchmarks/programs/heap_sort/main.lg");

/// Heap sort with `n` a constant: the loop-exit facts bound both down counters.
fn heap_source(n: usize) -> String {
    HEAP_FROM_ARGS.replace(
        "Let arguments be args().\nLet n be parseInt(item 2 of arguments).",
        &format!("Let n be {n}."),
    )
}

const UNDERFLOW_EDGE: &str = r#"
## Main
Let mutable i be -9223372036854775808.
While i is at least -9223372036854775808:
    Set i to i - 1.
Show i.
"#;

fn c_output(source: &str) -> String {
    let c = compile_to_c(source).expect("source compiles to direct C");
    let dir = tempfile::Builder::new()
        .prefix("logos_c_desc_count_")
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
fn heap_sort_down_counters_are_native_words() {
    let (c, trace) =
        compile_to_c_traced(HEAP_FROM_ARGS).expect("heap sort compiles to traced direct C");
    assert!(
        c.matches("/* exact descending loop: guarded native-word fast path */")
            .count()
            >= 2,
        "both heap construction and extraction loops must take the descending fast path:\n{c}"
    );
    assert!(
        trace.lowering.fired(Opt::Peephole),
        "descending representation lowering disappeared from the C trace: {:?}",
        trace.lowering.keywords()
    );

    for fast in c
        .split("/* exact descending loop: guarded native-word fast path */")
        .skip(1)
    {
        let hot = fast
            .split("\n        } else {\n")
            .next()
            .expect("guarded descending loop has a hot branch");
        assert!(
            hot.contains("for (;") && hot.contains("--"),
            "missing native descending loop:\n{hot}"
        );
        assert!(
            !hot.contains("logos_lint_cmp")
                && !hot.contains("logos_lint_add")
                && !hot.contains("logos_lint_sub"),
            "tagged exact arithmetic leaked into a descending hot loop:\n{hot}"
        );
    }
}

/// A counter whose run the optimizer bounds is a machine word outright: no guard, no exact
/// fallback, no exact arithmetic anywhere in either down loop.
#[test]
fn bounded_down_counters_need_no_guard() {
    let c = compile_to_c(&heap_source(64)).expect("heap sort compiles to direct C");
    assert!(
        !c.contains("/* exact descending loop: guarded native-word fast path */"),
        "a bounded counter took the guarded exact path:\n{c}"
    );
    let main = c.split("int main(").nth(1).expect("generated C has a main");
    for counter in ["start", "end"] {
        assert!(
            main.contains(&format!("int64_t {counter} = ")),
            "`{counter}` is not a machine word:\n{main}"
        );
    }
    for down in [
        "while (((start >= 0LL))) {",
        "while (((end > 0LL))) {",
    ] {
        let body = main
            .split(down)
            .nth(1)
            .unwrap_or_else(|| panic!("missing native down loop `{down}`:\n{main}"))
            .split("\n    }\n")
            .next()
            .expect("the down loop closes");
        assert!(
            !body.contains("lint_") && !body.contains("logos_lint_"),
            "exact arithmetic in a bounded down loop:\n{body}"
        );
    }
}

#[test]
fn descending_fast_path_and_underflow_fallback_match_the_vm() {
    if !common::require_tool(&common::c_compiler()) {
        return;
    }
    for (name, source) in [
        ("heap", heap_source(128)),
        ("underflow", UNDERFLOW_EDGE.to_string()),
    ] {
        let vm = vm_outcome(&source);
        assert!(vm.error.is_none(), "{name}: VM failed: {:?}", vm.error);
        assert_eq!(
            c_output(&source),
            vm.output,
            "{name}: direct C diverged from the VM"
        );
    }
}
