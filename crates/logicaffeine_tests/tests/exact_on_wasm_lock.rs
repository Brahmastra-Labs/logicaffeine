//! ★ LOCK — the exact extension runs on linked WebAssembly as on the VM. ★
//!
//! A value the exact extension reaches is held by a linked module as a handle into base's own Rust, the same
//! `logos_rt_value_*` ABI the C backends link, built for `wasm32` and linked into the module by `rust-lld`, so
//! every such value computes, prints, compares and fails as the VM's does. A self-contained module links no
//! runtime and cannot hold one: it refuses a program the extension reaches by name and points at the linked
//! target. A program the extension never reaches imports nothing of the value runtime.

#![cfg(not(target_arch = "wasm32"))]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

use logicaffeine_compile::compile;
use tier_parity::{exact, Backend, Program};

/// The child side of every isolated backend run. Does nothing unless `LOGOS_PARITY_JOB` is set.
#[test]
#[ignore = "the worker the parity sweep re-executes this binary into"]
fn tier_parity_worker() {
    tier_parity::worker_main();
}

fn differ(programs: &[Program]) -> Vec<String> {
    let rows = tier_parity::sweep(programs, &[Backend::TreeWalker, Backend::WasmLinked], 4)
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
fn every_extension_probe_runs_on_linked_wasm_as_on_the_vm() {
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
fn every_boundary_of_a_linked_module_holds_an_exact_value_as_the_vm_does() {
    let bad = differ(&exact::programs());
    assert!(
        bad.is_empty(),
        "{} (program, backend) runs differ from the VM:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// A self-contained module has no runtime to hold an exact value in: it refuses the program by name, naming the
/// target that runs it, instead of an internal refusal about some register.
#[test]
fn a_self_contained_module_refuses_an_exact_program_by_name() {
    for source in [
        "## Main\nLet dt be an infinitesimal.\nShow dt * 2.\n",
        "## Main\nLet x: Algebraic be sqrt(2).\nShow x * x.\n",
        "## To half (x: ExactReal) -> ExactReal:\n    Return x / 2.\n\n## Main\nShow half(3).\n",
    ] {
        let refused = match compile::compile_to_wasm(source) {
            Ok(_) => panic!("a self-contained module accepted a program the exact extension reaches:\n{source}"),
            Err(e) => format!("{e:?}"),
        };
        assert!(
            refused.contains("does not run the exact extension") && refused.contains("wasm-linked"),
            "the self-contained module must name what it refuses and the target that runs it: {refused}"
        );
    }
}

/// The value runtime is imported by what reaches it: a module whose program never names the extension imports
/// none of its ABI, and one whose program does imports it.
#[cfg(feature = "wasm-jit")]
#[test]
fn only_a_program_the_extension_reaches_imports_the_value_runtime() {
    let names_the_abi = |object: &[u8]| object.windows(b"logos_rt_value_".len()).any(|w| w == b"logos_rt_value_");
    let plain = compile::compile_to_wasm_relocatable(exact::PLAIN).unwrap_or_else(|e| panic!("{e:?}"));
    assert!(
        !names_the_abi(&plain),
        "a module whose program never names the extension imports the value runtime"
    );
    let reached = compile::compile_to_wasm_relocatable("## Main\nLet dt be an infinitesimal.\nShow dt * 2.\n")
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(
        names_the_abi(&reached),
        "a module whose program the extension reaches does not import the value runtime"
    );
}

/// The runtime calls a program's function back through the reader the module defines under one name: an object whose
/// program defines a real number by a function names it, and an object whose program defines none does not, nor
/// imports the runtime's definition.
#[cfg(feature = "wasm-jit")]
#[test]
fn only_a_program_that_defines_a_real_names_the_reader() {
    let has = |object: &[u8], text: &str| object.windows(text.len()).any(|w| w == text.as_bytes());
    let defines = compile::compile_to_wasm_relocatable(
        "## To seventh (i: Int) -> Int:\n    Return i % 7.\n\n## Main\nLet r be the real number whose digits in base 10 are seventh.\nShow r > 0.\n",
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(has(&defines, "logos_program_defines"), "the reader is not named in the object");
    assert!(has(&defines, "logos_rt_value_define"), "the definition is not the runtime's");
    let plain = compile::compile_to_wasm_relocatable("## Main\nLet dt be an infinitesimal.\nShow dt * 2.\n")
        .unwrap_or_else(|e| panic!("{e:?}"));
    assert!(!has(&plain, "logos_program_defines"), "a program that defines no real names the reader");
    assert!(!has(&plain, "logos_rt_value_define"), "a program that defines no real imports the definition");
}
