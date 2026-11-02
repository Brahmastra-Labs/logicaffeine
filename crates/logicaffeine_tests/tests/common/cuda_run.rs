//! Running a GPU program on the RTX 3060: compile it with nvcc for sm_86, run it, and read every
//! `OUT <tensor> <element> <value>` it prints, by tensor — or hold every output and state tensor it prints to the
//! evaluator outright.

#![allow(dead_code)]

use std::collections::BTreeMap;

use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
use logicaffeine_compile::spatial::ir::{SpatialProgram, TensorRole};

/// Compile `cu` for sm_86, run it, and read every `OUT <tensor> <element> <value>` it prints, by tensor.
pub fn run(label: &str, cu: &str) -> BTreeMap<usize, Vec<i64>> {
    let dir = std::env::temp_dir().join(format!("cuda_run_{}_{}", std::process::id(), label.replace([' ', '/', '×'], "_")));
    std::fs::create_dir_all(&dir).unwrap();
    let (src, bin) = (dir.join("program.cu"), dir.join("program"));
    std::fs::write(&src, cu).unwrap();
    let build = std::process::Command::new("nvcc").args(["-O3", "-arch=sm_86", "-std=c++17", "-o"]).arg(&bin).arg(&src).output().expect("nvcc");
    assert!(build.status.success(), "{label}: nvcc failed:\n{}", String::from_utf8_lossy(&build.stderr));
    let out = std::process::Command::new(&bin).current_dir(&dir).output().expect("run");
    assert!(out.status.success(), "{label}: the program failed: {}", String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&dir);
    let mut got: BTreeMap<usize, Vec<(usize, i64)>> = BTreeMap::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.first() == Some(&"OUT") {
            got.entry(t[1].parse().unwrap()).or_default().push((t[2].parse().unwrap(), t[3].parse().unwrap()));
        }
    }
    got.into_iter()
        .map(|(id, mut v)| {
            v.sort();
            (id, v.into_iter().map(|(_, x)| x).collect())
        })
        .collect()
}

/// `cu`, `p`'s GPU program, run from `inputs`: every output and state tensor it prints is the evaluator's.
pub fn runs_as_the_evaluator(label: &str, p: &SpatialProgram, cu: &str, inputs: &Inputs) {
    let want = eval_choreography(p, inputs).unwrap_or_else(|e| panic!("{label}: the evaluator: {e:?}"));
    let got = run(label, cu);
    for t in p.tensors.iter().filter(|t| matches!(t.role, TensorRole::Output | TensorRole::State)) {
        let Some(Value::Int(w)) = want.tensors.get(&t.id) else { continue };
        let g = got.get(&t.id.idx()).unwrap_or_else(|| panic!("{label}: the GPU prints {}", t.name));
        assert_eq!(g, w, "{label}: {} on the GPU is the evaluator's", t.name);
    }
}
