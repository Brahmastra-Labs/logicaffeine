//! Stage attribution is part of the optimization-parity contract: shared AST
//! transforms must agree between the run and VM/JIT paths, while bytecode
//! selection and its oracle facts belong to backend lowering.

use logicaffeine_compile::compile::{
    compile_program_backend_traced, compile_to_c_traced, compile_to_c_via_bytecode_traced,
    optimization_trace_run, optimization_trace_vm,
};
use logicaffeine_compile::optimization::{BackendOptimizationTrace, Opt, REGISTRY};
use std::fs;
use std::path::Path;

// Hash-map reads are the smallest current benchmark shape where the shared
// optimizer leaves the program unchanged but the VM oracle proves an element
// type for bytecode selection. Keep the source here so the trace contract does
// not depend on benchmark-corpus calibration edits.
const COLLECT: &str = r#"
## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let m be a new Map of Int to Int.
Let seed be 42.
Let i be 0.
While i is less than n:
    Set seed to (seed * 1103515245 + 12345) % 2147483648.
    Set m at seed to seed * 2.
    Set i to i + 1.
Let hit be 42.
Let found be 0.
Set i to 0.
While i is less than n:
    Set hit to (hit * 1103515245 + 12345) % 2147483648.
    If m contains hit:
        Set found to found + item hit of m.
    Set i to i + 1.
Show found.
"#;

const SHA256: &str = include_str!("../../../benchmarks/programs/sha256/main.lg");

fn explained_missing(trace: &BackendOptimizationTrace, opt: Opt) -> Option<String> {
    if trace.all_fired().fired(opt) {
        return Some("applied".to_string());
    }
    if let Some(decline) = trace.declines.iter().find(|decline| decline.opt == opt) {
        return Some(format!("declined: {}", decline.reason));
    }
    REGISTRY
        .iter()
        .find(|candidate| {
            trace.all_fired().fired(candidate.opt) && candidate.preempts.contains(&opt)
        })
        .map(|candidate| format!("preempted by {}", candidate.keyword))
}

#[test]
fn vm_oracle_facts_are_lowering_not_shared_optimizer_transforms() {
    let run = optimization_trace_run(COLLECT);
    let vm = optimization_trace_vm(COLLECT);

    assert_eq!(
        vm.optimizer.keywords(),
        run.optimizer.keywords(),
        "VM/JIT silently diverged from the shared run-path optimizer"
    );
    assert!(
        !run.optimizer.fired(Opt::ElemType),
        "collect does not transform through elemtype on the shared run path"
    );
    assert!(
        vm.lowering.fired(Opt::ElemType),
        "VM oracle elemtype evidence must remain visible as backend lowering"
    );
}

#[test]
fn sha_loop_specialization_is_available_to_run_and_vm() {
    let run = optimization_trace_run(SHA256);
    let vm = optimization_trace_vm(SHA256);

    assert!(
        run.optimizer.fired(Opt::SpecializeLoop),
        "SHA-256 loses range-aware loop specialization on run/JIT: {:?}",
        run.optimizer.keywords()
    );
    assert_eq!(
        vm.optimizer.keywords(),
        run.optimizer.keywords(),
        "VM/JIT silently diverged from the shared run-path optimizer"
    );
}

#[test]
fn every_benchmark_keeps_shared_optimizer_parity_across_backends() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/programs");
    let mut programs: Vec<_> = fs::read_dir(root)
        .expect("benchmark program directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("main.lg"))
        .filter(|path| path.is_file())
        .collect();
    programs.sort();
    assert_eq!(programs.len(), 35, "benchmark trace census drifted");
    let mut rust_tile_witnesses = 0usize;
    let mut rust_strength_witnesses = 0usize;
    let mut vm_unchecked_witnesses = 0usize;

    for path in programs {
        let name = path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .expect("benchmark directory name");
        let source = fs::read_to_string(&path).expect("benchmark source");

        let (_, rust) = compile_program_backend_traced(&source)
            .unwrap_or_else(|error| panic!("{name}: Rust trace failed: {error:?}"));
        let (_, c) = compile_to_c_traced(&source)
            .unwrap_or_else(|error| panic!("{name}: direct-C trace failed: {error:?}"));
        let (_, c_bytecode) = compile_to_c_via_bytecode_traced(&source)
            .unwrap_or_else(|error| panic!("{name}: bytecode-C trace failed: {error:?}"));
        let run = optimization_trace_run(&source);
        let vm = optimization_trace_vm(&source);

        assert!(
            c.fallbacks.is_empty(),
            "{name}: direct C reached a different backend instead of lowering the benchmark: {:?}",
            c.fallbacks
        );
        assert!(
            c_bytecode.fallbacks.is_empty(),
            "{name}: the bytecode-C label silently fell back to another C backend: {:?}",
            c_bytecode.fallbacks
        );
        for decline in c
            .declines
            .iter()
            .chain(c_bytecode.declines.iter())
            .chain(run.declines.iter())
            .chain(vm.declines.iter())
        {
            assert!(
                !decline.reason.trim().is_empty(),
                "{name}: {:?} declined {:?} without an auditable reason",
                decline.backend,
                decline.opt
            );
        }

        let rust_optimizer = rust.optimizer.keywords();
        let c_optimizer = c.optimizer.keywords();
        let c_bytecode_optimizer = c_bytecode.optimizer.keywords();
        let missing_from_c: Vec<_> = rust_optimizer
            .iter()
            .copied()
            .filter(|opt| !c_optimizer.contains(opt))
            .collect();
        let missing_from_c_bytecode: Vec<_> = rust_optimizer
            .iter()
            .copied()
            .filter(|opt| !c_bytecode_optimizer.contains(opt))
            .collect();
        assert!(
            missing_from_c.is_empty(),
            "{name}: direct C silently lost AOT optimizer passes {missing_from_c:?}; \
             Rust={rust_optimizer:?} C={c_optimizer:?}"
        );
        assert!(
            missing_from_c_bytecode.is_empty(),
            "{name}: bytecode C silently lost AOT optimizer passes {missing_from_c_bytecode:?}; \
             Rust={rust_optimizer:?} C-bytecode={c_bytecode_optimizer:?}"
        );
        assert_eq!(
            vm.optimizer.keywords(),
            run.optimizer.keywords(),
            "{name}: VM/JIT silently diverged from the run shared optimizer"
        );

        // These optimizations leave proof-bearing shapes/opcodes for a backend
        // to consume.  Seeing the optimizer bit is not enough: the target must
        // report that it actually emitted the optimized form.  Keep the check
        // over the whole benchmark corpus so a new program cannot reintroduce
        // the old "proof reached the tier, lowering silently discarded it"
        // failure mode.
        if rust.lowering.fired(Opt::Tile) {
            rust_tile_witnesses += 1;
            assert!(
                c.lowering.fired(Opt::Tile),
                "{name}: direct C silently discarded Rust's TileLegal lowering; \
                 Rust lowering={:?} C lowering={:?}",
                rust.lowering.keywords(),
                c.lowering.keywords()
            );
        }
        if rust.lowering.fired(Opt::StrengthReducePow2) {
            rust_strength_witnesses += 1;
            assert!(
                explained_missing(&c, Opt::StrengthReducePow2).is_some(),
                "{name}: direct C silently discarded Rust's power-of-two reduction; \
                 Rust lowering={:?} C lowering={:?} declines={:?}",
                rust.lowering.keywords(),
                c.lowering.keywords(),
                c.declines
            );
        }
        if vm.lowering.fired(Opt::Unchecked) {
            vm_unchecked_witnesses += 1;
            assert!(
                explained_missing(&c_bytecode, Opt::Unchecked).is_some(),
                "{name}: bytecode C silently lost VM unchecked-op selection without applying it, \
                 declining it, or firing a registered preemptor; VM lowering={:?} \
                 C-bytecode lowering={:?} declines={:?}",
                vm.lowering.keywords(),
                c_bytecode.lowering.keywords(),
                c_bytecode.declines
            );
        }
    }

    assert!(
        rust_tile_witnesses > 0,
        "the corpus no longer witnesses Rust Tile lowering; the parity ratchet would be vacuous"
    );
    assert!(
        rust_strength_witnesses > 0,
        "the corpus no longer witnesses Rust power-of-two lowering; the parity ratchet would be vacuous"
    );
    assert!(
        vm_unchecked_witnesses > 0,
        "the corpus no longer witnesses VM unchecked-op selection; the parity ratchet would be vacuous"
    );
}
