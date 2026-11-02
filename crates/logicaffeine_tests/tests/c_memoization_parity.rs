//! Direct C must consume the shared memoization transform whenever its
//! lowering can express the cache. This keeps an optimization from silently
//! becoming a Rust/bytecode-only feature.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::compile_to_c_traced;
use logicaffeine_compile::optimization::Opt;
use std::process::Command;
use tempfile::TempDir;

const FIB: &str = r#"## To fib (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Show fib(40).
"#;

fn gcc_available() -> bool {
    common::require_tool("gcc")
}

fn compile_and_run(c: &str) -> String {
    let dir = TempDir::new().expect("temporary C project");
    let source = dir.path().join("main.c");
    let binary = dir.path().join("main");
    std::fs::write(&source, c).expect("write generated C");
    let build = Command::new("gcc")
        .args(["-O3", "-march=native", "-flto", "-o"])
        .arg(&binary)
        .arg(&source)
        .arg("-lm")
        .output()
        .expect("run gcc");
    assert!(
        build.status.success(),
        "generated direct C must compile:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new(&binary).output().expect("run generated C");
    assert!(
        run.status.success(),
        "generated direct C exited {}",
        run.status
    );
    String::from_utf8(run.stdout).expect("program output is UTF-8")
}

#[test]
fn direct_c_fires_shared_memoization_and_runs_the_memoized_program() {
    if !gcc_available() {
        return;
    }

    let (c, trace) = compile_to_c_traced(FIB).expect("compile recursive fib through direct C");
    assert!(
        trace.optimizer.fired(Opt::Memo),
        "direct C must consume the shared memoization pass: {trace:?}"
    );
    assert_eq!(compile_and_run(&c), "102334155\n");
}

#[test]
fn direct_c_memoization_obeys_no_and_require_decorators() {
    let disabled = format!("## No memo\n{FIB}");
    let (_, disabled_trace) =
        compile_to_c_traced(&disabled).expect("compile direct C with memo disabled");
    assert!(
        !disabled_trace.all_fired().fired(Opt::Memo),
        "## No memo must disable the shared pass"
    );

    let required = format!("## Require memo\n{FIB}");
    let (_, required_trace) = compile_to_c_traced(&required)
        .expect("a memoizable direct-C program satisfies ## Require memo");
    assert!(required_trace.optimizer.fired(Opt::Memo));
}
