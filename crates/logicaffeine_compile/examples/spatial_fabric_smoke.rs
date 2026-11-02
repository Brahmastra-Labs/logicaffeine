//! Scratch: emit the fabric RTL of a small GEMM and print the model's cycle count (removed once the lock lands).
use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
use logicaffeine_compile::spatial::ir::{NumericsPolicy, TensorId};
use logicaffeine_compile::spatial::lower::fabric::{
    fabric_cycle_model, input_hex_files, FabricBackend,
};
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::passes::from_tensor_op::MatmulNames;
use logicaffeine_compile::spatial::pipeline::{run_passes, template_from_spec};
use logicaffeine_compile::spatial::target::{
    HandshakeKind, Interconnect, MeshShape, SpatialTarget, TileShape,
};
use logicaffeine_compile::tensor_recognize::{Fill, MatmulSpec};

fn prng(seed: u64, i: usize, lo: i64, hi: i64) -> i64 {
    let mut x = seed
        .wrapping_mul(0x9E3779B97F4A7C15)
        .wrapping_add(i as u64 + 1);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58476D1CE4E5B9);
    x ^= x >> 27;
    lo + ((x % ((hi - lo + 1) as u64)) as i64)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = std::path::PathBuf::from(&args[0]);
    let mcast = args.get(1).map(|s| s == "mcast").unwrap_or(false);
    let (m, n, k): (usize, usize, usize) = (
        args.get(2).and_then(|s| s.parse().ok()).unwrap_or(8),
        args.get(3).and_then(|s| s.parse().ok()).unwrap_or(8),
        args.get(4).and_then(|s| s.parse().ok()).unwrap_or(8),
    );
    let mut t = SpatialTarget::sky_fabric(16, 16);
    t.sites = MeshShape { x: 2, y: 2 };
    t.tile = TileShape { rows: 4, cols: 4 };
    t.site.acc_tiles = 4;
    if mcast {
        t.interconnect.insert(Interconnect::MulticastRect);
        t.handshakes.insert(HandshakeKind::Semaphore);
        // A small local memory forces a per-site block the whole output does not fit into, so the plan is a
        // real 2-D grid and `fuse_readers` has row and column groups to make.
        t.site.local_mem_bytes = args
            .get(5)
            .and_then(|s| s.parse().ok())
            .unwrap_or(t.site.local_mem_bytes);
        t.cycle.c_dram_shared = 400.0;
        t.cycle.c_mcast_block = 0.0;
        t.cycle.c_noc_tile = 0.0;
        t.cycle.c_dram_tile = 1.0;
    }
    let names = MatmulNames {
        a: "a".into(),
        b: "b".into(),
        c: "c".into(),
    };
    let spec = MatmulSpec {
        m,
        n,
        k,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    let p = template_from_spec(
        &spec,
        &names,
        (&Fill::Unknown, &Fill::Unknown),
        (Some((-128, 127)), Some((-128, 127))),
        NumericsPolicy::Exact,
        &t,
    )
    .unwrap();
    let p = run_passes(p, &t).unwrap();
    println!(
        "groups {} sites {} plan {:?}",
        p.placement.as_ref().unwrap().groups.len(),
        p.sites().len(),
        p.work.plan
    );
    let acts: Vec<i64> = (0..m * k).map(|i| prng(11, i, -128, 127)).collect();
    let weights: Vec<i64> = (0..k * n).map(|i| prng(12, i, -128, 127)).collect();
    let mut inputs = Inputs::default();
    inputs.tensors.insert(TensorId(0), Value::Int(acts));
    inputs.tensors.insert(TensorId(1), Value::Int(weights));
    let out = eval_choreography(&p, &inputs).unwrap();
    let expected = match &out.tensors[&TensorId(2)] {
        Value::Int(v) => v.clone(),
        Value::Float(v) => v.iter().map(|x| *x as i64).collect(),
    };
    let art = match FabricBackend::new(t.clone()).lower(&p).unwrap() {
        SpatialArtifact::Verilog(v) => v,
        other => panic!("{other:?}"),
    };
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("spatial_top.v"), &art.rtl).unwrap();
    std::fs::write(dir.join("tb.v"), &art.testbench).unwrap();
    for (name, body) in art.hex.iter().cloned().chain(input_hex_files(&p, &inputs)) {
        std::fs::write(dir.join(&name), body).unwrap();
    }
    std::fs::write(
        dir.join("expected.txt"),
        expected
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    println!(
        "model {} floor {} (twin {})",
        art.cycle_model,
        art.cycle_floor,
        fabric_cycle_model(&p, &t)
    );
}
