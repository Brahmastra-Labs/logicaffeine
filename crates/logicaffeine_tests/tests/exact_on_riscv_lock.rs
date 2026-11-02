//! ★ LOCK — the exact extension runs on RISC-V as on the VM. ★
//!
//! A value the exact extension reaches is held by a RISC-V program as a handle into base's own Rust: the same
//! `logos_rt_value_*` C ABI the C backends and linked WebAssembly link, built for `riscv64gc-unknown-linux-gnu` and
//! linked statically beside the runtime shim, so every such value computes, prints, compares and fails as the VM's
//! does. A program the extension never reaches links the shim alone, as it did. A handle no longer reached is dropped
//! by the shim's collector, so a loop that makes exact values runs in the memory it reaches.

#![cfg(not(target_arch = "wasm32"))]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

mod common;
#[path = "support/riscv_runner.rs"]
mod riscv_runner;

use logicaffeine_compile::codegen_c::value_runtime::{self, Target};
use logicaffeine_compile::codegen_riscv;
use logicaffeine_compile::compile;
use tier_parity::{exact, Backend, Program};

/// The child side of every isolated backend run. Does nothing unless `LOGOS_PARITY_JOB` is set.
#[test]
#[ignore = "the worker the parity sweep re-executes this binary into"]
fn tier_parity_worker() {
    tier_parity::worker_main();
}

fn differ(programs: &[Program]) -> Vec<String> {
    let rows = tier_parity::sweep(programs, &[Backend::TreeWalker, Backend::RiscV], 4)
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
fn every_extension_probe_runs_on_riscv_as_on_the_vm() {
    let probes = exact::probes();
    assert!(
        probes.len() >= 30,
        "the extension's semantic probes are the bulk of this lock; only {} were found",
        probes.len()
    );
    let bad = differ(&probes);
    assert!(
        bad.is_empty(),
        "{} (probe, backend) runs differ from the VM:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn every_boundary_of_a_riscv_program_holds_an_exact_value_as_the_vm_does() {
    let bad = differ(&exact::programs());
    assert!(
        bad.is_empty(),
        "{} (program, backend) runs differ from the VM:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn a_program_the_extension_never_reaches_links_the_shim_alone() {
    let asm = compile::compile_to_riscv(exact::PLAIN).unwrap_or_else(|e| panic!("{e:?}"));
    let runtime = codegen_riscv::runtime(&asm).unwrap_or_else(|why| panic!("{why}"));
    assert!(!codegen_riscv::links_value_runtime(&asm), "a program without the extension asks for the value runtime");
    assert_eq!(runtime.c, codegen_riscv::RISCV_RUNTIME_C, "a program without the extension carries the extension's adapter");
    assert_eq!(runtime.link_args, vec!["-lm".to_string()], "a program without the extension links more than the shim");
    assert!(!asm.contains("logos_tower_"), "a program without the extension calls the extension's adapter");
}

/// What a program that links the value runtime puts on its link line after the assembly and the shim: the archive
/// built for RISC-V, then every system library rustc names for it as a static link spells it, then the math library.
#[test]
fn a_program_the_extension_reaches_links_the_value_runtime_built_for_riscv() {
    let asm = compile::compile_to_riscv("## Main\nLet dt be an infinitesimal.\nShow dt * 2.\n")
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(codegen_riscv::links_value_runtime(&asm), "a program the extension reaches does not ask for the value runtime");
    let runtime = codegen_riscv::runtime(&asm).unwrap_or_else(|why| panic!("{why}"));
    assert!(
        runtime.c.starts_with(codegen_riscv::RISCV_RUNTIME_C) && runtime.c.len() > codegen_riscv::RISCV_RUNTIME_C.len(),
        "the shim of a program the extension reaches is the shim and the extension's adapter after it"
    );
    let archive = value_runtime::archive_for(Target::Riscv64)
        .unwrap_or_else(|why| panic!("the RISC-V value runtime did not build: {why}"));
    let bytes = std::fs::read(&archive.path).unwrap_or_else(|e| panic!("{}: {e}", archive.path.display()));
    let elf = bytes
        .windows(4)
        .position(|w| w == b"\x7fELF")
        .unwrap_or_else(|| panic!("{} holds no object", archive.path.display()));
    let machine = u16::from_le_bytes([bytes[elf + 18], bytes[elf + 19]]);
    assert_eq!(machine, 243, "the archive's objects are not RISC-V (EM_RISCV = 243)");
    assert!(
        !archive.system_libs.iter().any(|l| l == "-lgcc_s"),
        "a static link has no shared unwinder to name: {:?}",
        archive.system_libs
    );
    let mut expected = vec![archive.path.display().to_string()];
    expected.extend(archive.system_libs.iter().cloned());
    expected.push("-lm".to_string());
    assert_eq!(runtime.link_args, expected);
    let host = value_runtime::archive_for(Target::Host).unwrap_or_else(|why| panic!("{why}"));
    assert_ne!(host.path, archive.path, "the host's archive and RISC-V's are one file");
}

fn run_riscv(source: &str, env: &[(&str, String)]) -> riscv_runner::RvRun {
    let asm = compile::compile_to_riscv(source).unwrap_or_else(|e| panic!("{e:?}"));
    let linked = riscv_runner::link(&asm, "exact");
    riscv_runner::run(&linked, &[], env)
}

/// Every value a loop lets go of is dropped: the most handles alive at once stays a small fraction of the handles
/// made, as the heap report counts them. The loop makes ordinals, whose arithmetic qemu runs fast enough to make the
/// many handles the collector's first collection waits for several times over.
#[test]
fn the_values_a_loop_lets_go_of_are_freed() {
    if !common::require_tool("qemu-riscv64") || !common::require_tool("riscv64-linux-gnu-gcc") {
        return;
    }
    let made = 600_000;
    let source = format!(
        "## Main\nLet w: Ordinal be omega.\nLet mutable x: Ordinal be w.\nLet mutable n be 0.\nRepeat for i from 1 to {made}:\n    Set x to w + i.\n    If x > i:\n        Set n to n + 1.\nShow n.\nShow x.\n"
    );
    let expected = common::run_all_tiers(&source);
    assert_eq!(expected.error, None);
    let run = run_riscv(&source, &[("LOGOS_HEAP_REPORT", "1".to_string())]);
    assert!(run.success, "{}", run.describe());
    assert_eq!(run.stdout.trim_end(), expected.output.trim_end());
    let report = run
        .stderr
        .lines()
        .find_map(|l| l.strip_prefix("logos heap: "))
        .unwrap_or_else(|| panic!("no heap report on stderr:\n{}", run.stderr));
    let field = |name: &str| -> u64 {
        report
            .split_whitespace()
            .find_map(|kv| kv.strip_prefix(&format!("{name}=")))
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("no `{name}` in `{report}`"))
    };
    let (values, values_high) = (field("values"), field("values_high"));
    assert!(
        values_high >= 1 && values_high < made / 4,
        "{values_high} exact values were alive at once of the {made}+ the loop made"
    );
    assert!(
        values <= values_high,
        "{values} values alive at the end, more than the {values_high} ever alive at once"
    );
}

/// A handle reached only from the heap — an item of a list held in a field — survives every collection: with a
/// collection at each allocation, the program prints what the VM prints.
#[test]
fn a_handle_reached_through_the_heap_survives_every_collection() {
    if !common::require_tool("qemu-riscv64") || !common::require_tool("riscv64-linux-gnu-gcc") {
        return;
    }
    let source = "## A Box has:\n    a value, which is Hyperreal.\n\n## Main\nLet dt be an infinitesimal.\nLet mutable boxes be a new Seq of Box.\nRepeat for i from 1 to 50:\n    Push a new Box with value dt * i to boxes.\nLet mutable total: Hyperreal be 0.\nRepeat for b in boxes:\n    Set total to total + b's value.\nShow total.\nLet last be item 50 of boxes.\nShow last's value.\n";
    let expected = common::run_all_tiers(source);
    assert_eq!(expected.error, None);
    let run = run_riscv(source, &[("LOGOS_GC_STRESS", "1".to_string())]);
    assert!(run.success, "{}", run.describe());
    assert_eq!(run.stdout.trim_end(), expected.output.trim_end());
}

/// A real number the program defines by a function, read after the allocations of a loop: the runtime alone reaches
/// the function's closure. With a collection at each allocation the program prints what the VM prints, and the table
/// of the shapes its closures answer by is emitted for it and for no program that defines no real.
#[test]
fn a_function_a_real_number_is_defined_by_survives_every_collection() {
    let source = "## To seventh (i: Int) -> Int:\n    If i = 0:\n        Return 0.\n    Return item ((i - 1) % 6 + 1) of [1, 4, 2, 8, 5, 7].\n\n## Main\nLet r be the real number whose digits in base 10 are seventh.\nLet q: Rational be 1 / 8.\nLet mutable rows be a new Seq of Seq of Int.\nRepeat for i from 1 to 300:\n    Push [i, i * 2, i * 3] to rows.\nShow r > q.\nRepeat for i from 1 to 300:\n    Push [i] to rows.\nShow floor(r * 1000000000000 * 1000000000000).\nShow r.\nShow length of rows.\n";
    let asm = compile::compile_to_riscv(source).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(asm.contains(".Ldefined_answers"), "a program that defines a real has no table of answers");
    let plain = compile::compile_to_riscv("## Main\nLet dt be an infinitesimal.\nShow dt * 2.\n").expect("emits");
    assert!(!plain.contains(".Ldefined_answers"), "a program that defines no real emits the table of answers");
    if !common::require_tool("qemu-riscv64") || !common::require_tool("riscv64-linux-gnu-gcc") {
        return;
    }
    let expected = common::run_all_tiers(source);
    assert_eq!(expected.error, None);
    let run = run_riscv(source, &[("LOGOS_GC_STRESS", "1".to_string())]);
    assert!(run.success, "{}", run.describe());
    assert_eq!(run.stdout.trim_end(), expected.output.trim_end());
}

/// A cell crossing to a task holds a real number a function of the program defines: the program stops in the VM's
/// words, the task never started, as on every engine.
#[test]
fn a_defined_real_handed_to_a_task_stops_the_program_in_the_vm_s_words() {
    if !common::require_tool("qemu-riscv64") || !common::require_tool("riscv64-linux-gnu-gcc") {
        return;
    }
    let source = "## To seventh (i: Int) -> Int:\n    Return i % 7.\n\n## To look (x: ExactReal):\n    Show x.\n\n## Main\nLet r be the real number whose digits in base 10 are seventh.\nShow \"before\".\nLaunch a task to look with r.\nShow \"after\".\n";
    let expected = compile::vm_outcome(source);
    let why = expected.error.expect("the VM refuses the task");
    assert!(why.contains("NotSendable"), "{why}");
    let run = run_riscv(source, &[]);
    assert!(!run.success, "the program ran a task holding a defined real:\n{}", run.describe());
    assert_eq!(run.stdout.trim_end(), expected.output.trim_end());
    assert!(run.stderr.contains(&why), "{}", run.describe());
}
