//! Scratch: emit the persistent CUDA artifact and the reference outputs (removed once the lock lands).
use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
use logicaffeine_compile::spatial::ir::{NumericsPolicy, TensorId};
use logicaffeine_compile::spatial::lower::cuda::CudaPersistentBackend;
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::passes::from_tensor_op::MatmulNames;
use logicaffeine_compile::spatial::pipeline::{run_passes, template_from_spec};
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::tensor_recognize::{Fill, MatmulSpec};

fn prng(seed: u64, i: usize, lo: i64, hi: i64) -> i64 {
    let mut x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(i as u64 + 1);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    lo + (x % (hi - lo + 1) as u64) as i64
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let dir = std::path::PathBuf::from(&a[0]);
    let (m, n, k): (usize, usize, usize) = (
        a[1].parse().unwrap(),
        a[2].parse().unwrap(),
        a[3].parse().unwrap(),
    );
    let gpu = SpatialTarget::rtx3060();
    let spec = MatmulSpec {
        m,
        n,
        k,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    let names = MatmulNames {
        a: "a".into(),
        b: "b".into(),
        c: "c".into(),
    };
    let t = template_from_spec(
        &spec,
        &names,
        (&Fill::Unknown, &Fill::Unknown),
        (Some((-128, 127)), Some((-128, 127))),
        NumericsPolicy::Exact,
        &gpu,
    )
    .unwrap();
    let p = run_passes(t, &gpu).unwrap();
    println!("plan {:?} sites {}", p.work.plan, p.sites().len());
    let acts: Vec<i64> = (0..m * k).map(|i| prng(11, i, -128, 127)).collect();
    let weights: Vec<i64> = (0..k * n).map(|i| prng(12, i, -128, 127)).collect();
    let mut inputs = Inputs::default();
    inputs.tensors.insert(TensorId(0), Value::Int(acts.clone()));
    inputs
        .tensors
        .insert(TensorId(1), Value::Int(weights.clone()));
    let out = eval_choreography(&p, &inputs).unwrap();
    let expected = match &out.tensors[&TensorId(2)] {
        Value::Int(v) => v.clone(),
        Value::Float(v) => v.iter().map(|x| *x as i64).collect(),
    };
    let art = match CudaPersistentBackend::new(gpu).lower(&p).unwrap() {
        SpatialArtifact::Cuda(c) => c,
        o => panic!("{o:?}"),
    };
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("persistent.cu"), &art.program_cu).unwrap();
    std::fs::write(dir.join("bench.cu"), &art.bench_cu).unwrap();
    std::fs::write(
        dir.join("a.txt"),
        acts.iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    std::fs::write(
        dir.join("b.txt"),
        weights
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    std::fs::write(
        dir.join("expected.txt"),
        expected
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    println!("budget {:?}", art.budget);
}
