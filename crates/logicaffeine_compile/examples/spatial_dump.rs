//! Scratch: print a scheduled spatial program's engine programs (temporary, removed after the fabric lowering lands).
use logicaffeine_compile::codegen_sva::systolic::{FusedBlock, Systolic};
use logicaffeine_compile::spatial::ir::*;
use logicaffeine_compile::spatial::passes::from_tensor_op::{
    from_fused_block, from_scalar_body, MatmulNames,
};
use logicaffeine_compile::spatial::pipeline::{run_passes, template_from_spec};
use logicaffeine_compile::spatial::target::{
    HandshakeKind, Interconnect, MeshShape, SpatialTarget, TileShape,
};
use logicaffeine_compile::tensor_recognize::{Fill, MatmulSpec};

fn small_mesh() -> SpatialTarget {
    let mut t = SpatialTarget::sky_fabric(16, 16);
    t.sites = MeshShape { x: 2, y: 2 };
    t.tile = TileShape { rows: 4, cols: 4 };
    t.site.acc_tiles = 4;
    t.interconnect.insert(Interconnect::MulticastRect);
    t.handshakes.insert(HandshakeKind::Semaphore);
    t.cycle = SpatialTarget::wormhole().cycle;
    t
}
fn dump_ops(ops: &[TaggedOp], depth: usize) {
    for t in ops {
        match &t.op {
            ChoreoOp::Loop { var, count, body } => {
                println!(
                    "{}Loop {:?} x{} [stage {}]",
                    "  ".repeat(depth),
                    var,
                    count.bound(),
                    t.stage.idx()
                );
                dump_ops(body, depth + 1);
            }
            op => println!("{}{:?} [stage {}]", "  ".repeat(depth), op, t.stage.idx()),
        }
    }
}
fn dump(label: &str, p: &SpatialProgram) {
    println!("===== {label}: {} phase {:?}", p.name, p.phase());
    for t in &p.tensors {
        println!(
            "tensor {} {:?} {}x{} tiles {}x{} {:?}",
            t.id.idx(),
            t.role,
            t.elems.rows,
            t.elems.cols,
            t.tiles.rows_t,
            t.tiles.cols_t,
            t.dtype
        );
    }
    for s in &p.stages {
        println!(
            "stage {} {} {:?} site {:?} ins {:?} outs {:?} origin {:?}",
            s.id.idx(),
            s.name,
            std::mem::discriminant(&s.kind),
            s.site,
            s.ins,
            s.outs,
            s.origin
        );
    }
    for b in &p.buffers {
        println!(
            "buffer {} {} {:?} {:?} site {:?} origin {:?}",
            b.id.idx(),
            b.name,
            b.kind,
            b.dtype,
            b.site,
            b.origin
        );
    }
    for s in &p.semaphores {
        println!(
            "sem {} init {} {:?} operand {:?} sites {:?}",
            s.id.idx(),
            s.init,
            s.role,
            s.operand,
            s.sites
        );
    }
    for e in &p.edges {
        println!(
            "edge {:?} {:?} -> {:?} {:?}",
            e.from, e.buffer, e.to, e.handshake
        );
    }
    if let Some(pl) = &p.placement {
        for g in &pl.groups {
            println!("group {:?}", g);
        }
        for (s, items) in &pl.items_of_site {
            println!("site {:?} items {:?}", s, items);
        }
    }
    if let Some(m) = &p.memory {
        println!("memory {:?}", m);
    }
    println!("plan {:?} k_tiles {}", p.work.plan, p.work.k_tiles);
    for prog in &p.schedule.as_ref().unwrap().programs {
        println!(
            "--- engine site ({},{}) {:?} stages {:?}",
            prog.site.x, prog.site.y, prog.engine, prog.stages
        );
        dump_ops(&prog.ops, 1);
    }
}
fn main() {
    let names = MatmulNames {
        a: "a".into(),
        b: "b".into(),
        c: "c".into(),
    };
    let t = small_mesh();
    let spec = MatmulSpec {
        m: 8,
        n: 8,
        k: 8,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    let p = run_passes(
        template_from_spec(
            &spec,
            &names,
            (&Fill::Unknown, &Fill::Unknown),
            (Some((-128, 127)), Some((-128, 127))),
            NumericsPolicy::Exact,
            &t,
        )
        .unwrap(),
        &t,
    )
    .unwrap();
    dump("mcast 2x2 8^3", &p);
    let mut t2 = small_mesh();
    t2.interconnect.remove(&Interconnect::MulticastRect);
    let spec = MatmulSpec {
        m: 8,
        n: 4,
        k: 10,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    let p = run_passes(
        template_from_spec(
            &spec,
            &names,
            (&Fill::Unknown, &Fill::Unknown),
            (Some((-128, 127)), Some((-128, 127))),
            NumericsPolicy::Exact,
            &t2,
        )
        .unwrap(),
        &t2,
    )
    .unwrap();
    dump("streamed 8x4x10", &p);
    let sky = SpatialTarget::sky_fabric(16, 16);
    let fb = FusedBlock {
        sys: Systolic::new(4, 4, (0..16).map(|i| (i as i64 % 7) - 3).collect()),
        requant_m: 181,
        requant_s: 8,
        bias: vec![1, 2, 3, 4],
    };
    let p = run_passes(from_fused_block(&fb, 5, &sky).unwrap(), &sky).unwrap();
    dump("fused block", &p);
    let df = logicaffeine_compile::codegen_sva::fabric_rtl::decision_fabric_from_source(
        "## To affine (x: Int) -> Int:\n    Return 3 * x + 7.\n",
        "affine",
    );
    let p = run_passes(from_scalar_body(&df, 8, &sky).unwrap(), &sky).unwrap();
    dump("scalar body", &p);
}
