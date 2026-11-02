//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! SPATIAL IR → SPATIAL LANGUAGE LOCK — the verified spatial program printed in the RDU's own front-door
//! language (Spatial, Koeplinger et al. PLDI'18 — the Plasticine/RDU lineage), as a regression app the
//! public Spatial → PIR → Tungsten chain runs.
//!
//! `spatial_lang::emit_spatial_program` is the `SpatialProgram` twin of `codegen_sva::spatial_emit`
//! (which prints a `LoopFabric`). Every construct it prints is the image of an IR object, and this lock
//! counts them against the IR:
//!   * one `// stage <name>` controller per stage — a Load is a `load` into its buffer, a Store a `store`
//!     out of it, a `MatmulTiles` compute the `Foreach`/`Reduce` dot-product nest (a `MemReduce` when the
//!     IR spills partials through an `Interm` buffer), a `Generic` compute an element-wise `Foreach`
//!     whose reduce axis is a `Reduce`;
//!   * one memory per buffer — `SRAM[T]` (Spatial N-buffers it: the IR's depth is recorded beside it),
//!     `FIFO[T]` when the buffer's edge is a `Fifo` handshake;
//!   * one `DRAM[T]` per tensor, its tier named;
//!   * a host that fills every tensor from its recorded fill and checks the OUTPUT CHECKSUM against the
//!     gold — the spatial oracle's answer (`eval_choreography`) unless the caller hands over the VM's.
//! What the emitter does not spell — multicast groups, scalar bodies, the fabric epilogues, external
//! fills, weight-stationary dataflow, cross-site handshakes — it refuses BY NAME.
//!
//! ★ MUTATION PROBES (each run once against this tree; the named test went RED, the impl was restored
//! byte-identical — every line below is what the probe ACTUALLY did and what it ACTUALLY reported):
//!   - `spatial_lang.rs`: hand the host a constant gold (`None => Gold::Int(0)`) instead of the computed
//!     `checksum_gold(out_v)` → `a_verified_gemm_emits_spatial_source_that_names_every_stage_buffer_and_
//!     tier` fails on `the gold is the VM's checksum -7` (the emitted app would PASS against nothing);
//!   - `spatial_lang.rs`: make `refuse_unspelled` return `Ok(())` for everything →
//!     `constructs_the_emitter_does_not_spell_are_refused_by_name` fails on `the refusal must name
//!     \`multicast\`` (the wormhole GEMM's four multicast groups are printed instead of refused).
//!
//!   - `spatial_lang.rs`: print a flat read UNGUARDED (drop the off-page mux) → `a_window_before_its_page_reads_
//!     zero_through_a_guard` fails; `…reads_zero_in_spatials_simulator` stays green, because the emulator reads
//!     an off-page SRAM address as 0 by itself — a check that could not fail, found by this probe.
//!   - `spatial_lang.rs`: skip the `e^(a·ln b)` → `pow(b, a)` spelling → `a_power_through_ln_is_spelled_pow_on_
//!     the_fabric` fails (the decay's bare `ln` is refused on the fabric; before the spelling, Tungsten's C++ for the
//!     gated-delta segment did not compile).
//!   - `spatial_lang.rs` (2026-09-25, private copy): never nest the `Accel` sequence → `a_program_past_the_jvms_bounds_
//!     prints_in_chunks` fails on 130 calls in one block (the batched decode step's 320 hit Scala's 254-parameter
//!     limit); never use the DRAM registry → it fails on a `val d_… = DRAM[…]` in `main` (the batched step's 398 made
//!     `main` past the JVM's 64 KB).
//!
//! The container gate (`the_emitted_spatial_source_passes_pir_in_the_container`) is honestly tool-gated:
//! `docker` and the fetched+built toolchain tree (`LOGOS_TOOL_SKIP:` marker when absent, `LOGOS_REQUIRE_
//! TOOLS=docker,plasticine-toolchain` to fail loud). It is serialized in the `plasticine-chain` nextest
//! group: one chain, one build tree.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_ir_to_spatial_lang_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::codegen_sva::fabric_rtl::{AluOp, LoopCell, Src};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::ir::{
    BufferKind, ComputeOp, Handshake, IterKind, MemSpace, ReducePool, ScalarBody, SpatialProgram,
    StageKind, TensorInit,
};
use logicaffeine_compile::spatial::lower::spatial_lang::emit_spatial_program;
use logicaffeine_compile::spatial::lower::SpatialLowerError;
use logicaffeine_compile::spatial::pipeline::{compile_program_to_spatial, compile_to_spatial};
use logicaffeine_compile::spatial::target::{DataflowPolicy, SpatialTarget};
use spatial_support::*;

const SUGAR_PLUS: &str = r#"## Main
Let A be a 2 by 2 tensor of Int.
Set item 1 of A to 1.
Set item 2 of A to 2.
Set item 3 of A to 3.
Set item 4 of A to 4.
Let B be a 2 by 2 tensor of Int.
Set item 1 of B to 5.
Set item 2 of B to 6.
Set item 3 of B to 7.
Set item 4 of B to 8.
Let C be A plus B.
Show item 1 of C.
Show item 4 of C.
"#;

const SUGAR_RMSNORM: &str = r#"## Main
Let X be a 2 by 3 tensor of Real.
Set item 1 of X to 1.0.
Set item 2 of X to 2.0.
Set item 3 of X to 3.0.
Set item 4 of X to 4.0.
Set item 5 of X to 5.0.
Set item 6 of X to 6.0.
Let W be a 1 by 3 tensor of Real.
Set item 1 of W to 0.5.
Set item 2 of W to 1.0.
Set item 3 of W to 2.0.
Let Y be rmsnorm of X with weight W.
Show round(item 1 of Y * 1000000).
Show round(item 4 of Y * 1000000).
"#;

fn rdu() -> SpatialTarget {
    SpatialTarget::sn40l_rdu()
}

fn rdu_program(source: &str) -> SpatialProgram {
    compile_to_spatial(source, &rdu())
        .unwrap_or_else(|e| panic!("the program lowers on the RDU: {e:?}"))
}

fn gemm_64() -> SpatialProgram {
    let mut p = rdu_program(&gemm_program(64, 64, 64));
    p.name = "spatial_gemm_64".into();
    p
}

fn emit(p: &SpatialProgram) -> String {
    emit_spatial_program(p, &rdu(), None)
        .unwrap_or_else(|e| panic!("{} emits Spatial source: {e:?}", p.name))
}

fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

/// The controllers a stage spells: a `MatmulTiles` is the two-deep (row, column) `Foreach` nest around its
/// dot-product `Reduce`; a `Generic` is one element-wise `Foreach`, plus the broadcast write of its folded
/// scalar across the output tile when exactly one axis is reduced; a Load or Store whose buffer's edge is a
/// `Fifo` handshake streams one DRAM row per burst, which is one `Foreach` over the block's rows.
fn expected_foreach(p: &SpatialProgram) -> usize {
    let fifo_buffers: Vec<_> = p
        .edges
        .iter()
        .filter(|e| matches!(e.handshake, Handshake::Fifo))
        .map(|e| e.buffer)
        .collect();
    p.stages
        .iter()
        .map(|s| match &s.kind {
            StageKind::Compute(ComputeOp::MatmulTiles { .. }) => 2,
            StageKind::Compute(ComputeOp::Generic { iters, .. }) => {
                let reduced = iters
                    .iter()
                    .filter(|i| matches!(i.kind, IterKind::Reduce { .. }))
                    .count();
                // A space narrower than its output page zero-fills the page first, as the oracle's page starts.
                let out = p.buffer(s.outs[0]);
                let (rows, width) = (p.tile.rows, out.kind.page_tiles() * p.tile.cols);
                let folds = |k: usize| matches!(iters[k].kind, IterKind::Reduce { .. });
                let covered = match (folds(0), folds(1)) {
                    (false, false) => iters[0].extent >= rows && iters[1].extent >= width,
                    (false, true) => iters[0].extent >= rows,
                    (true, false) => iters[1].extent >= width,
                    (true, true) => true,
                };
                let zero_fill = (!covered && !fifo_buffers.contains(&out.id)) as usize;
                (if reduced == 1 { 2 } else { 1 }) + zero_fill
            }
            StageKind::Load { .. } => s.outs.iter().any(|b| fifo_buffers.contains(b)) as usize,
            StageKind::Store { .. } => s.ins.iter().any(|b| fifo_buffers.contains(b)) as usize,
            _ => 0,
        })
        .sum()
}

/// The `Reduce`s a program spells: one dot product per `MatmulTiles`, one fold per `Generic` reduce axis.
fn expected_reduce(p: &SpatialProgram) -> usize {
    p.stages
        .iter()
        .map(|s| match &s.kind {
            StageKind::Compute(ComputeOp::MatmulTiles { .. }) => 1,
            StageKind::Compute(ComputeOp::Generic { iters, .. }) => iters
                .iter()
                .any(|i| matches!(i.kind, IterKind::Reduce { .. }))
                as usize,
            _ => 0,
        })
        .sum()
}

fn expected_memreduce(p: &SpatialProgram) -> usize {
    p.stages
        .iter()
        .filter(|s| {
            matches!(
                &s.kind,
                StageKind::Compute(ComputeOp::MatmulTiles {
                    interm: Some(_),
                    ..
                })
            )
        })
        .count()
}

/// The structural bijection every emitted source must satisfy against its IR.
fn assert_names_every_object(p: &SpatialProgram, src: &str) {
    let class = format!(
        "Logos_{}",
        p.name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect::<String>()
    );
    assert!(
        src.contains("import spatial.dsl._"),
        "a Spatial app imports the DSL"
    );
    assert!(
        src.contains(&format!("@spatial class {class} extends SpatialTest")),
        "the regression-harness class is named after the program:\n{src}"
    );
    assert_eq!(count(src, "Accel {"), 1, "exactly one Accel block");
    for s in &p.stages {
        assert!(
            src.contains(&format!("// stage {}", s.name)),
            "stage {} is named in the source:\n{src}",
            s.name
        );
    }
    assert_eq!(
        count(src, "// stage "),
        p.stages.len(),
        "one controller marker per stage"
    );
    for b in &p.buffers {
        assert!(
            src.contains(&format!("// buffer {}", b.name)),
            "buffer {} is named in the source:\n{src}",
            b.name
        );
        let tier = match b.space {
            MemSpace::Local => "Local",
            MemSpace::Acc => "Acc",
            MemSpace::Pmu => "Pmu",
        };
        let depth = match b.kind {
            BufferKind::Fifo {
                depth_pages,
                page_tiles,
            } => format!("Fifo depth {depth_pages} pages x {page_tiles} tiles"),
            BufferKind::Interm {
                depth_pages,
                page_tiles,
            } => format!("Interm depth {depth_pages} pages x {page_tiles} tiles"),
            BufferKind::Accumulator { tiles } => format!("Accumulator {tiles} tiles"),
        };
        let marker = format!("// buffer {} {tier} {depth}", b.name);
        assert!(
            src.contains(&marker),
            "buffer {} names its tier and depth as `{marker}`:\n{src}",
            b.name
        );
    }
    let fifo_edges = p
        .edges
        .iter()
        .filter(|e| matches!(e.handshake, Handshake::Fifo))
        .count();
    assert_eq!(
        count(src, "= FIFO[T]("),
        fifo_edges,
        "one FIFO[T] per Fifo-handshake edge"
    );
    assert_eq!(
        count(src, "= SRAM[T]("),
        p.buffers.len() - fifo_edges,
        "one SRAM[T] per remaining buffer"
    );
    for t in &p.tensors {
        let marker = format!(
            "// tensor {} {:?} {}x{} {:?} {:?}",
            t.name, t.role, t.elems.rows, t.elems.cols, t.dtype, t.space
        );
        assert!(
            src.contains(&marker),
            "tensor {} names its role, shape, precision and tier as `{marker}`:\n{src}",
            t.name
        );
    }
    assert_eq!(
        count(src, "= DRAM[T]("),
        p.tensors.len(),
        "one DRAM[T] per tensor"
    );
    let loads = p
        .stages
        .iter()
        .filter(|s| matches!(s.kind, StageKind::Load { .. }))
        .count();
    let stores = p
        .stages
        .iter()
        .filter(|s| matches!(s.kind, StageKind::Store { .. }))
        .count();
    assert_eq!(count(src, " load "), loads, "one `load` per Load stage");
    assert_eq!(count(src, " store "), stores, "one `store` per Store stage");
    assert_eq!(
        count(src, "Foreach("),
        expected_foreach(p),
        "the Foreach controllers are exactly the compute stages' nests"
    );
    assert_eq!(
        count(src, "Reduce(Reg[T])"),
        expected_reduce(p),
        "one Reduce per dot product / reduce axis"
    );
    assert_eq!(
        count(src, "MemReduce("),
        expected_memreduce(p),
        "a MemReduce only where the IR spills partials"
    );
    assert!(src.contains("val gold = "), "the host carries a gold");
    assert!(
        src.contains(&format!("println(\"PASS: \" + cksum + \" ({class})\")")),
        "the harness's PASS line names the class"
    );
    assert!(
        src.contains("assert(cksum)"),
        "the harness asserts the checksum"
    );
    assert!(
        src.contains("Generated by LOGOS (logicaffeine_compile::spatial::lower::spatial_lang)"),
        "the provenance header"
    );
}

#[test]
fn a_verified_gemm_emits_spatial_source_that_names_every_stage_buffer_and_tier() {
    let p = gemm_64();
    let src = emit(&p);
    assert_names_every_object(&p, &src);
    assert!(
        src.contains("type T = Int"),
        "an int8 GEMM whose values fit 32 bits runs on Spatial's Int:\n{src}"
    );
    // The dot product reads the A block's row against the B block's column over the whole K block.
    assert!(
        src.contains("Reduce(Reg[T])(0 until 64"),
        "the dot product folds the 64-deep K block:\n{src}"
    );
    assert!(
        src.contains("in0_0_0(i, k) * in1_0_0(k, j)"),
        "the dot product's operands are the load buffers, row against column:\n{src}"
    );
    // The tensors are filled from the recognized fills and the output is checked by checksum against the
    // VM's answer — the gold is the spatial oracle's, and the oracle is the VM.
    assert!(
        src.contains("% 7) - 3).to[T]"),
        "A's fill (p % 7) - 3 is reproduced on the host:\n{src}"
    );
    assert!(
        src.contains("% 11) - 5).to[T]"),
        "B's fill (p % 11) - 5 is reproduced on the host:\n{src}"
    );
    let vm = vm_outcome(&gemm_program(64, 64, 64));
    assert!(vm.error.is_none(), "{:?}", vm.error);
    let checksum = shown_int(&vm.output);
    assert_eq!(
        checksum,
        spatial_support::checksum(&gemm_reference(64, 64, 64)),
        "the VM and the reference agree on the checksum"
    );
    assert!(
        src.contains(&format!("val gold = ({checksum}L: Long)")),
        "the gold is the VM's checksum {checksum}:\n{src}"
    );
    assert!(
        src.contains("val cksum = checksum == gold"),
        "an exact-numerics program compares exactly"
    );
    // The caller's gold overrides the oracle's, byte for byte in the one line.
    let handed = emit_spatial_program(&p, &rdu(), Some("12345")).unwrap();
    assert!(handed.contains("val gold = (12345L: Long)"));
    assert_eq!(handed.lines().count(), src.lines().count());
    assert_eq!(
        handed
            .lines()
            .zip(src.lines())
            .filter(|(a, b)| a != b)
            .count(),
        1,
        "only the gold line moves"
    );
    println!(
        "Spatial source for {}: {} lines, gold {checksum}\n{src}",
        p.name,
        src.lines().count()
    );
}

#[test]
fn elementwise_and_fused_row_emit_too() {
    let plus = rdu_program(SUGAR_PLUS);
    let src = emit(&plus);
    assert_names_every_object(&plus, &src);
    assert!(
        src.contains("type T = Float"),
        "bf16 elementwise runs on Spatial's Float:\n{src}"
    );
    assert!(
        src.contains("Foreach(0 until 32, 0 until 32)"),
        "the Generic's (rows, cols) iteration space:\n{src}"
    );
    assert!(
        src.contains("(in0_0_0(r, c) + in1_0_0(r, c))"),
        "the body `Add(Operand 0, Operand 1)` read through identity maps:\n{src}"
    );
    assert!(
        src.contains("val cksum = abs(checksum - gold) <="),
        "an approximate-numerics program compares within a tolerance"
    );
    assert!(
        src.contains("Array[T]("),
        "value fills are reproduced as a host array"
    );

    let rms = rdu_program(SUGAR_RMSNORM);
    let src = emit(&rms);
    assert_names_every_object(&rms, &src);
    assert!(src.contains("type T = Float"));
    assert!(
        src.contains("Reduce(Reg[T])(0 until 3)"),
        "the row reduce folds the 3 columns:\n{src}"
    );
    assert!(src.contains("{ _ + _ }"), "a Sum pool folds with +");
    assert!(
        src.contains("sqrt("),
        "the rsqrt transform is spelled through sqrt:\n{src}"
    );
    assert!(
        src.contains("(r, 0)"),
        "a per-row scalar operand reads column 0 (the absent axis is a broadcast):\n{src}"
    );
    assert!(
        src.contains("(0, c)"),
        "a per-column weight reads row 0 (the absent axis is a broadcast):\n{src}"
    );
    println!(
        "elementwise: {} lines; rmsnorm: {} lines",
        emit(&plus).lines().count(),
        src.lines().count()
    );

    // The same elementwise on a Fifo-handshake target: every buffer becomes a FIFO, read and written in
    // stream order, loaded and stored one DRAM row per burst.
    let wh = SpatialTarget::wormhole();
    let streamed = compile_to_spatial(SUGAR_PLUS, &wh).unwrap();
    assert!(
        streamed
            .edges
            .iter()
            .all(|e| matches!(e.handshake, Handshake::Fifo)),
        "control: wormhole's local edges are Fifo handshakes"
    );
    assert!(
        streamed.placement.as_ref().unwrap().groups.is_empty(),
        "control: no multicast on a one-tile elementwise"
    );
    let src = emit_spatial_program(&streamed, &wh, None)
        .unwrap_or_else(|e| panic!("the Fifo-handshake elementwise emits: {e:?}"));
    assert_names_every_object(&streamed, &src);
    assert_eq!(
        count(&src, "= FIFO[T]("),
        streamed.buffers.len(),
        "every buffer is a FIFO"
    );
    assert_eq!(
        count(&src, ".deq()"),
        2,
        "both operands are dequeued once per element"
    );
    assert_eq!(
        count(&src, ".enq("),
        1,
        "the result is enqueued once per element"
    );
    assert!(
        src.contains("(in0_0_0.deq() + in1_0_0.deq())"),
        "the body over the dequeued operands:\n{src}"
    );
}

#[test]
fn constructs_the_emitter_does_not_spell_are_refused_by_name() {
    fn refused(r: Result<String, SpatialLowerError>, must_name: &str) -> String {
        match r {
            Err(SpatialLowerError::Unsupported(why)) => {
                assert!(
                    why.to_lowercase().contains(must_name),
                    "the refusal must name `{must_name}`: {why}"
                );
                why.to_string()
            }
            Ok(src) => panic!("must be refused by name (`{must_name}`), was printed:\n{src}"),
            Err(other) => panic!("must be refused Unsupported naming `{must_name}`, got {other:?}"),
        }
    }
    // Multicast groups: the wormhole GEMM fuses readers across a 2x2 mesh.
    let fused = compile_to_spatial(&gemm_program(64, 64, 64), &SpatialTarget::wormhole()).unwrap();
    assert!(
        fused.placement.as_ref().unwrap().groups.len() > 0,
        "control: the wormhole GEMM has multicast groups"
    );
    refused(
        emit_spatial_program(&fused, &SpatialTarget::wormhole(), None),
        "multicast",
    );

    // A scalar body (the CGRA's LoopCell datapath) in place of the elementwise Generic.
    let mut scalar = rdu_program(SUGAR_PLUS);
    let compute = scalar
        .stages
        .iter()
        .position(|s| matches!(s.kind, StageKind::Compute(_)))
        .unwrap();
    scalar.stages[compute].kind = StageKind::Scalar(ScalarBody {
        n_in: 2,
        cells: vec![LoopCell::Arith {
            op: AluOp::Add,
            a: Src::In(0),
            b: Src::In(1),
        }],
        result: Src::Val(0),
    });
    refused(emit_spatial_program(&scalar, &rdu(), None), "scalar");

    // The fabric's fused epilogues.
    for (op, name) in [
        (ComputeOp::Relu, "relu"),
        (ComputeOp::Requant { m: 1, s: 0 }, "requant"),
    ] {
        let mut epi = rdu_program(SUGAR_PLUS);
        epi.stages[compute].kind = StageKind::Compute(op);
        refused(emit_spatial_program(&epi, &rdu(), None), name);
    }

    // A tensor without a deterministic fill has no host data to fill DRAM with.
    let mut external = gemm_64();
    external.tensors[0].init = TensorInit::External;
    refused(emit_spatial_program(&external, &rdu(), None), "fill");

    // Weight-stationary dataflow keeps the weight block resident across output blocks — a different nest.
    let mut ws = gemm_64();
    for s in &mut ws.stages {
        if let StageKind::Compute(ComputeOp::MatmulTiles { dataflow, .. }) = &mut s.kind {
            *dataflow = DataflowPolicy::WeightStationary;
        }
    }
    refused(emit_spatial_program(&ws, &rdu(), None), "weight");

    // An unverified program never reaches the printer.
    let mut unverified = gemm_64();
    unverified.certificate = None;
    assert!(matches!(
        emit_spatial_program(&unverified, &rdu(), None),
        Err(SpatialLowerError::NotVerified { .. })
    ));
}

#[test]
fn the_spatial_source_is_a_function_of_the_ir() {
    let p = gemm_64();
    let a = emit(&p);
    let b = emit(&p);
    assert_eq!(a, b, "the same program prints the same bytes");
    let mut q = rdu_program(&gemm_program(128, 128, 128));
    q.name = "spatial_gemm_128".into();
    let c = emit(&q);
    assert_ne!(a, c, "a different tiling prints differently");
    assert!(c.contains("class Logos_spatial_gemm_128"));
    assert!(
        c.contains("Reduce(Reg[T])(0 until 128"),
        "the 128-deep K block:\n{c}"
    );
    // The name is the only thing a rename moves.
    let mut renamed = p.clone();
    renamed.name = "spatial_gemm_64_again".into();
    let d = emit(&renamed);
    assert_eq!(
        d.replace("spatial_gemm_64_again", "spatial_gemm_64"),
        a,
        "a rename changes the class name and nothing else"
    );

    // The SN50 descriptor: the same computation compiled for the day's chip prints, names its target, and
    // carries the same gold — the checksum is a function of the computation, not of the chip.
    let sn50 = SpatialTarget::sn50_rdu();
    let mut q50 = compile_to_spatial(&gemm_program(64, 64, 64), &sn50)
        .unwrap_or_else(|e| panic!("the GEMM lowers on sn50: {e:?}"));
    q50.name = "spatial_gemm_64".into();
    let s50 = emit_spatial_program(&q50, &sn50, None).unwrap();
    assert_names_every_object(&q50, &s50);
    assert!(
        s50.lines().take(4).any(|l| l.contains("sn50-rdu")),
        "the header names the target it was compiled for:\n{s50}"
    );
    let gold_line = |src: &str| {
        src.lines()
            .find(|l| l.contains("val gold = "))
            .unwrap()
            .to_string()
    };
    assert_eq!(
        gold_line(&s50),
        gold_line(&a),
        "the gold is the computation's checksum on either chip"
    );
}

/// The gate: the emitted app runs through Spatial → PIR → plastiroute → Tungsten in the container and the
/// harness prints `PASS: true (Logos_spatial_gemm_64)` — the simulated RDU computed the VM's checksum.
#[test]
fn the_emitted_spatial_source_passes_pir_in_the_container() {
    if !common::require_tool("docker") {
        return;
    }
    if !common::require_tool("plasticine-toolchain") {
        return;
    }
    let p = gemm_64();
    let src = emit(&p);
    let regression = common::plasticine::regression_dir();
    std::fs::write(regression.join("Logos_spatial_gemm_64.scala"), &src)
        .expect("write the regression app");
    std::fs::write(
        regression.join("Logos_spatial_gemm_64.lg"),
        gemm_program(64, 64, 64),
    )
    .expect("write the LOGOS source beside it");
    let run = common::plasticine::run_gate("Logos_spatial_gemm_64");
    println!("gate log: {}", run.log_path.display());
    assert!(
        run.log.contains("PASS: true (Logos_spatial_gemm_64)"),
        "the chain must print the harness's PASS line — log at {}:\n{}",
        run.log_path.display(),
        run.tail(60)
    );
    let cycles = run
        .tungsten_cycles()
        .expect("Tungsten reports `Simulation complete at cycle N`");
    println!("Logos_spatial_gemm_64: PASS, Tungsten cycles {cycles}");
    println!(
        "ledger: python3 scripts/accel/plasticine-gate-ledger.py add --kind gemm --app Logos_spatial_gemm_64 --app-source \"emitted from LOGOS by spatial_lang::emit_spatial_program (Logos_spatial_gemm_64.lg, the verified spatial IR on sn40l-rdu)\" --gen-dir target/plasticine-toolchain/spatial/gen/Tst_pirTest/Logos_spatial_gemm_64 --shape 64x64x64 --params \"ts=32 its=64 ip=16 loop_ii=1 loop_jj=1 loop_kk=1 loop_i=1 loop_j=1\" --toolchain \"…\""
    );
}

// ── The constructs a composed decode step needs ─────────────────────────────────────────────────────

/// A source whose `Real` tensors, declared `1 by n` in `decls`, are filled deterministically, followed by `body`.
fn filled(decls: &[(&str, usize, usize)], body: &str) -> String {
    let mut s = String::from("## Main\n");
    for (name, rows, cols) in decls {
        s.push_str(&format!(
            "Let {name} be a {rows} by {cols} tensor of Real.\n"
        ));
        for i in 0..rows * cols {
            s.push_str(&format!(
                "Set item {} of {name} to {:?}.\n",
                i + 1,
                ((i * 7 + name.len()) % 11) as f64 / 8.0 - 0.625
            ));
        }
    }
    s.push_str(body);
    s
}

fn composed(source: &str) -> SpatialProgram {
    let pl = compile_program_to_spatial(source, &rdu()).expect("a tensor program");
    pl.whole()
        .unwrap_or_else(|| panic!("the program lowers whole: {}", pl.refusal()))
        .clone()
}

fn generic_maps(p: &SpatialProgram) -> Vec<logicaffeine_compile::spatial::ir::ElemAccess> {
    p.stages
        .iter()
        .filter_map(|s| match &s.kind {
            StageKind::Compute(ComputeOp::Generic { ins, .. }) => Some(ins.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

/// ── A COMPOSED PROGRAM'S SECTIONS PRINT IN ORDER, AS ONE `Sequential`: the IR's rendezvous holds section k+1
/// until section k is done, and a parallel controller orders no DRAM store before a DRAM load.
#[test]
fn a_composed_program_prints_its_sections_in_order_as_one_sequence() {
    let p = composed(&filled(
        &[("A", 1, 32), ("B", 32, 32), ("E", 1, 32)],
        "Let C be A matmul B.\nLet D be C plus E.\n",
    ));
    assert!(
        p.sections.len() >= 2,
        "the program composes sections: {}",
        p.sections.len()
    );
    let src = emit(&p);
    assert!(
        src.contains("Sequential {"),
        "the sections run as a sequence:\n{src}"
    );
    let markers: Vec<usize> = (0..p.sections.len())
        .map(|k| {
            src.find(&format!("// section {k}: "))
                .unwrap_or_else(|| panic!("section {k} is marked"))
        })
        .collect();
    assert!(
        markers.windows(2).all(|w| w[0] < w[1]),
        "the sections print in order"
    );
    for (k, at) in markers.iter().enumerate() {
        let end = markers.get(k + 1).copied().unwrap_or(src.len());
        for st in p
            .stages
            .iter()
            .filter(|st| p.section_of_stage(st.id) == Some(k))
        {
            let at_stage = src
                .find(&format!("// stage {} (", st.name))
                .unwrap_or_else(|| panic!("stage {} is printed", st.name));
            assert!(
                *at < at_stage && at_stage < end,
                "stage {} prints inside section {k}",
                st.name
            );
        }
    }
}

/// ── A PROGRAM PAST THE JVM'S BOUNDS PRINTS IN CHUNKS. Spatial writes a controller's block as one parameter list
/// (the JVM caps one at 254) and the host as one method (capped at 64 KB, a few hundred bytes a DRAM): 130 sections
/// nest their sequence — the `Accel`'s `Sequential` holds at most 128 calls, and every section is called exactly
/// once, in order — and a host past 128 tensors makes its DRAMs in registry methods of at most 64, every tensor an
/// accessor into one, none of them a `val` in `main`. (The batched decode step, 320 sections and 398 tensors, is the
/// app this runs whole in Spatial's simulator: `qwen38_batched_decode_lock`.)
#[test]
fn a_program_past_the_jvms_bounds_prints_in_chunks() {
    let mut body = String::new();
    let mut prev = "x".to_string();
    for k in 1..=130 {
        body.push_str(&format!("Let y{k} be {prev} plus x.\n"));
        prev = format!("y{k}");
    }
    let p = composed(&filled(&[("x", 1, 8)], &body));
    assert!(
        p.sections.len() > 128 && p.tensors.len() > 128,
        "the program is past both bounds: {} sections, {} tensors",
        p.sections.len(),
        p.tensors.len()
    );
    let src = emit(&p);
    let lines: Vec<&str> = src.lines().map(str::trim).collect();
    let calls: Vec<usize> = (0..p.sections.len())
        .map(|k| {
            let at: Vec<usize> = lines
                .iter()
                .enumerate()
                .filter(|(_, l)| **l == format!("section_{k}()"))
                .map(|(i, _)| i)
                .collect();
            assert_eq!(at.len(), 1, "section {k} is called exactly once");
            at[0]
        })
        .collect();
    assert!(
        calls.windows(2).all(|w| w[0] < w[1]),
        "the sections are called in order"
    );
    let accel = lines
        .iter()
        .position(|l| *l == "Accel {")
        .expect("an Accel block");
    let accel_calls: Vec<&&str> = lines[accel..]
        .iter()
        .skip(2)
        .take_while(|l| **l != "}")
        .collect();
    assert!(
        !accel_calls.is_empty() && accel_calls.len() <= 128,
        "the Accel's sequence holds at most 128 calls: {}",
        accel_calls.len()
    );
    assert!(
        accel_calls.iter().all(|c| c.starts_with("run_")),
        "and they are the nested chunks: {accel_calls:?}"
    );
    assert!(
        !src.lines()
            .any(|l| l.trim_start().starts_with("val d_") && l.contains("= DRAM[")),
        "no DRAM is a `val` of `main`"
    );
    let registries: Vec<usize> = src
        .split("def dram_")
        .skip(1)
        .map(|chunk| {
            chunk
                .split(")\n")
                .next()
                .unwrap_or("")
                .matches("DRAM[")
                .count()
        })
        .collect();
    assert!(
        !registries.is_empty() && registries.iter().all(|n| *n <= 64),
        "the DRAMs come from registry methods of at most 64: {registries:?}"
    );
    assert_eq!(
        registries.iter().sum::<usize>(),
        p.tensors.len(),
        "every tensor's DRAM is made by a registry"
    );
    for t in &p.tensors {
        assert!(
            src.contains(&format!("def d_{}: DRAM2[", t.name)),
            "`{}` is an accessor into its registry",
            t.name
        );
    }
}

/// ── A PROGRAM OF FLOATS AND INTEGERS DECLARES EACH MEMORY IN ITS OWN CLASS: the embedding row gather reads a
/// float table at an address computed from an integer token.
#[test]
fn a_program_of_floats_and_integers_declares_each_memory_in_its_class() {
    let p = composed(&filled(
        &[("Emb", 4, 8)],
        "Let tok be 3.\nLet x be row tok of Emb.\n",
    ));
    assert!(
        generic_maps(&p)
            .iter()
            .any(|m| m.indirect.is_some_and(|i| i.stride != 0)),
        "the row read is a gather"
    );
    let src = emit(&p);
    assert!(
        src.contains("type T = Float") && src.contains("type I = Int"),
        "a float scalar and an integer scalar"
    );
    let decl = |name: &str, ty: &str| {
        let t = p
            .tensors
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("`{name}` is a tensor"));
        format!(
            "val d_{name} = DRAM[{ty}]({}, {})",
            t.tiles.rows_t * p.tile.rows,
            t.tiles.cols_t * p.tile.cols
        )
    };
    assert!(
        src.contains(&decl("tok", "I")),
        "the token is an integer memory:\n{src}"
    );
    assert!(
        src.contains(&decl("Emb", "T")),
        "the table a float one:\n{src}"
    );
    assert!(
        src.contains("val ix") && src.contains(".to[Int]"),
        "the gather reads at an address computed from data"
    );
}

/// ── A WINDOW THAT STARTS BEFORE ITS PAGE READS ZERO THERE, THROUGH A GUARD — as the oracle reads it. Rope reads
/// each element's partner `half` columns away, before the page for the first half.
#[test]
fn a_window_before_its_page_reads_zero_through_a_guard() {
    let p = composed(&filled(
        &[("Q", 1, 8), ("RC", 1, 2), ("RS", 1, 2)],
        "Let Qr be rope of Q with cos RC and sin RS across 2 heads.\n",
    ));
    assert!(
        generic_maps(&p)
            .iter()
            .any(|m| m.base < 0 || m.terms.iter().any(|(_, s)| *s < 0)),
        "rope reads a signed window"
    );
    let src = emit(&p);
    assert!(
        src.contains(">= 0, mux(") && src.contains("0.0.to[T]), 0.0.to[T])"),
        "the read is muxed to 0 off the page:\n{src}"
    );
}

/// ── AN ARG-MAX IS THE VM'S SCAN: column 0 seeds the best, a later column replaces it only when strictly greater.
/// For the simulator it prints as that scan — `Sequential`, because it carries its best from column to column. For
/// the fabric it prints as two plain folds, because PIR maps a register only as an accumulation (the scan's pair of
/// mux-updated registers is its "unexpected accum pattern"): `m = max w` and `first = min { c : w_c = m }`, with `w`
/// the values and a NaN replaced by the seed, and 0 when the seed itself is NaN — the same scan.
#[test]
fn an_argmax_prints_the_seeded_scan() {
    use logicaffeine_compile::spatial::lower::spatial_lang::{
        emit_spatial_program_checked, SpatialCheck, SpatialFlavor,
    };
    let p = composed(&filled(&[("L", 1, 40)], "Let t be argmax of L.\n"));
    assert!(
        p.stages.iter().any(|s| matches!(&s.kind, StageKind::Compute(ComputeOp::Generic { iters, .. }) if iters.iter().any(|it| it.kind == IterKind::Reduce { pool: ReducePool::ArgMax }))),
        "the program folds an index"
    );
    let check = [SpatialCheck {
        tensor: "t".into(),
        offset: 0,
        gold: vec![1.0],
        rel_tol: 0.0,
    }];
    let sim = emit_spatial_program_checked(&p, &rdu(), &check, SpatialFlavor::Simulator)
        .expect("the arg-max prints for the simulator");
    assert!(
        sim.contains("Sequential.Foreach(0 until 40)"),
        "a sequential scan over the 40 columns:\n{sim}"
    );
    assert!(
        sim.contains("best := mux(c < 1, v, mux(ahead, v, best.value))"),
        "seeded by column 0, replaced only when ahead"
    );
    assert!(
        sim.contains("at := mux(c < 1, c.to[I], mux(ahead, c.to[I], at.value))"),
        "the index follows the best"
    );
    let fabric = emit(&p);
    assert!(
        !fabric.contains("best := "),
        "no register scan on the fabric:\n{fabric}"
    );
    assert!(
        fabric.contains("mux(v === v, v, v0)") && fabric.contains("{ (a, b) => max(a, b) }"),
        "the maximum of the values, a NaN read as the seed:\n{fabric}"
    );
    assert!(
        fabric.contains("=== m.value, c, 40.to[I32])")
            && fabric.contains("{ (a, b) => min(a, b) }"),
        "the first column holding it"
    );
    assert!(
        fabric.contains("mux(v0 === v0, first.value, 0.to[I32])"),
        "and column 0 when the seed is NaN"
    );
}

/// ── AND IN SPATIAL'S SIMULATOR A READ BEFORE THE PAGE IS 0, AS THE ORACLE READS IT: a bare shift `y[c] = x[c − 2]`
/// uses the off-page read itself (rope and the conv ring read off the page only on a `Select`'s discarded branch).
/// This pins the VALUE. It cannot pin the guard: Spatial's emulator reads an SRAM address before the page as 0 on
/// its own (measured — the app passes with the guard removed). On hardware that address is undefined, which is
/// why the guard is printed; `a_window_before_its_page_reads_zero_through_a_guard` pins its spelling.
#[test]
fn a_window_before_its_page_reads_zero_in_spatials_simulator() {
    use logicaffeine_compile::codegen_sva::fabric_target::Precision;
    use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
    use logicaffeine_compile::spatial::ir::{
        AccPolicy, ElemAccess, IterSpec, NumericsClaim, TensorBody, TensorRole, TileRect, WorkItem,
        WorkItemId,
    };
    use logicaffeine_compile::spatial::lower::spatial_lang::{
        emit_spatial_program_checked, SpatialCheck, SpatialFlavor,
    };
    use logicaffeine_compile::spatial::passes::from_tensor_op::{ident_map, Template};
    use logicaffeine_compile::spatial::passes::{LV_GEN_COL, LV_GEN_ROW};
    use logicaffeine_compile::spatial::pipeline::run_passes;
    if !common::require_tool("docker") || !common::require_tool("plasticine-toolchain") {
        return;
    }
    let target = rdu();
    let w = target.tile.cols;
    let x: Vec<f64> = (0..w).map(|c| c as f64 / 4.0 + 1.0).collect();
    let mut tp = Template::new(&target).expect("a template builder");
    let xt = tp.tensor(
        "x",
        TensorRole::Input,
        1,
        w,
        Precision::Fp32,
        TensorInit::FloatValues(x.iter().map(|v| v.to_bits()).collect()),
    );
    let xb = tp.buffer("in_x", 1, Precision::Fp32);
    let l = tp.load("load_x", xt, vec![], 1, xb);
    let yt = tp.tensor(
        "y",
        TensorRole::Output,
        1,
        w,
        Precision::Fp32,
        TensorInit::Zero,
    );
    let yb = tp.buffer("out", 1, Precision::Fp32);
    let iters = vec![
        IterSpec {
            var: LV_GEN_ROW,
            extent: 1,
            kind: IterKind::Parallel,
        },
        IterSpec {
            var: LV_GEN_COL,
            extent: w,
            kind: IterKind::Parallel,
        },
    ];
    let shifted = ElemAccess {
        base: -2,
        terms: vec![(LV_GEN_COL, 1)],
        indirect: None,
    };
    let g = tp.stage(
        "shift",
        StageKind::Compute(ComputeOp::Generic {
            iters,
            ins: vec![shifted],
            out: ident_map(w),
            body: TensorBody::Operand(0),
        }),
        vec![xb],
        vec![yb],
    );
    let st = tp.store("store_y", yt, vec![], 1, yb);
    tp.edge(l, xb, g);
    tp.edge(g, yb, st);
    let items = vec![WorkItem {
        id: WorkItemId::new(0),
        coord: (0, 0),
        out: TileRect {
            row0_t: 0,
            col0_t: 0,
            h_t: 1,
            w_t: 1,
        },
    }];
    let p = tp
        .finish(
            "window_shift".into(),
            items,
            AccPolicy::F32,
            NumericsClaim::Exact,
            &target,
        )
        .expect("the shift validates");
    let p = run_passes(p, &target).expect("the shift verifies");
    let want: Vec<f64> = (0..w)
        .map(|c| if c >= 2 { x[c - 2] } else { 0.0 })
        .collect();
    let mut inputs = Inputs::default();
    inputs.tensors.insert(xt, Value::Float(x.clone()));
    let oracle = eval_choreography(&p, &inputs).expect("the oracle runs the shift");
    assert_eq!(
        oracle.tensors.get(&yt),
        Some(&Value::Float(want.clone())),
        "the oracle reads 0 before the page"
    );
    let app = emit_spatial_program_checked(
        &p,
        &target,
        &[SpatialCheck {
            tensor: "y".into(),
            offset: 0,
            gold: want,
            rel_tol: 1e-6,
        }],
        SpatialFlavor::Simulator,
    )
    .expect("the shift prints");
    let class = app
        .split("@spatial class ")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .expect("the app's class")
        .to_string();
    std::fs::write(
        common::plasticine::regression_dir().join(format!("{class}.scala")),
        &app,
    )
    .expect("write the regression app");
    let run = common::plasticine::run_sim(&class);
    assert!(
        run.passed(),
        "Spatial's simulator runs the shift and its host check passes — log at {}:\n{}",
        run.log_path.display(),
        run.tail(60)
    );
}

/// ── ON THE FABRIC, `e^(a·ln b)` IS SPELLED `pow(b, a)`, AND A BARE `ln` IS REFUSED BY NAME. The chain's Tungsten
/// backend prints `ln` as a bare `log(…)` that its contexts shadow: the generated C++ does not compile (measured on
/// Gated DeltaNet's decay, `e^(A·ln(1 + e^u))`). `1 + e^u` is positive by structure, so the power is the same value.
#[test]
fn a_power_through_ln_is_spelled_pow_on_the_fabric() {
    use logicaffeine_compile::spatial::lower::spatial_lang::{
        emit_spatial_program_checked, SpatialCheck, SpatialFlavor,
    };
    let p = composed(&filled(
        &[("A", 1, 4), ("D", 1, 4), ("R", 1, 4)],
        "Let y be decay of A with bias D rate R.\n",
    ));
    let has_ln =
        |b: &logicaffeine_compile::spatial::ir::TensorBody| format!("{b:?}").contains("Ln");
    assert!(
        p.stages.iter().any(|s| matches!(&s.kind, StageKind::Compute(ComputeOp::Generic { body, .. }) if has_ln(body))),
        "the decay's body takes a natural log"
    );
    let y = p.tensors.iter().find(|t| t.name == "y").expect("y");
    let check = [SpatialCheck {
        tensor: "y".into(),
        offset: 0,
        gold: vec![0.0; y.elems.cols],
        rel_tol: 1.0,
    }];
    let fabric = emit_spatial_program_checked(&p, &rdu(), &check, SpatialFlavor::Fabric)
        .expect("the decay prints for the fabric");
    // `ln` as a call of its own — not the tail of `println(`.
    let calls_ln = |src: &str| {
        src.match_indices("ln(")
            .any(|(i, _)| i == 0 || !src.as_bytes()[i - 1].is_ascii_alphanumeric())
    };
    assert!(
        fabric.contains("pow(") && !calls_ln(&fabric),
        "the decay is a power on the fabric, never an `ln`"
    );

    // A bare `ln` has no such spelling: refused by name on the fabric, printed for the simulator.
    use logicaffeine_compile::codegen_sva::fabric_target::Precision;
    use logicaffeine_compile::spatial::ir::{
        AccPolicy, IterSpec, NumericsClaim, SfpuKind, TensorBody, TensorRole, TileRect, WorkItem,
        WorkItemId,
    };
    use logicaffeine_compile::spatial::passes::from_tensor_op::{ident_map, Template};
    use logicaffeine_compile::spatial::passes::{LV_GEN_COL, LV_GEN_ROW};
    let target = rdu();
    let w = target.tile.cols;
    let mut tp = Template::new(&target).expect("a template builder");
    let xt = tp.tensor(
        "x",
        TensorRole::Input,
        1,
        w,
        Precision::Fp32,
        TensorInit::FloatValues((0..w).map(|c| (c as f64 + 1.0).to_bits()).collect()),
    );
    let xb = tp.buffer("in_x", 1, Precision::Fp32);
    let l = tp.load("load_x", xt, vec![], 1, xb);
    let yt = tp.tensor(
        "y",
        TensorRole::Output,
        1,
        w,
        Precision::Fp32,
        TensorInit::Zero,
    );
    let yb = tp.buffer("out", 1, Precision::Fp32);
    let iters = vec![
        IterSpec {
            var: LV_GEN_ROW,
            extent: 1,
            kind: IterKind::Parallel,
        },
        IterSpec {
            var: LV_GEN_COL,
            extent: w,
            kind: IterKind::Parallel,
        },
    ];
    let body = TensorBody::Unary {
        kind: SfpuKind::Ln,
        arg: Box::new(TensorBody::Operand(0)),
    };
    let g = tp.stage(
        "ln",
        StageKind::Compute(ComputeOp::Generic {
            iters,
            ins: vec![ident_map(w)],
            out: ident_map(w),
            body,
        }),
        vec![xb],
        vec![yb],
    );
    let st = tp.store("store_y", yt, vec![], 1, yb);
    tp.edge(l, xb, g);
    tp.edge(g, yb, st);
    let items = vec![WorkItem {
        id: WorkItemId::new(0),
        coord: (0, 0),
        out: TileRect {
            row0_t: 0,
            col0_t: 0,
            h_t: 1,
            w_t: 1,
        },
    }];
    let bare = tp
        .finish(
            "bare_ln".into(),
            items,
            AccPolicy::F32,
            NumericsClaim::Exact,
            &target,
        )
        .expect("the ln validates");
    let bare = logicaffeine_compile::spatial::pipeline::run_passes(bare, &target)
        .expect("the ln verifies");
    let check = [SpatialCheck {
        tensor: "y".into(),
        offset: 0,
        gold: (0..w).map(|c| (c as f64 + 1.0).ln()).collect(),
        rel_tol: 1e-6,
    }];
    match emit_spatial_program_checked(&bare, &target, &check, SpatialFlavor::Fabric) {
        Err(SpatialLowerError::Unsupported(why)) => {
            assert!(why.contains("natural log"), "refused by name: {why}")
        }
        other => panic!("a bare ln is refused on the fabric, got {other:?}"),
    }
    let sim = emit_spatial_program_checked(&bare, &target, &check, SpatialFlavor::Simulator)
        .expect("a bare ln prints for the simulator");
    assert!(calls_ln(&sim), "the simulator spells it `ln`");
}
