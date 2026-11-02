//! Does the REAL Qwen3.8-27B forward lower to the SN50 spatial fabric — not a miniature of it?
use logicaffeine_compile::codegen_cuda::qwen38::{gen_qwen38_lg, Qwen38Cfg};
use logicaffeine_compile::spatial::ir::{ComputeOp, StageKind, TensorRole};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;

/// Bytes this value takes in the encoding the neutral artifact ships, counted without building a string.
fn json_bytes<T: serde::Serialize>(v: &T) -> usize {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0 += b.len();
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut c = Count(0);
    let _ = serde_json::to_writer(&mut c, v);
    c.0
}

fn main() {
    let which = std::env::args().nth(1).unwrap_or_else(|| "real".into());
    let mut cfg = if which == "tiny" {
        Qwen38Cfg::tiny()
    } else {
        Qwen38Cfg::real()
    };
    // Optional overrides so the scaling wall can be located: layers, d, vocab.
    if let Some(v) = std::env::args().nth(2) {
        cfg.layers = v.parse().unwrap_or(cfg.layers);
    }
    if let Some(v) = std::env::args().nth(3) {
        cfg.d = v.parse().unwrap_or(cfg.d);
    }
    if let Some(v) = std::env::args().nth(4) {
        cfg.vocab = v.parse().unwrap_or(cfg.vocab);
    }
    let t0 = std::time::Instant::now();
    let (src, f32w, q8w) = gen_qwen38_lg(&cfg, "/w");
    println!(
        "cfg={which} layers={} d={} dff={} vocab={}",
        cfg.layers, cfg.d, cfg.dff, cfg.vocab
    );
    println!(
        "authored source: {} bytes, {} lines, {} f32 tensors, {} q8 tensors, gen {:?}",
        src.len(),
        src.lines().count(),
        f32w.len(),
        q8w.len(),
        t0.elapsed()
    );

    if let Ok(path) = std::env::var("LOGOS_DUMP_SRC") {
        std::fs::write(&path, &src).expect("write the authored source");
        println!("wrote {path}");
        return;
    }
    let target = RduChip::Sn50.facts().spatial_target();
    let t1 = std::time::Instant::now();
    match compile_program_to_spatial(&src, &target) {
        Ok(l) => {
            println!(
                "LOWERED {}/{} statements into {} sections  ({:?})",
                l.lowered,
                l.statements,
                l.sections,
                t1.elapsed()
            );
            println!("carried state tensors: {}", l.state.len());
            if !l.missing.is_empty() {
                println!("MISSING {} stage(s):", l.missing.len());
                let mut by_kind: std::collections::BTreeMap<String, usize> = Default::default();
                for m in &l.missing {
                    *by_kind.entry(m.kind.clone()).or_default() += 1;
                }
                for (k, n) in &by_kind {
                    println!("   {k}: {n}");
                }
                for m in l.missing.iter().take(3) {
                    println!(
                        "   e.g. stmt {} `{}` [{}]: {}",
                        m.stmt_index, m.binder, m.kind, m.why
                    );
                }
            }
            if let Some(p) = l.program.as_ref() {
                let mut by_init: std::collections::BTreeMap<&str, (usize, usize)> =
                    Default::default();
                for t in &p.tensors {
                    let k = match &t.init {
                        logicaffeine_compile::spatial::ir::TensorInit::Zero => "Zero",
                        logicaffeine_compile::spatial::ir::TensorInit::Pattern { .. } => "Pattern",
                        logicaffeine_compile::spatial::ir::TensorInit::Values(_) => "Values",
                        logicaffeine_compile::spatial::ir::TensorInit::FloatValues(_) => {
                            "FloatValues"
                        }
                        logicaffeine_compile::spatial::ir::TensorInit::ConstF(_) => "ConstF",
                        logicaffeine_compile::spatial::ir::TensorInit::RandomBf16 { .. } => {
                            "RandomBf16"
                        }
                        logicaffeine_compile::spatial::ir::TensorInit::External => "External",
                    };
                    let e = by_init.entry(k).or_default();
                    e.0 += 1;
                    e.1 += t.elems.rows * t.elems.cols;
                }
                println!("tensor inits (count, total elements):");
                for (k, (c, n)) in &by_init {
                    println!(
                        "   {k}: {c} tensors, {n} elements ~{} MB if materialized as 8B",
                        n * 8 / 1_000_000
                    );
                }
            }
            match l.whole() {
                Some(p) => {
                    let mm = p
                        .stages
                        .iter()
                        .filter(|s| {
                            matches!(s.kind, StageKind::Compute(ComputeOp::MatmulTiles { .. }))
                        })
                        .count();
                    let gen = p
                        .stages
                        .iter()
                        .filter(|s| matches!(s.kind, StageKind::Compute(ComputeOp::Generic { .. })))
                        .count();
                    let params: usize = p
                        .tensors
                        .iter()
                        .filter(|t| t.role == TensorRole::Param)
                        .count();
                    println!("WHOLE PROGRAM: {} stages ({mm} matmul, {gen} generic), {} tensors ({params} param), {} buffers, cert={}",
                        p.stages.len(), p.tensors.len(), p.buffers.len(), p.certificate.is_some());
                    // WHERE THE BYTES ARE. Counts alone do not say which field is the wall, so each
                    // component is serialized through a counting sink — the same encoding the neutral
                    // artifact uses, so the number is one a reader can check.
                    let mut ops = 0usize;
                    let mut top = 0usize;
                    if let Some(sch) = p.schedule.as_ref() {
                        for prog in &sch.programs {
                            top += prog.ops.len();
                            logicaffeine_compile::spatial::ir::walk_tagged_ops(
                                &prog.ops,
                                1,
                                &mut |_, _| ops += 1,
                            );
                        }
                        println!("schedule: {} engine programs, {top} top-level ops, {ops} ops with loop bodies", sch.programs.len());
                    }
                    println!(
                        "edges {}  semaphores {}  work items {}  sections {}",
                        p.edges.len(),
                        p.semaphores.len(),
                        p.work.items.len(),
                        p.sections.len()
                    );
                    // How many DISTINCT dataflows this forward runs — the number a layer loop collapses to.
                    let groups =
                        logicaffeine_compile::spatial::passes::section_shape::section_repeat_groups(
                            p,
                        );
                    let mut sizes: Vec<usize> = groups.iter().map(|(_, v)| v.len()).collect();
                    sizes.sort_unstable_by(|a, b| b.cmp(a));
                    println!(
                        "distinct section shapes: {} over {} sections; largest groups {:?}",
                        groups.len(),
                        p.sections.len(),
                        &sizes[..sizes.len().min(8)]
                    );
                    for (name, bytes) in [
                        ("stages", json_bytes(&p.stages)),
                        ("buffers", json_bytes(&p.buffers)),
                        ("tensors", json_bytes(&p.tensors)),
                        ("edges", json_bytes(&p.edges)),
                        ("work", json_bytes(&p.work)),
                        ("schedule", json_bytes(&p.schedule)),
                        ("placement", json_bytes(&p.placement)),
                        ("sections", json_bytes(&p.sections)),
                    ] {
                        println!("   {name}: {:.2} GB serialized", bytes as f64 / 1e9);
                    }
                }
                None => println!("NOT WHOLE — refused rather than reporting a fraction"),
            }
        }
        Err(e) => println!("REJECTED: {e:?}  ({:?})", t1.elapsed()),
    }
}
