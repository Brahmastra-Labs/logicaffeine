//! ★ LOCK — the exact extension runs on C as on the VM. ★
//!
//! A value the exact extension reaches (an infinitesimal, a hyperreal, an infinite integer, an algebraic
//! number, a computable real, an ordinal, a residue-dependent or a provisional answer) is held by both
//! C backends as a handle into base's own Rust, linked as one static library behind the `logos_rt_value_*`
//! C ABI, so every such value computes, prints, compares and fails as the VM's does: one implementation of
//! the extension, not a mirror. A program the extension never reaches stays self-contained C, its link
//! line no longer than it was. A handle no longer reached is dropped by the collector, so a loop that
//! makes exact values runs in the memory it reaches.

#![cfg(not(target_arch = "wasm32"))]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

mod common;

use std::process::Command;

use logicaffeine_compile::codegen_c::value_runtime;
use logicaffeine_compile::compile;
use tier_parity::{exact, Backend, Program};

/// The child side of every isolated backend run. Does nothing unless `LOGOS_PARITY_JOB` is set.
#[test]
#[ignore = "the worker the parity sweep re-executes this binary into"]
fn tier_parity_worker() {
    tier_parity::worker_main();
}

fn differ(programs: &[Program]) -> Vec<String> {
    let rows = tier_parity::sweep(
        programs,
        &[Backend::TreeWalker, Backend::C, Backend::CBytecode],
        4,
    )
    .unwrap_or_else(|why| panic!("the sweep could not start: {why}"));
    rows.iter()
        .filter(|row| !row.matches())
        .map(|row| {
            format!(
                "{} on {}: {} {}",
                row.program,
                row.backend.name(),
                row.status(),
                row.detail()
            )
        })
        .collect()
}

#[test]
fn every_program_here_reaches_the_backends() {
    let refused: Vec<&str> = exact::PROGRAMS
        .iter()
        .filter(|(_, source)| tier_parity::rejection(source).is_some())
        .map(|(id, _)| *id)
        .collect();
    assert!(
        refused.is_empty(),
        "the front end refuses {} program(s), which pin nothing: {refused:?}",
        refused.len()
    );
    assert!(
        exact::probes().len() >= 30,
        "the extension's semantic probes are the bulk of this lock; only {} were found",
        exact::probes().len()
    );
}

#[test]
fn every_extension_probe_runs_on_both_c_backends_as_on_the_vm() {
    let bad = differ(&exact::probes());
    assert!(
        bad.is_empty(),
        "{} (probe, backend) runs differ from the VM:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn every_boundary_of_a_c_program_holds_an_exact_value_as_the_vm_does() {
    let bad = differ(&exact::programs());
    assert!(
        bad.is_empty(),
        "{} (program, backend) runs differ from the VM:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn a_program_the_extension_never_reaches_links_nothing_more() {
    for (backend, emitted) in [
        ("c", compile::compile_to_c(exact::PLAIN)),
        ("c-bytecode", compile::compile_to_c_via_bytecode(exact::PLAIN)),
    ] {
        let code = emitted.unwrap_or_else(|e| panic!("{backend}: {e:?}"));
        assert!(
            !value_runtime::links(&code),
            "{backend}: a program without the extension asks for the value runtime"
        );
        assert!(
            !code.contains("logos_rt_value_"),
            "{backend}: a program without the extension names the value runtime's ABI"
        );
        assert_eq!(
            value_runtime::link_args(&code).expect("no runtime to build"),
            Vec::<String>::new(),
            "{backend}: a program without the extension links more than its own source"
        );
    }
}

#[test]
fn a_program_the_extension_reaches_asks_for_the_value_runtime() {
    let source = "## Main\nLet dt be an infinitesimal.\nShow dt * 2.\n";
    for (backend, emitted) in [
        ("c", compile::compile_to_c(source)),
        ("c-bytecode", compile::compile_to_c_via_bytecode(source)),
    ] {
        let code = emitted.unwrap_or_else(|e| panic!("{backend}: {e:?}"));
        assert!(
            value_runtime::links(&code),
            "{backend}: a program the extension reaches does not ask for the value runtime"
        );
    }
}

/// What a program that links the value runtime puts on its link line after its own source: the archive, then every
/// system library rustc names for it, in rustc's order.
#[test]
fn the_link_line_is_the_archive_then_the_system_libraries_it_calls() {
    let code = compile::compile_to_c("## Main\nLet dt be an infinitesimal.\nShow dt * 2.\n").expect("emits");
    let archive = value_runtime::archive().unwrap_or_else(|why| panic!("the value runtime did not build: {why}"));
    assert!(archive.path.is_file(), "{} is not an archive on disk", archive.path.display());
    assert!(!archive.system_libs.is_empty(), "rustc named no system library for the value runtime");
    assert!(
        archive.system_libs.iter().all(|l| l.starts_with('-')),
        "not a link flag among {:?}",
        archive.system_libs
    );
    let mut expected = vec![archive.path.display().to_string()];
    expected.extend(archive.system_libs.iter().cloned());
    assert_eq!(value_runtime::link_args(&code).expect("links"), expected);
}

/// Build `source` with one C backend, linking what it asks for, and run it with `env`.
fn build_and_run(
    backend: &str,
    code: &str,
    env: &[(&str, &str)],
) -> std::process::Output {
    let dir = tier_parity::scratch_dir(&format!("exact-on-c-{backend}"));
    std::fs::write(dir.join("prog.c"), code).expect("write C");
    let mut gcc = Command::new("gcc");
    gcc.args(["-O2", "-o", "prog", "prog.c"])
        .args(value_runtime::link_args(code).unwrap_or_else(|why| panic!("{backend}: {why}")))
        .arg("-lm")
        .current_dir(&dir);
    let built = gcc.output().expect("gcc starts");
    assert!(
        built.status.success(),
        "{backend}: the emitted C does not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let mut run = Command::new(dir.join("prog"));
    for (k, v) in env {
        run.env(k, v);
    }
    let out = run.output().expect("the program starts");
    let _ = std::fs::remove_dir_all(&dir);
    out
}

/// Every value a loop lets go of is dropped: the most handles alive at once stays a small fraction of the
/// handles made, as the heap report counts them.
#[test]
fn the_values_a_loop_lets_go_of_are_freed() {
    if !common::require_tool("gcc") {
        return;
    }
    let made = 200_000;
    let source = format!(
        "## Main\nLet dt be an infinitesimal.\nLet mutable x be dt.\nLet mutable n be 0.\nRepeat for i from 1 to {made}:\n    Set x to dt * i + 1.\n    If x > 1:\n        Set n to n + 1.\nShow n.\nShow x.\n"
    );
    let expected = common::run_all_tiers(&source);
    assert_eq!(expected.error, None);
    for (backend, emitted) in [
        ("c", compile::compile_to_c(&source)),
        ("c-bytecode", compile::compile_to_c_via_bytecode(&source)),
    ] {
        let code = emitted.unwrap_or_else(|e| panic!("{backend}: {e:?}"));
        let out = build_and_run(backend, &code, &[("LOGOS_HEAP_REPORT", "1")]);
        assert!(out.status.success(), "{backend}: {}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), expected.output.trim_end(), "{backend}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        let report = stderr
            .lines()
            .find_map(|l| l.strip_prefix("logos heap: "))
            .unwrap_or_else(|| panic!("{backend}: no heap report on stderr:\n{stderr}"));
        let field = |name: &str| -> u64 {
            report
                .split_whitespace()
                .find_map(|kv| kv.strip_prefix(&format!("{name}=")))
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(|| panic!("{backend}: no `{name}` in `{report}`"))
        };
        let (values, values_high) = (field("values"), field("values_high"));
        assert!(
            values_high >= 1 && values_high < made / 4,
            "{backend}: {values_high} exact values were alive at once of the {made}+ the loop made"
        );
        assert!(
            values <= values_high,
            "{backend}: {values} values alive at the end, more than the {values_high} ever alive at once"
        );
    }
}

/// A handle reached only from the heap — an item of a list held in a field — survives every collection:
/// with a collection at each allocation, the program prints what the VM prints.
#[test]
fn a_handle_reached_through_the_heap_survives_every_collection() {
    if !common::require_tool("gcc") {
        return;
    }
    let source = "## A Box has:\n    a value, which is Hyperreal.\n\n## Main\nLet dt be an infinitesimal.\nLet mutable boxes be a new Seq of Box.\nRepeat for i from 1 to 50:\n    Push a new Box with value dt * i to boxes.\nLet mutable total: Hyperreal be 0.\nRepeat for b in boxes:\n    Set total to total + b's value.\nShow total.\nLet last be item 50 of boxes.\nShow last's value.\n";
    let expected = common::run_all_tiers(source);
    assert_eq!(expected.error, None);
    for (backend, emitted) in [
        ("c", compile::compile_to_c(source)),
        ("c-bytecode", compile::compile_to_c_via_bytecode(source)),
    ] {
        let code = emitted.unwrap_or_else(|e| panic!("{backend}: {e:?}"));
        let out = build_and_run(backend, &code, &[("LOGOS_GC_STRESS", "1")]);
        assert!(out.status.success(), "{backend}: {}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), expected.output.trim_end(), "{backend}");
    }
}

/// A real number the program defines by a function, read after the allocations of a loop: the runtime alone reaches
/// the function's closure, so the program keeps it reached for as long as it runs.
const DEFINES_UNDER_PRESSURE: &str = "## To seventh (i: Int) -> Int:\n    If i = 0:\n        Return 0.\n    Return item ((i - 1) % 6 + 1) of [1, 4, 2, 8, 5, 7].\n\n## Main\nLet r be the real number whose digits in base 10 are seventh.\nLet q: Rational be 1 / 8.\nLet mutable rows be a new Seq of Seq of Int.\nRepeat for i from 1 to 300:\n    Push [i, i * 2, i * 3] to rows.\nShow r > q.\nRepeat for i from 1 to 300:\n    Push [i] to rows.\nShow floor(r * 10 ** 24).\nShow r.\nShow length of rows.\n";

/// With a collection at each allocation, and allocations between the definition and every read of its function, a
/// program that defines a real number by a function prints what the VM prints.
#[test]
fn a_function_a_real_number_is_defined_by_survives_every_collection() {
    if !common::require_tool("gcc") {
        return;
    }
    let expected = common::run_all_tiers(DEFINES_UNDER_PRESSURE);
    assert_eq!(expected.error, None);
    for (backend, emitted) in [
        ("c", compile::compile_to_c(DEFINES_UNDER_PRESSURE)),
        ("c-bytecode", compile::compile_to_c_via_bytecode(DEFINES_UNDER_PRESSURE)),
    ] {
        let code = emitted.unwrap_or_else(|e| panic!("{backend}: {e:?}"));
        let out = build_and_run(backend, &code, &[("LOGOS_GC_STRESS", "1")]);
        assert!(out.status.success(), "{backend}: {}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim_end(), expected.output.trim_end(), "{backend}");
    }
}

/// The reader the runtime calls a program's function back through is emitted for a program that defines a real
/// number by a function, and for no other: one the extension reaches by an infinitesimal alone has none.
#[test]
fn only_a_program_that_defines_a_real_emits_the_reader() {
    let other = "## Main\nLet dt be an infinitesimal.\nShow dt * 2.\n";
    type Emit = fn(&str) -> Result<String, logicaffeine_compile::ParseError>;
    for (backend, reader, emit) in [
        ("c", "__logos_defines_", compile::compile_to_c as Emit),
        ("c-bytecode", "lc_defines", compile::compile_to_c_via_bytecode as Emit),
    ] {
        let defines = emit(DEFINES_UNDER_PRESSURE).unwrap_or_else(|e| panic!("{backend}: {e:?}"));
        assert!(defines.contains(reader), "{backend}: a program that defines a real has no `{reader}`");
        assert!(defines.contains("ltower_define("), "{backend}: the definition is not the runtime's");
        let plain = emit(other).unwrap_or_else(|e| panic!("{backend}: {e:?}"));
        assert!(!plain.contains(reader), "{backend}: a program that defines no real emits `{reader}`");
    }
}

/// The CUDA tier runs a program on a GPU thread, which has no way to call the host's value runtime: it refuses a
/// program the extension reaches by name instead of failing inside the device compiler.
#[test]
fn the_gpu_tier_refuses_an_exact_program_by_name() {
    let refused = match compile::compile_to_cuda("## Main\nLet dt be an infinitesimal.\nShow dt * 2.\n") {
        Ok(_) => panic!("the CUDA tier accepted a program the exact extension reaches"),
        Err(e) => format!("{e:?}"),
    };
    assert!(
        refused.contains("does not run the exact extension"),
        "the CUDA tier must name what it refuses: {refused}"
    );
    let refused = match compile::compile_to_cuda(DEFINES_UNDER_PRESSURE) {
        Ok(_) => panic!("the CUDA tier accepted a program that defines a real number by a function"),
        Err(e) => format!("{e:?}"),
    };
    assert!(refused.contains("does not run the exact extension"), "{refused}");
}

/// The bytecode carries each declared exact type as the type it is (`BoundaryType::Builtin`), so a backend that lowers
/// the bytecode types a parameter, a result and a field of one as an exact value; before, the bytecode left them
/// unmodeled and a record holding one fell out of its declared layouts.
#[test]
fn the_bytecode_carries_each_declared_exact_type() {
    use logicaffeine_compile::vm::BoundaryType;
    let program = compile::compile_to_program_optimized(
        "## A Box has:\n    a v, which is Ordinal.\n\n## No Inline\n## To half (x: Hyperreal) -> ExactReal:\n    Return x / 2.\n\n## Main\nLet b be a new Box with v 3.\nShow b's v.\nShow half(3).\n",
    )
    .expect("compiles");
    let half = program
        .functions
        .iter()
        .find(|f| f.param_types.len() == 1)
        .expect("half survives into the bytecode");
    assert_eq!(half.param_types[0], Some(BoundaryType::Builtin("Hyperreal".to_string())));
    assert_eq!(half.return_type, Some(BoundaryType::Builtin("ExactReal".to_string())));
    let boxed = program
        .struct_types
        .iter()
        .find(|s| s.name == "Box")
        .expect("Box keeps its declared layout");
    assert_eq!(boxed.fields, vec![("v".to_string(), BoundaryType::Builtin("Ordinal".to_string()))]);
}
