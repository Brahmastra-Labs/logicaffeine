//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! SPATIAL IR LOCK — the one program every spatial backend lowers from is a REAL, structurally
//! validated object, not a convention.
//!
//! A `SpatialProgram` (stages / bounded buffers / typed handshakes / a mesh placement / a per-engine
//! choreography) is what "one IR, every chip" means. This lock pins:
//!   A. the template a recognized matmul lifts to (four stages, three FIFOs, one work item per output tile,
//!      tensors named from the source, the fill recognized, the numerics claim honest);
//!   B. `validate()` refuses every structural corruption with a TYPED violation — dangling ids, a FIFO
//!      with two consumers, an output tile never stored (or stored twice), an out-of-order phase marker,
//!      an intermediate buffer narrower than the accumulator under exact numerics;
//!   C. the neutral dump is deterministic and TOTAL (every object appears; any mutation changes it);
//!   D. the plan summary is DERIVED from structure — there is no strategy field anywhere;
//!   E. the target presets pin their published numbers and declare which fields are modeled, and the
//!      fabric lane's `FabricTarget` is EMBEDDED in the descriptor, never duplicated.
//!
//! Run: `cargo test -p logicaffeine-tests --test spatial_ir_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::codegen_cuda::chip::CudaChip;
use logicaffeine_compile::codegen_sva::fabric_target::{FabricTarget, Precision};
use logicaffeine_compile::spatial::ir::{
    AccPolicy, BufferId, BufferKind, ChoreoOp, ComputeOp, Handshake, IrViolation, NumericsClaim,
    NumericsPolicy, Phase, Schedule, SiteRole, StageId, StageKind, TaggedOp, TensorInit, TileGrid,
};
use logicaffeine_compile::spatial::pipeline::{
    compile_to_spatial, run_passes, template_from_source,
};
use logicaffeine_compile::spatial::target::{
    ComputeCap, HandshakeKind, Interconnect, MeshShape, SpatialTarget, TileShape,
};
use spatial_support::*;

#[test]
fn template_from_matmul_has_four_stages_three_fifos_and_all_tiles_as_work() {
    let wh = SpatialTarget::wormhole();
    let p = template_from_source(&gemm_program(64, 128, 96), &wh)
        .expect("the canonical matmul lifts to a template");
    assert_eq!(p.phase(), Phase::Template);
    p.validate().expect("a fresh template validates");

    let names: Vec<&str> = p.tensors.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["a", "b", "c"], "tensors are named from the source");
    assert_eq!(
        p.tensors[0].tiles,
        TileGrid {
            rows_t: 2,
            cols_t: 3
        }
    );
    assert_eq!(
        p.tensors[1].tiles,
        TileGrid {
            rows_t: 3,
            cols_t: 4
        }
    );
    assert_eq!(
        p.tensors[2].tiles,
        TileGrid {
            rows_t: 2,
            cols_t: 4
        }
    );
    assert_eq!(p.tile, TileShape::TENSIX);
    // The fills are recognized from the array-build loops, so a host can reproduce the data.
    assert_eq!(
        p.tensors[0].init,
        TensorInit::Pattern {
            modulus: 7,
            shift: -3
        }
    );
    assert_eq!(
        p.tensors[1].init,
        TensorInit::Pattern {
            modulus: 11,
            shift: -5
        }
    );
    assert_eq!(p.tensors[2].init, TensorInit::Zero);
    // Wormhole's matmul FPU is float-primary: an Int source rides as bf16 operands with an fp32 accumulator,
    // and the claim is EXACT only because the fill's range and K keep every partial sum inside fp32's mantissa.
    assert_eq!(p.tensors[0].dtype, Precision::Bf16);
    assert_eq!(p.tensors[2].dtype, Precision::Fp32);
    assert_eq!(p.claim, NumericsClaim::Exact);

    let kinds: Vec<&str> = p
        .stages
        .iter()
        .map(|s| match &s.kind {
            StageKind::Load { .. } => "load",
            StageKind::Store { .. } => "store",
            StageKind::Recv { .. } => "recv",
            StageKind::AllReduce { .. } => "allreduce",
            StageKind::Compute(ComputeOp::MatmulTiles { .. }) => "matmul",
            StageKind::Compute(_) => "compute",
            StageKind::Scalar(_) => "scalar",
        })
        .collect();
    assert_eq!(kinds, ["load", "load", "matmul", "store"]);
    assert_eq!(p.stages[0].role, SiteRole::Movement);
    assert_eq!(p.stages[2].role, SiteRole::Compute);
    match &p.stages[2].kind {
        StageKind::Compute(ComputeOp::MatmulTiles { acc, operand, .. }) => {
            assert_eq!(*acc, AccPolicy::F32);
            assert_eq!(*operand, Precision::Bf16);
        }
        other => panic!("stage 2 must be the tile matmul, got {other:?}"),
    }
    assert_eq!(p.buffers.len(), 3);
    for b in &p.buffers {
        assert!(
            matches!(b.kind, BufferKind::Fifo { depth_pages: 2, .. }),
            "template FIFOs are double-buffered: {b:?}"
        );
    }
    assert_eq!(p.edges.len(), 3);
    assert!(p.edges.iter().all(|e| e.handshake == Handshake::Fifo));
    assert_eq!(p.work.items.len(), 2 * 4, "one work item per output tile");
    assert_eq!(p.work.k_tiles, 3);
    assert!(p.work.plan.is_none());
    assert!(
        p.placement.is_none()
            && p.memory.is_none()
            && p.schedule.is_none()
            && p.certificate.is_none()
    );
}

#[test]
fn ids_are_dense_newtypes_and_validate_rejects_dangling_references() {
    let wh = SpatialTarget::wormhole();
    let p = template_from_source(&gemm_program(64, 64, 64), &wh).unwrap();
    let mut dangling = p.clone();
    dangling.edges[0].buffer = BufferId(99);
    assert!(
        matches!(
            dangling.validate(),
            Err(IrViolation::DanglingBuffer {
                buffer: BufferId(99),
                ..
            })
        ),
        "{:?}",
        dangling.validate()
    );
    let mut sparse = p.clone();
    sparse.stages.swap(0, 1);
    assert!(
        matches!(
            sparse.validate(),
            Err(IrViolation::IdNotDense { what: "stage", .. })
        ),
        "{:?}",
        sparse.validate()
    );
}

#[test]
fn every_fifo_has_exactly_one_producer_and_one_consumer() {
    let wh = SpatialTarget::wormhole();
    let p = template_from_source(&gemm_program(64, 64, 64), &wh).unwrap();
    let mut two_consumers = p.clone();
    let mut extra = two_consumers.edges[0].clone();
    extra.id = logicaffeine_compile::spatial::ir::EdgeId(two_consumers.edges.len() as u32);
    // Route the same buffer to the store stage as a second consumer.
    let store = two_consumers
        .stages
        .iter()
        .find(|s| matches!(s.kind, StageKind::Store { .. }))
        .unwrap()
        .id;
    extra.to = vec![store];
    two_consumers.stages[store.idx()].ins.push(extra.buffer);
    two_consumers.edges.push(extra);
    match two_consumers.validate() {
        Err(IrViolation::FifoProducerCount { count: 2, .. })
        | Err(IrViolation::FifoConsumerCount { count: 2, .. }) => {}
        other => panic!("a FIFO with two consumers must be refused, got {other:?}"),
    }
}

#[test]
fn output_tensor_must_be_covered_disjointly_by_stores() {
    let wh = SpatialTarget::wormhole();
    let p = template_from_source(&gemm_program(64, 64, 64), &wh).unwrap();
    let mut missing = p.clone();
    missing.work.items.pop();
    assert!(
        matches!(missing.validate(), Err(IrViolation::OutputUncovered { .. })),
        "{:?}",
        missing.validate()
    );
    let mut doubled = p.clone();
    let mut dup = doubled.work.items[0].clone();
    dup.id = logicaffeine_compile::spatial::ir::WorkItemId(doubled.work.items.len() as u32);
    doubled.work.items.push(dup);
    assert!(
        matches!(doubled.validate(), Err(IrViolation::OutputOverlap { .. })),
        "{:?}",
        doubled.validate()
    );
}

#[test]
fn phase_markers_are_monotone() {
    let wh = SpatialTarget::wormhole();
    let mut p = template_from_source(&gemm_program(64, 64, 64), &wh).unwrap();
    p.schedule = Some(Schedule { programs: vec![] });
    assert!(
        matches!(
            p.validate(),
            Err(IrViolation::PhaseOrder {
                have: "schedule",
                ..
            })
        ),
        "{:?}",
        p.validate()
    );
}

#[test]
fn interm_narrower_than_acc_is_rejected_under_exact_numerics() {
    // The subject here is the VALIDATOR, not the device: an intermediate narrower than the accumulator is a
    // rounding, and the Exact policy must refuse it while a declared spill-rounding policy accepts it.
    //
    // That needs a program that actually spills, so the part is one that declares it can reload a partial at
    // the accumulator's width. Real Wormhole cannot — its exact plans keep the reduction in the register file
    // or stream it, which is pinned separately by
    // `spatial_choreography_semantics_lock::f32_accumulation_equals_eval_systolic_bf16_bitwise_including_multi_block_spills`.
    // Building the fixture on a capable part keeps this a validator lock rather than one that quietly stops
    // testing anything the day a device refuses to spill.
    let mut wh = SpatialTarget::wormhole();
    for c in wh.compute.iter_mut() {
        if let ComputeCap::MatmulTile {
            wide_spill_reload, ..
        } = c
        {
            *wide_spill_reload = true;
        }
    }
    let p =
        compile_to_spatial(&gemm_program(2048, 2048, 1024), &wh).expect("2048x2048x1024 compiles");
    let interm = p
        .stages
        .iter()
        .find_map(|s| match &s.kind {
            StageKind::Compute(ComputeOp::MatmulTiles {
                interm: Some(i), ..
            }) => Some(*i),
            _ => None,
        })
        .expect("a two-block plan spills into an intermediate buffer");
    assert_eq!(
        p.buffers[interm.idx()].dtype,
        Precision::Fp32,
        "the spill carries the fp32 partial sum losslessly"
    );
    let mut narrowed = p.clone();
    narrowed.certificate = None;
    narrowed.buffers[interm.idx()].dtype = Precision::Bf16;
    assert!(
        matches!(
            narrowed.validate(),
            Err(IrViolation::IntermNarrowerThanAcc { .. })
        ),
        "{:?}",
        narrowed.validate()
    );
    narrowed.numerics = NumericsPolicy::AllowSpillRounding;
    narrowed
        .validate()
        .expect("a declared spill-rounding policy accepts the narrower intermediate");
}

#[test]
fn neutral_dump_is_deterministic_and_total() {
    let wh = SpatialTarget::wormhole();
    let p = template_from_source(&gemm_program(64, 128, 96), &wh).unwrap();
    let d1 = p.neutral_dump();
    let d2 = p.neutral_dump();
    assert_eq!(d1, d2, "the dump must be byte-deterministic");
    for s in &p.stages {
        assert!(
            d1.contains(&s.name),
            "stage {} missing from the dump",
            s.name
        );
    }
    for b in &p.buffers {
        assert!(
            d1.contains(&b.name),
            "buffer {} missing from the dump",
            b.name
        );
    }
    for t in &p.tensors {
        assert!(
            d1.contains(&format!("\"{}\"", t.name)),
            "tensor {} missing from the dump",
            t.name
        );
    }
    let back = logicaffeine_compile::spatial::ir::SpatialProgram::parse_neutral(&d1)
        .expect("the dump parses back");
    assert_eq!(back, p, "the dump round-trips to the identical program");
    // Ten distinct mutations, ten distinct dumps.
    let mut seen = std::collections::BTreeSet::new();
    seen.insert(d1.clone());
    for i in 0..10usize {
        let mut q = p.clone();
        match i {
            0 => q.name.push('x'),
            1 => q.tensors[0].name.push('x'),
            2 => q.stages[0].name.push('x'),
            3 => q.buffers[0].name.push('x'),
            4 => q.work.k_tiles += 1,
            5 => q.numerics = NumericsPolicy::AllowSpillRounding,
            6 => {
                q.claim = NumericsClaim::Approx {
                    reason: "mutated".into(),
                }
            }
            7 => q.acc_tiles_used += 1,
            8 => q.tensors[2].dtype = Precision::Bf16,
            _ => q.edges[0].handshake = Handshake::ValidReady,
        }
        assert!(
            seen.insert(q.neutral_dump()),
            "mutation {i} did not change the dump — the dump is not total"
        );
    }
}

#[test]
fn summary_is_derived_from_structure_not_a_stored_enum() {
    let wh = SpatialTarget::wormhole();
    let p = fused_2048_program();
    let s = p.summary();
    assert_eq!(
        s.multicast_groups, 16,
        "a 2-D 8×8 plan fuses readers into 8 row + 8 column groups: {s:?}"
    );
    assert_eq!((s.row_groups, s.col_groups), (8, 8));
    let mut q = p.clone();
    q.certificate = None;
    q.placement.as_mut().unwrap().groups.clear();
    assert_eq!(
        q.summary().multicast_groups,
        0,
        "the summary reads the structure, it is not a stored decision"
    );
    let json = serde_json::to_value(&s).unwrap();
    for key in json.as_object().unwrap().keys() {
        assert!(
            !key.to_lowercase().contains("strateg"),
            "no strategy field may exist on the summary: {key}"
        );
    }
    let template = template_from_source(&gemm_program(64, 64, 64), &wh)
        .unwrap()
        .summary();
    assert_eq!(template.phase, Phase::Template);
    assert_eq!(template.multicast_groups, 0);
    assert_eq!(
        template.dram_tiles_read,
        2 * 2 * 2 * 2,
        "a template reads every operand tile per output tile"
    );
}

#[test]
fn validate_after_every_pass_holds_on_the_whole_preset_matrix() {
    // ★ THE SIZES ARE IN TILES, NOT ELEMENTS. This used to run a fixed 64³ and 640³ on every
    // preset, which is not one workload run five times — it is five wildly different ones,
    // because the preset's TILE SHAPE decides what 640 elements become:
    //
    //     wormhole / blackhole / sn40l   32×32 tile  ->   20×20 =     400 tiles, k=20
    //     rtx3060                        16×16 tile  ->   40×40 =   1,600 tiles, k=40
    //     sky130-fabric                    1×1 tile  -> 640×640 = 409,600 tiles, k=640
    //
    // The fabric lane drew 1024× the structural work of the Tensix lanes for the SAME
    // assertion. `run_passes` walks eight passes, rebuilding the program and re-running
    // `validate()` at each one, so that lane alone drove this binary past 100 GB RSS and
    // OOM-killed the user slice — seven times in the fortnight to 2026-09-12, taking live
    // SSH sessions down with it.
    //
    // What the invariant needs is STRUCTURAL: a grid with several tiles on both axes and
    // several k-steps, on every preset. Sizing in TILES gives every preset exactly that and
    // the SAME amount of it, so the lock finally compares like with like. G = 20 reproduces
    // the original 640 on the 32×32 Tensix presets, so the lane this test was written for is
    // bit-for-bit unchanged. (Every preset's tile is square, so one factor sizes m, n and k.)
    for target in SpatialTarget::presets() {
        for g in [2usize, 20] {
            let (m, n, k) = (
                target.tile.rows * g,
                target.tile.cols * g,
                target.tile.rows * g,
            );
            let p = template_from_source(&gemm_program(m, n, k), &target)
                .unwrap_or_else(|e| panic!("{} {m}x{n}x{k}: template refused: {e:?}", target.name));
            p.validate()
                .unwrap_or_else(|e| panic!("{} {m}x{n}x{k}: template invalid: {e:?}", target.name));
            let verified = run_passes(p, &target)
                .unwrap_or_else(|e| panic!("{} {m}x{n}x{k}: passes refused: {e:?}", target.name));
            verified.validate().unwrap_or_else(|e| {
                panic!(
                    "{} {m}x{n}x{k}: verified program invalid: {e:?}",
                    target.name
                )
            });
            assert_eq!(
                verified.phase(),
                Phase::Verified,
                "{} {m}x{n}x{k}",
                target.name
            );
        }
    }
}

#[test]
fn the_simulated_wormhole_is_the_production_chip_on_its_slow_dispatch_grid() {
    // The 80-core descriptor under slow dispatch exposes 8×9 compute-with-storage cores; everything else about
    // the chip is identical, and the chip id (the ledger key) does not change with the deployment grid.
    let sim = SpatialTarget::wormhole_ttsim();
    let prod = SpatialTarget::wormhole();
    assert_eq!((sim.sites.x, sim.sites.y), (8, 9));
    assert_eq!(sim.name, prod.name);
    assert_eq!(prod.clone().with_sites(8, 9), sim);
    assert_eq!(sim.with_sites(8, 8), prod);
    assert_eq!(
        logicaffeine_compile::spatial::target::sim_grid("wormhole_b0"),
        Some(MeshShape { x: 8, y: 9 })
    );
    assert_eq!(
        logicaffeine_compile::spatial::target::sim_grid("blackhole"),
        Some(SpatialTarget::blackhole().sites),
        "the Blackhole preset already is its simulated grid"
    );
    assert_eq!(
        logicaffeine_compile::spatial::target::sim_grid("n150"),
        None
    );
}

#[test]
fn target_presets_pin_published_numbers_and_declare_modeled_fields() {
    let wh = SpatialTarget::wormhole();
    assert_eq!((wh.sites.x, wh.sites.y), (8, 8));
    assert_eq!(wh.site.local_mem_bytes, 1464 * 1024);
    assert_eq!(wh.site.acc_tiles, 8);
    assert!(wh.site.fp32_acc_halves_acc_tiles);
    assert_eq!(
        wh.effective_acc_tiles(AccPolicy::F32, true),
        4,
        "fp32 partials that must survive packing halve the DST"
    );
    assert_eq!(
        wh.effective_acc_tiles(AccPolicy::F32, false),
        8,
        "bf16 spill rounding keeps the whole DST"
    );
    assert_eq!(wh.effective_acc_tiles(AccPolicy::I64, true), 8);
    assert_eq!((wh.site.movement_engines, wh.site.nocs), (2, 2));
    assert!(wh.provides_interconnect(Interconnect::MulticastRect));
    assert!(
        wh.provides_handshake(HandshakeKind::Semaphore)
            && wh.provides_handshake(HandshakeKind::Fifo)
    );
    assert!(
        !wh.matmul_supports(Precision::Int8),
        "the Tensix matmul FPU is float-primary (measured): Int8 must not be advertised"
    );
    assert!(wh.matmul_supports(Precision::Bf16));
    assert!(wh.modeled.is_empty(), "Wormhole is fully published");
    let rule = wh
        .noc_rule
        .expect("Wormhole carries the measured NOC direction rule");
    assert_eq!(
        (
            rule.sender_noc,
            rule.receiver_noc,
            rule.writer_noc_default,
            rule.writer_noc_first_column
        ),
        (0, 1, 0, 1)
    );

    let bh = SpatialTarget::blackhole();
    assert_eq!((bh.sites.x, bh.sites.y), (13, 10));
    // ★ THE SAME EIGHT-TILE ACCUMULATOR AS WORMHOLE. This read 16, on the belief that Blackhole has a
    // bigger DST. It does not: the vendor's `tensix_types.h` gives both parts a 16x16 dest face, so
    // `DEST_NUM_TILES_FP16` is 16 on each — and our kernels use `tile_regs_acquire`/`tile_regs_commit`,
    // which is half-sync, so eight is what a single acquire can reach. The claim of sixteen produced a
    // WRONG ANSWER the first time a plan used more than eight (2048x1024x512 on Blackhole, PCC 0.872 at
    // exactly the right retired arithmetic). `spatial_chip_facts_lock` holds it to the vendor header now.
    assert_eq!(bh.site.acc_tiles, 8);
    assert_eq!(
        bh.effective_acc_tiles(AccPolicy::F32, true),
        4,
        "fp32 partials that must survive packing halve it, as on Wormhole"
    );

    let rdu = SpatialTarget::sn40l_rdu();
    let published = FabricTarget::rdu_shaped()
        .silicon
        .expect("the SN40L descriptor is published")
        .compute_tiles;
    assert_eq!(
        rdu.sites.sites(),
        published,
        "the RDU mesh has exactly the published PCU count"
    );
    for f in ["sites", "acc_tiles", "tile"] {
        assert!(
            rdu.modeled.contains(&f),
            "the RDU descriptor must declare `{f}` as modeled"
        );
    }
    assert!(rdu.provides_handshake(HandshakeKind::Token));

    let gpu = SpatialTarget::rtx3060();
    assert_eq!(
        gpu.site.local_mem_opt_in_bytes,
        Some(CudaChip::rtx3060().smem_kib_per_block * 1024)
    );
    assert_eq!(gpu.tile, TileShape::WMMA16);
    assert!(
        gpu.provides_interconnect(Interconnect::SharedMemoryBlock)
            && !gpu.provides_interconnect(Interconnect::MulticastRect)
    );
    assert!(gpu.modeled.contains(&"acc_tiles"));

    let sky = SpatialTarget::sky_fabric(64, 64);
    assert_eq!(sky.tile, TileShape::SCALAR);
    assert!(sky.provides_handshake(HandshakeKind::ValidReady));
    assert!(sky.matmul_supports(Precision::Int8) && sky.matmul_supports(Precision::Int64));

    // Every `modeled` name is a real field of the descriptor (reflection through the serialized form).
    for t in SpatialTarget::presets() {
        let json = serde_json::to_value(&t).unwrap();
        let text = json.to_string();
        for f in t.modeled {
            assert!(
                text.contains(&format!("\"{f}\"")),
                "{}: modeled field `{f}` is not a field of the descriptor",
                t.name
            );
        }
    }
}

#[test]
fn fabric_target_is_embedded_not_duplicated() {
    let sky = SpatialTarget::sky_fabric(8, 8);
    let embedded = sky
        .scalar_target()
        .expect("the fabric preset embeds its scalar datapath");
    assert_eq!(
        *embedded,
        FabricTarget::full(8, 8, 64),
        "the embedded descriptor IS the fabric lane's descriptor"
    );
    assert!(matches!(
        sky.compute
            .iter()
            .find(|c| matches!(c, ComputeCap::Scalar(_))),
        Some(_)
    ));
    let rdu = SpatialTarget::sn40l_rdu();
    assert_eq!(*rdu.scalar_target().unwrap(), FabricTarget::rdu_shaped());
    assert!(
        SpatialTarget::rtx3060().scalar_target().is_none(),
        "a GPU site has no CGRA datapath"
    );
}

/// ★ THE SCHEDULE IS THE PROGRAM THE STRUCTURAL PROOFS ARE ABOUT.
///
/// `validate_tensors` proves every output tile is stored exactly once ACROSS THE WORK LIST, and the
/// certificate's dry run trusts exactly that proof rather than re-checking coverage itself — it cannot,
/// because it executes only `certificate::item_window` items per site, so a tile beyond the window is
/// legitimately unwritten there. Both statements are about STAGES and ITEMS. Neither noticed when the
/// SCHEDULE did not issue the stage's data movement.
///
/// So deleting the `WriteTiles` from a store stage left a program the work list still vouched for:
/// coverage passed, the dry run declined to look, a certificate was issued, and a real evaluation
/// faulted with `OutputTileNeverStored`. `a_certificate_is_never_issued_to_a_program_whose_evaluation_faults`
/// found it at case 105 of its 300-program mutation sweep; this pins the property by name so the next
/// reader meets the invariant rather than a fuzz seed.
///
/// NON-VACUITY: proven by construction — the assertion below IS the mutation, and the same program
/// validates before it.
#[test]
fn a_store_stage_that_the_schedule_never_writes_is_not_a_valid_program() {
    for target in [SpatialTarget::wormhole(), SpatialTarget::blackhole()] {
        let p = compile_to_spatial(&gemm_program(128, 128, 64), &target).expect("a scheduled GEMM");
        assert!(
            p.validate().is_ok(),
            "{}: the unmutated program is valid",
            target.name
        );

        let store = p
            .stages
            .iter()
            .find(|s| matches!(s.kind, StageKind::Store { .. }))
            .expect("a store stage");
        let mut q = p.clone();
        let sch = q.schedule.as_mut().expect("scheduled");
        let mut removed = 0usize;
        for prog in &mut sch.programs {
            strip_writes(&mut prog.ops, store.id, &mut removed);
        }
        assert!(
            removed > 0,
            "{}: the mutation must actually remove a WriteTiles",
            target.name
        );

        match q.validate() {
            Err(IrViolation::StageNotScheduled { stage, op }) => {
                assert_eq!(stage, store.id);
                assert_eq!(op, "WriteTiles");
            }
            other => panic!(
                "{}: a store stage that writes nothing must be rejected by `validate`, not left for the \
                 certifier to vouch for — got {other:?}",
                target.name
            ),
        }
    }
}

/// Drop every `WriteTiles` tagged with `stage`, loop bodies included.
fn strip_writes(ops: &mut Vec<TaggedOp>, stage: StageId, removed: &mut usize) {
    for t in ops.iter_mut() {
        if let ChoreoOp::Loop { body, .. } = &mut t.op {
            strip_writes(body, stage, removed);
        }
    }
    let before = ops.len();
    ops.retain(|t| !(t.stage == stage && matches!(t.op, ChoreoOp::WriteTiles { .. })));
    *removed += before - ops.len();
}
