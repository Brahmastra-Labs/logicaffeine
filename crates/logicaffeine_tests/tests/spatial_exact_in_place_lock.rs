//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! spatial_exact_in_place_lock — an exact GEMM on Tensix splits its reduction and reads each operand once.
//!
//! Exact numerics keep the partial sums at the accumulator's width: fp32 under bf16 operands. Wormhole and
//! Blackhole cannot reload an fp32 partial into the register file, so an exact plan that split K had nowhere
//! to finish, and none was offered: each core's block stayed at what its accumulator holds, the grid outgrew
//! the mesh, no operand could be multicast, and a 2048³ GEMM read its operands 22 times over.
//!
//! The packer adds every K block into local memory in place (`pack_reconfig_l1_acc`), and where the partial
//! already IS the result — the output's format, no compute after the matmul — the intermediate shares the
//! output's memory (`Buffer::alias_of`, two circular-buffer indices over one region, as tt-metal's own
//! bias-fused matmul does). The last K block then accumulates straight into the output page the writer stores:
//! no reload at all. The output's pages are reserved for each item before any partial lands in their memory,
//! so the writer has always stored the previous item first; the evaluator faults a write into a page its
//! alias still holds.
//!
//!   cargo nextest run -p logicaffeine-tests --test spatial_exact_in_place_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::spatial::eval::{eval_choreography_with, ChoreoFault, Scheduler, Value};
use logicaffeine_compile::spatial::ir::{
    BlockShape, BufferId, ChoreoOp, EnginePacking, NumericsPolicy, SpatialProgram, TaggedOp,
};
use logicaffeine_compile::spatial::passes::size_buffers::{
    footprint_bytes, footprint_bytes_for_plan,
};
use logicaffeine_compile::spatial::pipeline::{
    calibration_candidates_matching, compile_to_spatial_with, PlanFilter,
};
use logicaffeine_compile::spatial::target::{MeshShape, SiteSpec, SpatialTarget};
use logicaffeine_compile::spatial::work::work_of;
use spatial_support::*;

/// Every op of every engine program, loops flattened.
fn all_ops(p: &SpatialProgram) -> Vec<ChoreoOp> {
    fn walk(ops: &[TaggedOp], out: &mut Vec<ChoreoOp>) {
        for t in ops {
            if let ChoreoOp::Loop { body, .. } = &t.op {
                walk(body, out);
            }
            out.push(t.op.clone());
        }
    }
    let mut out = Vec::new();
    for prog in &p.schedule.as_ref().expect("a scheduled program").programs {
        walk(&prog.ops, &mut out);
    }
    out
}

fn reloads(p: &SpatialProgram) -> usize {
    all_ops(p)
        .iter()
        .filter(|op| {
            matches!(
                op,
                ChoreoOp::Compute {
                    acc_reload: Some(_),
                    ..
                }
            )
        })
        .count()
}

fn aliased(p: &SpatialProgram) -> Vec<(BufferId, BufferId)> {
    p.buffers
        .iter()
        .filter_map(|b| b.alias_of.map(|a| (b.id, a)))
        .collect()
}

fn as_ints(v: &Value) -> Vec<i64> {
    match v {
        Value::Int(x) => x.clone(),
        Value::Float(x) => x
            .iter()
            .map(|f| {
                assert_eq!(
                    f.fract(),
                    0.0,
                    "an integer-range GEMM must produce integral floats: {f}"
                );
                *f as i64
            })
            .collect(),
    }
}

/// The compulsory DRAM bytes of an exact `m` x `n` x `k` GEMM on Tensix: bf16 operands read once, the fp32
/// result written once.
fn floor(m: usize, n: usize, k: usize) -> u64 {
    ((m * k + k * n) * 2 + m * n * 4) as u64
}

#[test]
fn an_exact_tensix_gemm_reads_each_operand_once_and_never_reloads_a_partial() {
    for t in [SpatialTarget::wormhole_ttsim(), SpatialTarget::blackhole()] {
        for (m, n, k) in [(2048, 2048, 2048), (1024, 1024, 2048), (1536, 1536, 1536)] {
            let p = compile_to_spatial_with(&gemm_program(m, n, k), &t, NumericsPolicy::Exact)
                .unwrap_or_else(|e| panic!("{} {m}x{n}x{k}: {e:?}", t.name));
            let w = work_of(&p, &t);
            assert_eq!(w.dram_bytes, floor(m, n, k), "{} {m}x{n}x{k}: an exact plan moves {} DRAM bytes, {:.1}x what the GEMM must ({:?})", t.name, w.dram_bytes, w.dram_bytes as f64 / floor(m, n, k) as f64, p.work.plan);
            let plan = p.work.plan.expect("a planned GEMM");
            assert!(
                plan.inner.num_blocks > 1,
                "{} {m}x{n}x{k}: the reduction is split ({plan:?})",
                t.name
            );
            assert_eq!(
                reloads(&p),
                0,
                "{} {m}x{n}x{k}: an exact partial is never reloaded into the register file",
                t.name
            );
            assert!(
                !aliased(&p).is_empty(),
                "{} {m}x{n}x{k}: the intermediate shares the output's memory",
                t.name
            );
        }
    }
}

/// The best plan of `m` x `n` x `k` whose packer accumulates in place, under exact numerics, restricted to `filter`.
fn in_place(
    t: &SpatialTarget,
    (m, n, k): (usize, usize, usize),
    filter: PlanFilter,
) -> SpatialProgram {
    let filter = PlanFilter {
        pack_accumulate: Some(true),
        ..filter
    };
    let built =
        calibration_candidates_matching(&gemm_program(m, n, k), t, NumericsPolicy::Exact, filter)
            .unwrap_or_else(|e| panic!("{} {m}x{n}x{k}: {e:?}", t.name));
    let (_, p) = built
        .into_iter()
        .find(|(_, p)| {
            p.work
                .plan
                .is_some_and(|pl| pl.pack_accumulate && pl.inner.num_blocks > 1)
        })
        .unwrap_or_else(|| {
            panic!(
                "{} {m}x{n}x{k}: an exact plan that accumulates in place",
                t.name
            )
        });
    assert!(
        !aliased(&p).is_empty(),
        "{} {m}x{n}x{k}: the in-place plan aliases its intermediate to its output",
        t.name
    );
    assert_eq!(
        reloads(&p),
        0,
        "{} {m}x{n}x{k}: and reloads nothing",
        t.name
    );
    let plan = p.work.plan.expect("a planned GEMM");
    assert_eq!(
        footprint_bytes_for_plan(&p, &plan, t),
        footprint_bytes(&p, t),
        "{} {m}x{n}x{k}: the planner prices the in-place plan at what the allocator lays out ({plan:?})",
        t.name
    );
    p
}

#[test]
fn an_in_place_finish_computes_the_product_exactly() {
    for t in [SpatialTarget::wormhole_ttsim(), SpatialTarget::blackhole()] {
        for shape in [(128, 128, 256), (96, 160, 384)] {
            let p = in_place(&t, shape, PlanFilter::default());
            for sched in [
                Scheduler::RoundRobin,
                Scheduler::ComputeFirst,
                Scheduler::MovementFirst,
                Scheduler::Reverse,
            ] {
                let out = eval_choreography_with(&p, &inputs_from_fills(&p), sched)
                    .unwrap_or_else(|f| panic!("{} {shape:?} {sched:?}: {f:?}", t.name));
                assert_eq!(
                    as_ints(&out.tensors[&output_id(&p)]),
                    gemm_reference(shape.0, shape.1, shape.2),
                    "{} {shape:?} {sched:?}",
                    t.name
                );
            }
        }
    }
}

/// One core whose local memory holds a quarter of a 128 x 128 output's block and not the whole of it, so the
/// GEMM's items run on it in turn: the next item's first partial lands in memory the writer may still be storing
/// from.
fn one_core() -> SpatialTarget {
    let t = SpatialTarget::wormhole_ttsim();
    SpatialTarget {
        sites: MeshShape { x: 1, y: 1 },
        site: SiteSpec {
            local_mem_bytes: 48 * 1024,
            local_mem_opt_in_bytes: None,
            ..t.site
        },
        ..t
    }
}

/// The interleavings the evaluator explores: its named orders and a spread of seeded ones.
fn schedules() -> Vec<Scheduler> {
    let mut v = vec![
        Scheduler::RoundRobin,
        Scheduler::Reverse,
        Scheduler::MovementFirst,
        Scheduler::ComputeFirst,
    ];
    v.extend((0..64).map(Scheduler::Seeded));
    v
}

#[test]
fn the_output_is_reserved_before_a_partial_lands_in_its_memory() {
    let t = one_core();
    // The writer on a movement engine of its own: where the store shares an engine with an operand's loader,
    // program order alone keeps it ahead of the next item.
    let p = in_place(
        &t,
        (128, 128, 256),
        PlanFilter {
            per_site: Some((2, 2)),
            engines: Some(EnginePacking::LoadsTogether),
            ..PlanFilter::default()
        },
    );
    let (interm, out) = aliased(&p)[0];
    assert!(
        p.work.plan.is_some_and(|pl| pl.grid.0 * pl.grid.1 > 1),
        "several items share the one core"
    );
    let ops = enumerate_ops(p.schedule.as_ref().unwrap());
    let reserves: Vec<_> = ops
        .iter()
        .filter(|(_, op)| matches!(op, ChoreoOp::Reserve { buf, .. } if *buf == out))
        .map(|(path, _)| path.clone())
        .collect();
    assert_eq!(
        reserves.len(),
        1,
        "the item reserves its output pages once, up front, and the last block packs into them"
    );
    let reserve = reserves[0].clone();
    let out_inputs = inputs_from_fills(&p);
    let reference = gemm_reference(128, 128, 256);
    for sched in schedules() {
        let got = eval_choreography_with(&p, &out_inputs, sched)
            .unwrap_or_else(|f| panic!("{sched:?}: {f:?}"));
        assert_eq!(
            as_ints(&got.tensors[&output_id(&p)]),
            reference,
            "{sched:?}"
        );
    }
    // The same finish with the reservation taken page by page, as the pack is: the next item's partials may
    // then land in pages the writer has not stored.
    let mut late = without_op(&p, &reserve);
    let packs: Vec<_> = enumerate_ops(late.schedule.as_ref().unwrap())
        .into_iter()
        .filter(|(_, op)| matches!(op, ChoreoOp::PackAcc { to, .. } if *to == out))
        .map(|(path, _)| path)
        .collect();
    assert!(!packs.is_empty(), "the last block packs into the output");
    for path in packs.iter().rev() {
        edit_op(&mut late, path, |list, i| {
            let stage = list[i].stage;
            list.insert(
                i,
                TaggedOp {
                    stage,
                    op: ChoreoOp::Reserve { buf: out, pages: 1 },
                },
            );
        });
    }
    let mut clobbered = 0;
    for sched in schedules() {
        match eval_choreography_with(&late, &out_inputs, sched) {
            Err(ChoreoFault::AliasClobber { buf, alias, .. }) => {
                assert_eq!(
                    (buf, alias),
                    (interm, out),
                    "{sched:?}: the clobber is named on the intermediate and the output it aliases"
                );
                clobbered += 1;
            }
            Ok(got) => assert_eq!(
                as_ints(&got.tensors[&output_id(&late)]),
                reference,
                "{sched:?}: an interleaving the hazard misses still computes the product"
            ),
            Err(other) => {
                panic!("{sched:?}: the late reservation may fail only by clobbering, got {other:?}")
            }
        }
    }
    assert!(clobbered > 0, "some interleaving lets the next item's partial land in an output page the writer has not stored");
}

#[test]
fn only_an_exact_partial_on_a_target_whose_packer_can_finish_in_place_aliases_the_output() {
    let (m, n, k) = (128, 128, 256);
    let rounding = calibration_candidates_matching(
        &gemm_program(m, n, k),
        &SpatialTarget::wormhole_ttsim(),
        NumericsPolicy::AllowSpillRounding,
        PlanFilter {
            pack_accumulate: Some(true),
            ..PlanFilter::default()
        },
    )
    .expect("rounding candidates");
    assert!(
        rounding.iter().all(|(_, p)| aliased(p).is_empty()),
        "a rounded partial is not the result: it keeps the reload, and its numerics"
    );
    let fabric = calibration_candidates_matching(
        &gemm_program(16, 16, 32),
        &SpatialTarget::sky_fabric(16, 16),
        NumericsPolicy::Exact,
        PlanFilter::default(),
    )
    .expect("fabric candidates");
    assert!(
        fabric.iter().all(|(_, p)| aliased(p).is_empty()),
        "the fabric's packer accumulates in local memory but not into its output"
    );
}

#[test]
fn an_exact_reduction_over_data_of_unknown_range_keeps_its_order() {
    // External data: no operand bound, so no partial sum is known to be an integer fp32 holds exactly, and adding a
    // block's sum to the partial would round differently from the straight K-ascending accumulation.
    use logicaffeine_compile::spatial::ir::NumericsClaim;
    use logicaffeine_compile::spatial::passes::from_tensor_op::MatmulNames;
    use logicaffeine_compile::spatial::pipeline::{run_passes, template_from_spec};
    use logicaffeine_compile::tensor_recognize::{Fill, MatmulSpec};
    for t in [SpatialTarget::wormhole_ttsim(), SpatialTarget::blackhole()] {
        for (m, n, k) in [(2048, 2048, 2048), (128, 128, 256)] {
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
            let template = template_from_spec(
                &spec,
                &names,
                (&Fill::Unknown, &Fill::Unknown),
                (None, None),
                NumericsPolicy::Exact,
                &t,
            )
            .expect("an external-data template");
            assert!(
                matches!(template.claim, NumericsClaim::Approx { .. }),
                "{} {m}x{n}x{k}: external data claims no exactness",
                t.name
            );
            let p = run_passes(template, &t)
                .unwrap_or_else(|e| panic!("{} {m}x{n}x{k}: {e:?}", t.name));
            assert!(aliased(&p).is_empty(), "{} {m}x{n}x{k}: a reduction whose sums are not provably exact never finishes in place", t.name);
        }
    }
}

#[test]
fn the_intermediate_is_an_index_of_the_outputs_circular_buffer() {
    use logicaffeine_compile::spatial::lower::tensix::TensixBackend;
    let t = SpatialTarget::wormhole_ttsim();
    let p = in_place(&t, (128, 128, 256), PlanFilter::default());
    let art = TensixBackend::new(t.clone())
        .lower_tensix(&p)
        .expect("the in-place GEMM lowers to Tensix");
    let configs: Vec<&str> = art
        .host_cpp
        .lines()
        .filter(|l| l.contains("CircularBufferConfig cb_"))
        .collect();
    assert_eq!(
        configs.len(),
        3,
        "the two operands and the output: the intermediate has no config of its own\n{configs:#?}"
    );
    let shared: Vec<&&str> = configs
        .iter()
        .filter(|l| {
            l.contains("{16, tt::DataFormat::Float32}")
                && l.contains("{24, tt::DataFormat::Float32}")
        })
        .collect();
    assert_eq!(shared.len(), 1, "one config carries the output's index and the intermediate's over the same memory\n{configs:#?}");
    assert!(
        shared[0].contains("set_page_size(16,") && shared[0].contains("set_page_size(24,"),
        "both indices have their page size\n{}",
        shared[0]
    );
    for k in &art.kernels {
        assert!(
            !k.source.contains("copy_tile_to_dst_init_short"),
            "{}: no partial is reloaded into the register file",
            k.name
        );
    }
}

#[test]
fn a_ragged_grid_finishes_in_place_at_its_boundary_sites_too() {
    // Three tile rows over a two-row block: the boundary site holds one row, so it walks one sub-block of the
    // nominal two, and its intermediate and output must be sized to the sub-blocks it walks, page for page.
    let t = SpatialTarget::wormhole_ttsim();
    let shape = (96, 64, 256);
    let filter = PlanFilter {
        per_site: Some((2, 2)),
        block: Some(BlockShape {
            osh: 1,
            osw: 2,
            ibw: 2,
        }),
        ..PlanFilter::default()
    };
    let p = in_place(&t, shape, filter);
    assert!(
        p.work.plan.is_some_and(|pl| 3 % pl.per_site_m != 0),
        "the grid is ragged in M"
    );
    let mm = p.memory.as_ref().expect("a laid-out program");
    for (b, owner) in aliased(&p) {
        let pages = |id: BufferId| {
            (
                p.buffer(id).kind.depth_pages(),
                p.buffer(id).kind.page_tiles(),
            )
        };
        assert_eq!(
            pages(b),
            pages(owner),
            "{}: an intermediate and the output it shares are the same pages",
            p.buffer(b).name
        );
        assert_eq!(
            mm.buffer_at[b.idx()].offset_bytes,
            mm.buffer_at[owner.idx()].offset_bytes
        );
    }
    for sched in schedules() {
        let out = eval_choreography_with(&p, &inputs_from_fills(&p), sched)
            .unwrap_or_else(|f| panic!("{shape:?} {sched:?}: {f:?}"));
        assert_eq!(
            as_ints(&out.tensors[&output_id(&p)]),
            gemm_reference(shape.0, shape.1, shape.2),
            "{shape:?} {sched:?}"
        );
    }
}
