//! ★ THE PERSISTENT KERNEL'S RINGS — a FIFO is a shared-memory ring of `cp.async` commit groups, with no
//! run-time flow control, so what keeps it correct is proved by the printer and pinned here:
//!   A. every Wait publishes its page to the whole block: the waits on the committed groups are followed by a
//!      block barrier before anything else runs (a thread's wait completes only its own copies);
//!   B. a rotated K loop waits for all but the block it prefetched: the wait count is exactly the commits the
//!      prefetch issues, and the last block, which prefetches nothing, commits as many empty groups;
//!   C. a ring too shallow to hold the block being read and the block being prefetched prints without the
//!      prefetch, and still computes `eval_choreography`'s answer on the GPU;
//!   D. a producer that fills a ring faster than its consumer drains it is refused, typed — never printed.

use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
use logicaffeine_compile::spatial::ir::{
    BufferKind, ChoreoOp, NumericsPolicy, SpatialProgram, TaggedOp, TensorId,
};
use logicaffeine_compile::spatial::lower::cuda::{CudaArtifact, CudaPersistentBackend};
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend, SpatialLowerError};
use logicaffeine_compile::spatial::passes::from_tensor_op::MatmulNames;
use logicaffeine_compile::spatial::pipeline::{run_passes, template_from_spec};
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::tensor_recognize::{Fill, MatmulSpec};

mod common;

fn prng(seed: u64, i: usize, lo: i64, hi: i64) -> i64 {
    let mut x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(i as u64 + 1);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    lo + (x % (hi - lo + 1) as u64) as i64
}

fn program(m: usize, n: usize, k: usize, int8: bool) -> SpatialProgram {
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
    let ranges = if int8 {
        (Some((-128, 127)), Some((-128, 127)))
    } else {
        (None, None)
    };
    let t = template_from_spec(
        &spec,
        &names,
        (&Fill::Unknown, &Fill::Unknown),
        ranges,
        NumericsPolicy::Exact,
        &gpu,
    )
    .unwrap();
    run_passes(t, &gpu).unwrap()
}

fn lower(p: &SpatialProgram) -> Result<CudaArtifact, SpatialLowerError> {
    CudaPersistentBackend::new(SpatialTarget::rtx3060())
        .lower(p)
        .map(|a| match a {
            SpatialArtifact::Cuda(a) => a,
            other => panic!("{other:?}"),
        })
}

/// The operand FIFOs: every FIFO a tile read fills.
fn operand_fifos(p: &SpatialProgram) -> Vec<usize> {
    fn reads(ops: &[TaggedOp], out: &mut Vec<usize>) {
        for t in ops {
            match &t.op {
                ChoreoOp::ReadTiles { into, .. } => out.push(into.idx()),
                ChoreoOp::Loop { body, .. } => reads(body, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for prog in &p.schedule.as_ref().unwrap().programs {
        reads(&prog.ops, &mut out);
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// The text between the `{` that ends `head` and its matching `}`.
fn block_after<'a>(cu: &'a str, head: &str) -> Option<&'a str> {
    let open = cu.find(head)? + head.len();
    let mut depth = 1usize;
    for (i, ch) in cu[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&cu[open..open + i]);
                }
            }
            _ => {}
        }
    }
    None
}

fn nvcc_run(cu: &str, label: &str, acts: &[i64], weights: &[i64]) -> Vec<i64> {
    let dir =
        std::env::temp_dir().join(format!("logos_spatial_ring_{}_{label}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (src, bin) = (dir.join("ring.cu"), dir.join("ring"));
    std::fs::write(&src, cu).unwrap();
    let out = std::process::Command::new("nvcc")
        .args(["-O3", "-arch=sm_86", "-std=c++17", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect("nvcc");
    assert!(
        out.status.success(),
        "{label}: nvcc failed:\n{}\n\n{cu}",
        String::from_utf8_lossy(&out.stderr)
    );
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
    let run = std::process::Command::new(&bin)
        .current_dir(&dir)
        .output()
        .expect("run");
    assert!(
        run.status.success(),
        "{label}: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8_lossy(&run.stdout)
        .split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect()
}

#[test]
fn every_wait_publishes_its_page_to_the_whole_block() {
    for (m, n, k, int8) in [
        (256, 256, 256, true),
        (256, 256, 256, false),
        (2048, 2048, 2048, false),
        (200, 96, 136, true),
    ] {
        let cu = lower(&program(m, n, k, int8)).unwrap().program_cu;
        let lines: Vec<&str> = cu.lines().map(str::trim).collect();
        let mut waits = 0;
        for (i, l) in lines.iter().enumerate() {
            if !l.starts_with("__pipeline_wait_prior(") {
                continue;
            }
            waits += 1;
            let after = lines[i + 1..]
                .iter()
                .find(|l| !l.starts_with("__pipeline_wait_prior("))
                .copied();
            assert_eq!(
                after,
                Some("__syncthreads();"),
                "{m}x{n}x{k}: a run of waits is closed by a block barrier before line {}:\n{cu}",
                i + 2
            );
        }
        assert!(
            waits > 0,
            "{m}x{n}x{k}: the kernel waits on its operand rings"
        );
    }
}

#[test]
fn the_rotated_loop_waits_for_all_but_the_prefetched_block() {
    let cu = lower(&program(2048, 2048, 2048, false)).unwrap().program_cu;
    let head = cu
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("if (lv") && l.contains(" + 1 < "))
        .expect("the K loop is rotated at 2048³");
    let prefetch = block_after(&cu, head).unwrap();
    let issued = prefetch.matches("__pipeline_commit();").count();
    assert!(
        issued > 0,
        "the prefetch commits what it copies:\n{prefetch}"
    );
    let tail = &cu[cu.find(head).unwrap() + head.len() + prefetch.len()..];
    let last = block_after(tail, "else {").expect("the last block commits in place of a prefetch");
    assert_eq!(
        last.matches("__pipeline_commit();").count(),
        issued,
        "the last block commits as many (empty) groups as a prefetch:\n{last}"
    );
    assert_eq!(
        last.matches("__pipeline_memcpy_async").count(),
        0,
        "and copies nothing"
    );
    let waited = format!("__pipeline_wait_prior({issued});");
    assert!(
        tail.contains(&waited),
        "each wait leaves exactly the prefetched block's {issued} groups in flight:\n{cu}"
    );
    assert_eq!(
        cu.matches("__pipeline_wait_prior(").count(),
        cu.matches(&waited).count(),
        "every wait of the rotated loop has the same count:\n{cu}"
    );
}

#[test]
fn a_ring_too_shallow_to_rotate_prints_without_the_prefetch() {
    let (m, n, k) = (128, 128, 1024);
    let mut p = program(m, n, k, true);
    let fifos = operand_fifos(&p);
    assert!(!fifos.is_empty());
    for b in p.buffers.iter_mut() {
        if fifos.contains(&b.id.idx()) || b.origin.is_some_and(|o| fifos.contains(&o.idx())) {
            if let BufferKind::Fifo { depth_pages, .. } = &mut b.kind {
                *depth_pages = 1;
            }
        }
    }
    let cu = lower(&p).expect("a one-page ring still lowers").program_cu;
    assert!(
        !cu.contains("_pf"),
        "a one-page ring cannot hold a prefetched block beside the one being read:\n{cu}"
    );
    assert!(
        cu.contains("__pipeline_wait_prior(0);"),
        "without a prefetch every wait drains every group"
    );
    if !common::require_tool("nvcc") || !common::require_gpu() {
        return;
    }
    let acts: Vec<i64> = (0..m * k).map(|j| prng(7, j, -128, 127)).collect();
    let weights: Vec<i64> = (0..k * n).map(|j| prng(8, j, -128, 127)).collect();
    let mut inputs = Inputs::default();
    inputs.tensors.insert(TensorId(0), Value::Int(acts.clone()));
    inputs
        .tensors
        .insert(TensorId(1), Value::Int(weights.clone()));
    let expected = match &eval_choreography(&p, &inputs).unwrap().tensors[&TensorId(2)] {
        Value::Int(v) => v.clone(),
        Value::Float(v) => v.iter().map(|x| *x as i64).collect(),
    };
    assert_eq!(
        nvcc_run(&cu, "shallow", &acts, &weights),
        expected,
        "the unrotated kernel == eval_choreography"
    );
}

#[test]
fn a_producer_that_outruns_its_ring_is_refused() {
    let mut p = program(2048, 2048, 2048, false);
    let fifo = operand_fifos(&p)[0];
    // The producer fills the ring twice per step; its consumer drains it once.
    fn doubled(ops: &mut Vec<TaggedOp>, fifo: usize) -> bool {
        let run = ops
            .iter()
            .position(|t| matches!(&t.op, ChoreoOp::Reserve { buf, .. } if buf.idx() == fifo))
            .and_then(|start| {
                ops[start..]
                    .iter()
                    .position(|t| matches!(&t.op, ChoreoOp::Push { buf, .. } if buf.idx() == fifo))
                    .map(|len| (start, start + len + 1))
            });
        if let Some((start, end)) = run {
            let copy: Vec<TaggedOp> = ops[start..end].to_vec();
            ops.splice(end..end, copy);
            return true;
        }
        ops.iter_mut().any(|t| match &mut t.op {
            ChoreoOp::Loop { body, .. } => doubled(body, fifo),
            _ => false,
        })
    }
    let doubled_any = p
        .schedule
        .as_mut()
        .unwrap()
        .programs
        .iter_mut()
        .map(|prog| doubled(&mut prog.ops, fifo))
        .fold(false, |a, b| a | b);
    assert!(doubled_any, "the fixture doubles a producer");
    match lower(&p) {
        Err(SpatialLowerError::TooBig { .. }) => {}
        Err(SpatialLowerError::Unsupported(why)) if why.contains("drain") => {}
        other => panic!(
            "a ring filled faster than it drains must be refused, not printed: {:?}",
            other.map(|a| a.program_cu)
        ),
    }
}
