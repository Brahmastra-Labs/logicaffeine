//! ★ LOCK — `pi`, `π` and `e` are the real constants wherever no exact type is declared, on every backend. ★
//!
//! In an exact context (a declared `Rational`, `Algebraic`, `ExactReal` or `Hyperreal`) the names `pi` (`π`) and `e`
//! are the exact constants, `exactPi()` and `exactE()` (`exact_by_context`); everywhere else they are the `Real`
//! constants, the doubles nearest π and e, as the quick guide's exact-functions row says. A name the scope binds — a
//! `Let`, a parameter, a closure's parameter, a loop's pattern, an arm's binding — is that binding and never a
//! constant, and `pi(digits)` is still the standard library's digits. The tree-walker prints what each program says
//! here, and every backend that runs a program prints the tree-walker's bytes; the one program that reaches the exact
//! extension skips the self-contained WebAssembly module, which refuses that extension by name (`exact_on_wasm_lock`).
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test real_constants_every_tier_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

use logicaffeine_compile::compile::tw_outcome;
use tier_parity::{Backend, Program};

/// Each program, and what it prints.
const PROGRAMS: &[(&str, &str, &str)] = &[
    (
        "the_nearest_doubles",
        "## Main\nShow pi.\nShow π.\nShow e.\n",
        "3.141592653589793\n3.141592653589793\n2.718281828459045\n",
    ),
    (
        "in_real_arithmetic",
        "## Main\nLet r be 2.0.\nShow pi * r * r.\nLet x: Real be e.\nShow x * x.\nShow sqrt(pi).\nShow 2.0 * π.\n",
        "12.566370614359172\n7.3890560989306495\n1.7724538509055159\n6.283185307179586\n",
    ),
    (
        "beside_a_number_read_at_run_time",
        "## To native parseInt (s: Text) -> Int\n\n## Main\nLet r be 2.0 * parseInt(\"1\").\nShow pi * r * r.\nShow e * parseInt(\"3\").\n",
        "12.566370614359172\n8.154845485377136\n",
    ),
    (
        "in_a_function_a_closure_a_condition_a_list_and_a_text",
        "## To area (r: Real) -> Real:\n    Return pi * r * r.\n\n## Main\nShow area(2.0).\nLet circumference be (d: Real) -> pi * d.\nShow circumference(2.0).\nIf e is greater than 2.7:\n    Show \"above\".\nLet xs be [pi, e].\nShow length of xs.\nShow item 2 of xs.\nShow \"pi is {pi}\".\n",
        "12.566370614359172\n6.283185307179586\nabove\n2\n2.718281828459045\npi is 3.141592653589793\n",
    ),
    (
        "a_let_named_e_or_pi",
        "## Main\nLet e be 5.\nShow e + 1.\nLet pi be 3.\nShow pi * 2.\n",
        "6\n6\n",
    ),
    (
        "a_parameter_named_e",
        "## To twice (e: Int) -> Int:\n    Return e * 2.\n\n## Main\nShow twice(4).\n",
        "8\n",
    ),
    (
        "a_closure_parameter_named_e",
        "## Main\nLet f be (e: Int) -> e + 1.\nShow f(2).\n",
        "3\n",
    ),
    (
        "a_loop_variable_named_e",
        "## Main\nRepeat for e in [1, 2]:\n    Show e.\n",
        "1\n2\n",
    ),
    (
        "an_arm_binding_named_e",
        "## A Shape is either:\n    A Circle with radius Int.\n    A Dot.\n\n## Main\nLet s be a new Circle with radius 7.\nInspect s:\n    When Circle (e): Show e.\n    When Dot: Show \"dot\".\n",
        "7\n",
    ),
    (
        "pi_of_a_count_of_digits",
        "## Main\nShow pi(3).\n",
        "3.141\n",
    ),
];

/// The program that reaches the exact extension, and what it prints.
const EXACT: (&str, &str, &str) = (
    "both_readings_in_one_program",
    "## Main\nLet exact: ExactReal be pi.\nLet near be pi.\nShow exact.\nShow near.\n",
    "π\n3.141592653589793\n",
);

/// Every backend that runs a program: the engines, the generated Rust, both C lowerings, both WebAssembly modules
/// and RISC-V.
const BACKENDS: &[Backend] = &[
    Backend::TreeWalker,
    Backend::Vm,
    Backend::VmOptimized,
    Backend::VmForge,
    Backend::Interpret,
    Backend::RustDebug,
    Backend::RustRelease,
    Backend::C,
    Backend::CBytecode,
    Backend::Wasm,
    Backend::WasmLinked,
    Backend::RiscV,
];

/// The child side of every isolated backend run. Does nothing unless `LOGOS_PARITY_JOB` is set.
#[test]
#[ignore = "the worker the parity sweep re-executes this binary into"]
fn tier_parity_worker() {
    tier_parity::worker_main();
}

fn program((id, source, _): &(&str, &str, &str)) -> Program {
    Program::new(format!("real_constants/{id}"), *source)
}

fn programs() -> Vec<Program> {
    PROGRAMS.iter().chain([&EXACT]).map(program).collect()
}

#[test]
fn every_program_here_reaches_the_backends() {
    let refused: Vec<String> = programs()
        .into_iter()
        .filter(|p| tier_parity::rejection(&p.source).is_some())
        .map(|p| p.id)
        .collect();
    assert!(refused.is_empty(), "the front end refuses {refused:?}, which pin nothing");
}

#[test]
fn the_tree_walker_prints_what_each_program_says() {
    for (id, source, printed) in PROGRAMS.iter().chain([&EXACT]) {
        let run = tw_outcome(source);
        assert_eq!(run.error, None, "{id}: the tree-walker refused:\n{source}");
        assert_eq!(run.output.trim_end(), printed.trim_end(), "{id}:\n{source}");
    }
}

#[test]
fn every_backend_prints_the_tree_walkers_bytes() {
    let plain: Vec<Program> = PROGRAMS.iter().map(program).collect();
    let exact_backends: Vec<Backend> = BACKENDS.iter().copied().filter(|b| *b != Backend::Wasm).collect();
    let mut rows = tier_parity::sweep(&plain, BACKENDS, 4).unwrap_or_else(|why| panic!("the sweep could not start: {why}"));
    rows.extend(
        tier_parity::sweep(&[program(&EXACT)], &exact_backends, 4).unwrap_or_else(|why| panic!("the sweep could not start: {why}")),
    );
    let bad: Vec<String> = rows
        .iter()
        .filter(|row| !row.matches())
        .map(|row| format!("{} on {}: {} {}", row.program, row.backend.name(), row.status(), row.detail()))
        .collect();
    assert!(bad.is_empty(), "{} runs differ from the tree-walker:\n{}", bad.len(), bad.join("\n"));
}
