//! sass_depgraph_lock — the register-dependency-graph engine, validated on our real GEMM machine code. This is
//! the core of the SASS scheduler: for every instruction it computes the RAW/WAR/WAW hazards + barrier ordering
//! that a legal reschedule MUST respect. We prove it is (1) a sound DAG (all edges point backward, a
//! topological order exists), (2) captures the real hazards (the IMMA accumulator chains + the ldmatrix->IMMA
//! data flow), and (3) the original ptxas order is itself a valid topological order (a sanity anchor).
#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_cuda::gemm_tile::emit_mma_tn_k64_kernel;
use logicaffeine_compile::codegen_sass::{dependency_graph_from_cubin, SassClass};
use std::process::Command;

fn have() -> bool {
    Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        && Command::new("nvdisasm")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

#[test]
fn the_dependency_graph_is_a_sound_dag_over_our_gemm_sass() {
    if !have() {
        eprintln!("SKIP: needs nvcc + nvdisasm");
        return;
    }
    let dir = std::env::temp_dir().join(format!("logos_dep_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let src = dir.join("k.cu");
    let cubin = dir.join("k.cubin");
    std::fs::write(&src, emit_mma_tn_k64_kernel(128, 128, 2, 2, 3)).expect("write");
    let o = Command::new("nvcc")
        .args(["-O3", "-arch=sm_86", "-cubin", "-o"])
        .arg(&cubin)
        .arg(&src)
        .output()
        .expect("nvcc");
    assert!(
        o.status.success(),
        "cubin: {}",
        String::from_utf8_lossy(&o.stderr)
    );

    let (insns, classes, g) = dependency_graph_from_cubin(&cubin).expect("build dep graph");
    let n = insns.len();
    assert_eq!(g.deps.len(), n);
    eprintln!(
        "dependency graph over {n} SASS instructions; {} IMMA, {} edges",
        classes.iter().filter(|&&c| c == SassClass::Tensor).count(),
        g.deps.iter().map(|d| d.len()).sum::<usize>()
    );

    // (1) SOUND DAG: every dependency edge points to an EARLIER instruction (so the original order is a valid
    //     topological sort, and cycles are impossible by construction).
    for (i, ds) in g.deps.iter().enumerate() {
        for &j in ds {
            assert!(j < i, "edge {j}->{i} must point backward (a forward edge would be a scheduling bug/cycle)");
        }
    }

    // (2) The graph must capture REAL hazards, not be empty: the IMMA accumulator (read-modify-write of the
    //     same C registers) forms long dependency chains. At least some IMMA must depend on a prior IMMA.
    let imma_idx: Vec<usize> = (0..n)
        .filter(|&i| classes[i] == SassClass::Tensor)
        .collect();
    assert!(imma_idx.len() >= 8, "must see the tensor-core MACs");
    let imma_with_imma_dep = imma_idx
        .iter()
        .filter(|&&i| g.deps[i].iter().any(|&j| classes[j] == SassClass::Tensor))
        .count();
    assert!(
        imma_with_imma_dep >= 8,
        "IMMA accumulator chains must create IMMA->IMMA hazards (got {imma_with_imma_dep})"
    );

    // (3) an IMMA must depend on a prior LDSM (ldmatrix): the fragment it multiplies. This is the data flow the
    //     scheduler will reorder to hide the ldmatrix latency behind earlier IMMAs.
    let imma_reads_ldsm = imma_idx
        .iter()
        .any(|&i| g.deps[i].iter().any(|&j| classes[j] == SassClass::Memory));
    assert!(
        imma_reads_ldsm,
        "an IMMA must depend on an ldmatrix load (the fragment data flow)"
    );

    // (4) a valid topological schedule exists (Kahn's algorithm terminates over all n).
    let mut indeg = vec![0usize; n];
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, ds) in g.deps.iter().enumerate() {
        indeg[i] = ds.len();
        for &j in ds {
            succ[j].push(i);
        }
    }
    let mut ready: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    let mut scheduled = 0usize;
    while let Some(v) = ready.pop() {
        scheduled += 1;
        for &s in &succ[v] {
            indeg[s] -= 1;
            if indeg[s] == 0 {
                ready.push(s);
            }
        }
    }
    assert_eq!(
        scheduled, n,
        "the dependency graph must be schedulable (acyclic) — {scheduled}/{n} placed"
    );
    eprintln!("SOUND: DAG acyclic, {n} instructions topologically schedulable, hazards captured. Scheduler foundation ready.");
    let _ = std::fs::remove_dir_all(&dir);
}
