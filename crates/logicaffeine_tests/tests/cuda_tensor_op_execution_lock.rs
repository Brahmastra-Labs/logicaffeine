//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! EVERY TENSOR OP RUNS ON THE GPU AND PRINTS WHAT THE VM PRINTS.
//!
//! `tensor_op_coverage_lock` scores its CUDA column as covered when a lowering returns `Ok` — a program was
//! PRINTED. This lock runs it: every probe of the backend matrix is lowered by the general CUDA tier (the C tier's
//! proven body lifted onto the device), compiled by `nvcc` for this box's GPU, executed, and its stdout compared
//! token for token with the VM's. The probes print `round(y · 10⁶)`, so the comparison is exact.
//!
//! `NOT_YET_EQUAL` is shrink-only: a probe listed there must still disagree with the VM (or fail to build), and a
//! probe that starts agreeing must leave the list. The GPU legs need `nvcc` and a device;
//! `LOGOS_REQUIRE_TOOLS=nvcc,gpu` makes their absence a failure.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;

use std::process::Command;

use logicaffeine_compile::compile::{compile_to_cuda, vm_outcome};

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

/// Probes whose GPU run does not yet print the VM's output, with the reason. Shrink-only.
const NOT_YET_EQUAL: &[(&str, &str)] = &[];

fn words(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

/// The probe's stdout on the GPU, or why there is none.
fn gpu_stdout(op: &str, program: &str) -> Result<Vec<String>, String> {
    let artifact = compile_to_cuda(program).map_err(|e| format!("does not lower: {e:?}"))?;
    let dir = std::env::temp_dir().join(format!("logos_topexec_{op}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let (src, bin) = (dir.join("k.cu"), dir.join("k"));
    std::fs::write(&src, &artifact.kernel_cu).map_err(|e| e.to_string())?;
    let built = Command::new("nvcc")
        .args(["-O2", "-arch=sm_86"])
        .arg(&src)
        .arg("-o")
        .arg(&bin)
        .output()
        .map_err(|e| e.to_string())?;
    let result = if !built.status.success() {
        Err(format!(
            "nvcc rejects the emitted program:\n{}",
            String::from_utf8_lossy(&built.stderr)
        ))
    } else {
        let run = Command::new(&bin).output().map_err(|e| e.to_string())?;
        if run.status.success() {
            Ok(words(&String::from_utf8_lossy(&run.stdout)))
        } else {
            Err(format!(
                "the program fails on the device:\n{}",
                String::from_utf8_lossy(&run.stderr)
            ))
        }
    };
    std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    result
}

#[test]
fn every_probe_prints_the_vms_output_on_the_gpu() {
    if !(common::require_tool("nvcc") && common::require_gpu()) {
        return;
    }
    let mut disagreements = Vec::new();
    let mut stale = Vec::new();
    for p in tensor_probes::BACKEND_MATRIX_PROBES {
        let vm = vm_outcome(p.program);
        assert!(vm.error.is_none(), "{}: VM error {:?}", p.op, vm.error);
        let want = words(&vm.output);
        assert_eq!(
            want.len(),
            p.shows,
            "{}: the VM prints one value per Show — the probe is broken",
            p.op
        );
        let verdict = match gpu_stdout(p.op, p.program) {
            Ok(got) if got == want => Ok(()),
            Ok(got) => Err(format!("prints {got:?}, the VM prints {want:?}")),
            Err(why) => Err(why),
        };
        let listed = NOT_YET_EQUAL.iter().any(|(op, _)| *op == p.op);
        match (verdict, listed) {
            (Err(why), false) => disagreements.push(format!("{}: {why}", p.op)),
            (Ok(()), true) => stale.push(p.op),
            _ => {}
        }
    }
    assert!(
        disagreements.is_empty(),
        "tensor ops whose GPU run disagrees with the VM:\n{}",
        disagreements.join("\n")
    );
    assert!(
        stale.is_empty(),
        "these probes now print the VM's output on the GPU — remove them from NOT_YET_EQUAL: {stale:?}"
    );
}

/// Every listed gap names a probe of the matrix, so the list cannot carry dead rows.
#[test]
fn every_listed_gap_is_a_probe_of_the_matrix() {
    for (op, why) in NOT_YET_EQUAL {
        assert!(
            tensor_probes::BACKEND_MATRIX_PROBES.iter().any(|p| p.op == *op),
            "`{op}` is no probe of the matrix"
        );
        assert!(!why.is_empty(), "`{op}` carries no reason");
    }
}
