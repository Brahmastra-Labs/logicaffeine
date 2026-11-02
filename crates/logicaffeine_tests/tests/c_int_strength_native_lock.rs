//! Direct C consumes the shared integer-induction recurrence proof. The hot
//! counted-loop clone keeps each guarded progression in a native word, while
//! the exact loop remains the fallback whenever the seed or final value cannot
//! fit `i64`.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::{compile_to_c, compile_to_c_traced, vm_outcome};
use logicaffeine_compile::optimization::Opt;

const ARRAY_FILL: &str = include_str!("../../../benchmarks/programs/array_fill/main.lg");

const OVERFLOWING_PROGRESSION: &str = r#"
## Main
Let n be 3.
Let mutable total be 0.
Let mutable i be 0.
While i is less than n:
    Set total to total + (i * 2 + 9223372036854775807).
    Set i to i + 1.
Show total.
"#;

fn fixed_array_fill(n: i64) -> String {
    ARRAY_FILL
        .replace("Let arguments be args().\n", "")
        .replace(
            "Let n be parseInt(item 2 of arguments).",
            &format!("Let n be {n}."),
        )
}

fn c_output(source: &str) -> String {
    let c = compile_to_c(source).expect("source compiles to direct C");
    let dir = tempfile::Builder::new()
        .prefix("logos_c_native_intstrength_")
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
fn array_fill_consumes_intstrength_as_native_c_arithmetic() {
    let (c, trace) = compile_to_c_traced(ARRAY_FILL).expect("array_fill compiles to direct C");
    assert!(
        trace.optimizer.fired(Opt::IntStrength),
        "shared IntStrength transform disappeared: {:?}",
        trace.optimizer.keywords()
    );
    assert!(
        trace.lowering.fired(Opt::IntStrength),
        "direct C silently ignored the IntStrength recurrence: {:?}",
        trace.lowering.keywords()
    );
    let hot = c
        .split("/* shared IntStrength: guarded native accumulators */")
        .nth(1)
        .and_then(|rest| rest.split("} else {").next())
        .expect("guarded native IntStrength branch");
    assert!(
        hot.contains("logos_imod") && hot.contains("+ 7LL"),
        "native recurrence vanished:\n{hot}"
    );
    assert!(
        !hot.contains("logos_lint_add") && !hot.contains("lint_mod"),
        "tagged exact arithmetic leaked into the IntStrength hot loop:\n{hot}"
    );
}

#[test]
fn native_and_exact_intstrength_paths_match_the_vm() {
    if !common::require_tool(&common::c_compiler()) {
        return;
    }
    for (name, source) in [
        ("native", fixed_array_fill(1024)),
        ("overflow-fallback", OVERFLOWING_PROGRESSION.to_string()),
    ] {
        let vm = vm_outcome(&source);
        assert!(vm.error.is_none(), "{name}: VM failed: {:?}", vm.error);
        assert_eq!(
            c_output(&source),
            vm.output,
            "{name}: direct C diverged from the exact VM"
        );
    }
}
