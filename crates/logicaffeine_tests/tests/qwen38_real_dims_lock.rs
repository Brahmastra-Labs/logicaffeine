//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! QWEN 3.8 AT ITS REAL DIMENSIONS — because the miniature hid two walls.
//!
//! `qwen38_rdu_lock` proves the whole authored forward composes, certifies and equals the VM. Every one
//! of its assertions runs on `Qwen38Cfg::tiny()` — 4 layers, `d = 8`, `vocab = 32`. That config is
//! structurally faithful: same op sequence, same hybrid schedule. It is not dimensionally faithful, and
//! the difference is not cosmetic. Measured on `Qwen38Cfg::real()` (64 layers, `d = 5120`,
//! `dff = 17408`, `vocab = 248320`):
//!
//!   * the whole forward is **OOM-killed** (exit 137) before it finishes lowering;
//!   * at ONE layer with real widths, only **16 of 28** statements lower. The twelve refusals are
//!     `AccTilesExceeded { needed: 160, available: 8 }` and
//!     `NoLegalTiling { rows_t: 1, cols_t: 256, acc_tiles: 8 }` — every row-wise op (RMSNorm, L2Norm,
//!     SiLU, causal conv, gated delta) demands the ENTIRE row in the accumulator at once. At `d = 8`
//!     that is one tile and always fits. At `d = 5120` it is 160 tiles against a budget of 8.
//!
//! ★ WHY THIS FILE EXISTS. "We lower Qwen 3.8" was true of a toy and false of the model, and nothing in
//! the suite could tell the difference, because every lock chose the config that fits. A dimension the
//! test suite never reaches is a dimension the compiler has never been asked about. These assertions are
//! written at the REAL widths so the claim we make to a silicon vendor is the claim the tests check.
//!
//! The per-layer tests use ONE layer at full width deliberately: layer count multiplies work linearly and
//! is not where the wall is, whereas `d`, `dff` and `vocab` are. `the_whole_real_forward_lowers` is the
//! end-to-end statement and is `#[ignore]`d only for RUNTIME, never to hide a red — the ignored form is
//! run by the campaign gate.
//!
//! ★ THE DEPLOYMENT (DEPLOY-QWEN38 of `benchmarks/PREPARE_FOR_SAMBA.md`): Qwen 3.5 and Qwen 3.8 (`qwen35`, `qwen35moe`)
//! on the SN50 through the generic path. Qwen 3.5's tiny twin (its hybrid of Gated DeltaNet and gated full attention, a
//! quarter of each head rotated) brings up from its Hugging Face directory and computes what transformers, the f64
//! reference and the VM compute (`common/samba_deploy.rs`; its mixture's twin, Qwen's experts beside a shared expert
//! under its gate, is a handed RED of `deploy_reds_lock`, NUM-1's); and every size Qwen publishes, each its own
//! `config.json`, lowers at its real widths for the SN50 one program per layer kind, each certified — the mixtures'
//! router rows of 256 and 512 experts in blocks of the row, top 8 and top 10.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test qwen38_real_dims_lock`
//!
//! ★ MUTATION PROBE: set `Qwen38Cfg::real().d` to 8 in `real_width_layer` — `every_statement_of_a_real_
//! width_layer_lowers` goes green while proving nothing, which is precisely the failure this file is a
//! response to. The assertion messages name the measured refusal so a regression reports the wall it hit.
//!
//! ★ MUTATION PROBE (2026-09-25, private copy): lower every multi-row norm in the shape the program binds its input
//! at, instead of the layout the program's other ops agree on — `the_blocked_per_head_rmsnorm_computes_what_the_vm_
//! computes` reds (a `4 × 256` input normalized across its 4 heads takes the one-item matrix form, not the blocked
//! one), which is how the stacked-batch layout rule was found to need the agreement.
//!
//! ★ MUTATION PROBES (2026-09-27, private copy; logs `logs/sn50/probe_realdims_*.txt`): the stored set ignored → `Wd0`
//! (17408 × 5120, whole K blocks and tiles) stays dense and `a_real_width_decode_layer_reads_its_q8_weights_as_stored`
//! reds; every product reshaping its own view of the activation → the layer is refused at composition
//! (`OutputOverlap` on the view, two writers of one activation) — the same test reds. Across the node: a Q8_0 plane
//! left unplaced (`chip_of` by name only) → `Wd0_c0`'s planes on no chip; one view of a replicated activation shared by
//! every chip's products → `xn0__q8_rows1` read on chip 1, living on chip 0 — the tensor-parallel test reds on each.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]
#![cfg(feature = "codegen")]

mod common;
#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/samba_bringup.rs"]
mod samba_bringup;
#[path = "common/model_harness.rs"]
mod model_harness;
#[path = "common/samba_deploy.rs"]
mod samba_deploy;

use logicaffeine_compile::codegen_cuda::qwen38::{gen_qwen38_lg, Qwen38Cfg};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::model::open_checkpoint;
use logicaffeine_compile::model::spec::ModelSpec;
use samba_bringup::every;
use samba_deploy::{at_size, every_layer_kind_lowers_and_certifies, the_tiny_deploys, Experts, Gdn, Size, DENSE};

/// One layer at the model's REAL widths. Vocabulary is cut to keep the embedding and the output
/// projection off the critical path of this particular assertion — they are covered separately by
/// `the_real_vocabulary_projection_lowers`, so a failure here is unambiguously about the layer body.
fn real_width_layer() -> Qwen38Cfg {
    let mut cfg = Qwen38Cfg::real();
    cfg.layers = 1;
    cfg.vocab = 512;
    cfg
}

fn lower(cfg: &Qwen38Cfg) -> logicaffeine_compile::spatial::pipeline::ProgramLowering {
    let (src, _, _) = gen_qwen38_lg(cfg, "/w");
    let target = RduChip::Sn50.facts().spatial_target();
    compile_program_to_spatial(&src, &target).expect("the authored forward is a tensor program")
}

/// ── THE HEADLINE. Every statement of a full-width layer reaches the fabric.
#[test]
fn every_statement_of_a_real_width_layer_lowers() {
    let cfg = real_width_layer();
    assert_eq!(
        cfg.d, 5120,
        "this lock is meaningless at a width that already fits"
    );
    let l = lower(&cfg);
    if l.lowered != l.statements {
        let mut by_kind: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
        for m in &l.missing {
            by_kind
                .entry(m.kind.as_str())
                .or_default()
                .push(m.why.as_str());
        }
        let detail: Vec<String> = by_kind
            .iter()
            .map(|(k, v)| format!("{k} x{} (e.g. {})", v.len(), v[0]))
            .collect();
        panic!(
            "★ {} of {} statements of ONE full-width Qwen3.8 layer do not lower to the SN50 fabric.\n\
             {}\n\n\
             A row-wise op must not demand the whole row in the accumulator: at d={} a row is {} tiles \
             against an accumulator budget of 8. The fix is to BLOCK the row along its feature axis — a \
             streaming reduction for the norms, a streaming elementwise pass for the rest — not to raise \
             the budget, which is a property of the silicon and not ours to move.",
            l.statements - l.lowered,
            l.statements,
            detail.join("\n"),
            cfg.d,
            cfg.d / 32
        );
    }
    assert!(
        l.whole().is_some(),
        "a fully lowered layer must compose into one program"
    );
}

/// ── AND THE OTHER LAYER KIND, AS A DECODE STEP RUNS IT. `every_statement_of_a_real_width_layer_lowers` lowers layer 0,
/// a Gated-DeltaNet layer; the 16 attention layers are the other kind, and in a deployment each appends to and attends
/// over a KV cache. Layer 3 at full width — 24 query heads over 4 KV heads of 256, a 4096-position cache — lowers
/// whole, certifies, and carries its caches as dense `ctx × akv` state.
#[test]
fn every_statement_of_a_real_width_attention_decode_layer_lowers() {
    use logicaffeine_compile::codegen_cuda::qwen38::gen_qwen38_decode_layer_lg;
    use logicaffeine_compile::spatial::ir::TensorRole;
    let cfg = real_width_layer();
    let ctx = 4096;
    let l = 3;
    assert!(cfg.is_attn(l), "layer {l} is an attention layer");
    let (src, _, _) = gen_qwen38_decode_layer_lg(&cfg, l, "/w", ctx, 1);
    let target = RduChip::Sn50.facts().spatial_target();
    let lowered =
        compile_program_to_spatial(&src, &target).expect("the decode layer is a tensor program");
    assert_eq!(
        lowered.lowered,
        lowered.statements,
        "every statement of the attention decode layer lowers: {:?}",
        lowered.missing.first()
    );
    let p = lowered.whole().unwrap_or_else(|| {
        panic!(
            "the attention decode layer composes and certifies: {}",
            lowered.refusal()
        )
    });
    assert!(
        p.certificate.as_ref().is_some_and(|c| c.recheck(p)),
        "and its certificate rechecks"
    );
    for cache in [format!("Kc{l}"), format!("Vc{l}")] {
        let t = p
            .tensors
            .iter()
            .find(|t| t.name == cache)
            .unwrap_or_else(|| panic!("the layer carries `{cache}`"));
        assert_eq!(t.role, TensorRole::State, "`{cache}` is carried state");
        assert_eq!(
            (t.elems.rows, t.elems.cols),
            (ctx, cfg.akv()),
            "`{cache}` is dense tiles, a tile row per 32 positions"
        );
    }
    println!(
        "real-width attention decode layer: {} statements, {} sections",
        lowered.statements, lowered.sections
    );
}

/// ── AND BATCHED, THE WAY A SERVER RUNS IT. Both layer kinds of a BATCHED decode step at full width — the hidden
/// states stacked `2 × 5120`, every projection a `2 × 5120` matmul reading its weight once for both sequences, each
/// sequence's rows gathered out of and written back into the stacks, and both norms of the stack the blocked
/// sections over the matrix — lower whole and certify, with every sequence's caches its own dense state.
#[test]
fn every_statement_of_a_real_width_batched_decode_layer_lowers() {
    use logicaffeine_compile::codegen_cuda::qwen38::gen_qwen38_decode_batch_layer_lg;
    use logicaffeine_compile::spatial::ir::{StageKind, TensorRole};
    let cfg = real_width_layer();
    let (ctx, batch) = (4096, 2);
    let target = RduChip::Sn50.facts().spatial_target();
    for l in [0, 3] {
        let (src, _, _) = gen_qwen38_decode_batch_layer_lg(&cfg, l, "/w", ctx, &vec![1; batch]);
        let lowered = compile_program_to_spatial(&src, &target)
            .expect("the batched decode layer is a tensor program");
        assert_eq!(
            lowered.lowered,
            lowered.statements,
            "layer {l}: every statement of the batched decode layer lowers: {:?}",
            lowered.missing.first()
        );
        let p = lowered.whole().unwrap_or_else(|| {
            panic!(
                "layer {l}: the batched decode layer composes and certifies: {}",
                lowered.refusal()
            )
        });
        assert!(
            p.certificate.as_ref().is_some_and(|c| c.recheck(p)),
            "layer {l}: and its certificate rechecks"
        );
        let stack = p
            .tensors
            .iter()
            .find(|t| t.name == format!("bxn{l}"))
            .expect("the normalized stack");
        assert_eq!(
            (stack.elems.rows, stack.elems.cols),
            (batch, cfg.d),
            "layer {l}: the stack is a {batch} × d matrix"
        );
        let wq = p
            .tensors
            .iter()
            .find(|t| t.name == format!("Wq{l}"))
            .expect("the q projection");
        let reads: std::collections::BTreeSet<_> = p
            .stages
            .iter()
            .filter(|s| matches!(&s.kind, StageKind::Load { tensor, .. } if *tensor == wq.id))
            .map(|s| p.section_of_stage(s.id))
            .collect();
        assert_eq!(
            reads.len(),
            1,
            "layer {l}: `Wq{l}` is read by one section for both sequences"
        );
        if cfg.is_attn(l) {
            for b in 0..batch {
                for cache in [format!("Kc{l}_s{b}"), format!("Vc{l}_s{b}")] {
                    let t = p
                        .tensors
                        .iter()
                        .find(|t| t.name == cache)
                        .unwrap_or_else(|| panic!("the layer carries `{cache}`"));
                    assert_eq!(t.role, TensorRole::State, "`{cache}` is carried state");
                    assert_eq!(
                        (t.elems.rows, t.elems.cols),
                        (ctx, cfg.akv()),
                        "`{cache}` is dense tiles"
                    );
                }
            }
        }
        println!(
            "real-width batched decode layer {l} (batch {batch}): {} statements, {} sections",
            lowered.statements, lowered.sections
        );
    }
}

/// ── AND TENSOR-PARALLEL, THE WAY AN SN50 NODE SERVES IT. Both layer kinds of a decode step at full width split over
/// the 8 chips Qwen3.8-27B admits (`fit::qwen38_tensor_parallel`): every chip its 3 query heads and the KV head they
/// read (each KV head on 2 chips), its 2 key and 6 value Gated-DeltaNet heads, its 2,176 FFN columns — and the partials
/// summed by two all-reduces a layer, each over all 8 chips — lower whole and certify.
#[test]
fn every_statement_of_a_real_width_tensor_parallel_decode_layer_lowers() {
    use logicaffeine_compile::codegen_cuda::qwen38::gen_qwen38_tp_decode_layer_lg;
    use logicaffeine_compile::spatial::fit::qwen38_tensor_parallel;
    use logicaffeine_compile::spatial::ir::StageKind;
    use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial_tp;
    let cfg = real_width_layer();
    let tp = qwen38_tensor_parallel(&Qwen38Cfg::real(), &RduChip::Sn50.facts());
    assert_eq!(tp, 8, "Qwen3.8-27B is served over one node of 8 SN50s");
    let target = RduChip::Sn50.facts().spatial_target();
    for l in [0, 3] {
        let (src, _, _, m) = gen_qwen38_tp_decode_layer_lg(&cfg, l, "/w", 4096, 1, tp);
        let lowered = compile_program_to_spatial_tp(&src, &target, &m)
            .expect("the tensor-parallel layer is a tensor program");
        assert_eq!(
            lowered.lowered,
            lowered.statements,
            "layer {l}: every statement of the tensor-parallel layer lowers: {:?}",
            lowered.missing.first()
        );
        let p = lowered.whole().unwrap_or_else(|| {
            panic!(
                "layer {l}: the tensor-parallel layer composes and certifies: {}",
                lowered.refusal()
            )
        });
        assert!(
            p.certificate.as_ref().is_some_and(|c| c.recheck(p)),
            "layer {l}: and its certificate rechecks"
        );
        let chips: std::collections::BTreeSet<u16> = p
            .stages
            .iter()
            .filter_map(|s| s.site.map(|x| x.chip))
            .collect();
        assert_eq!(
            chips.len(),
            tp,
            "layer {l}: the layer runs on all {tp} chips"
        );
        let reduces: Vec<_> = p
            .stages
            .iter()
            .filter(|s| matches!(s.kind, StageKind::AllReduce { .. }))
            .collect();
        let reduced: std::collections::BTreeSet<_> = reduces
            .iter()
            .map(|s| match &s.kind {
                StageKind::AllReduce { tensor, .. } => p.tensor(*tensor).name.clone(),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            reduced,
            [format!("ao{l}"), format!("dn{l}")].into_iter().collect(),
            "layer {l}: two all-reduces, after the output projection and after the FFN"
        );
        for s in &reduces {
            let from: std::collections::BTreeSet<u16> = s
                .ins
                .iter()
                .filter_map(|b| p.buffer(*b).site.map(|x| x.chip))
                .collect();
            assert_eq!(
                from.len(),
                tp,
                "layer {l}: each all-reduce folds a partial from every chip"
            );
        }
        let wg: usize = (0..tp)
            .map(|c| m.slices[&format!("Wg{l}_c{c}")].shape().1)
            .sum();
        assert_eq!(
            wg, cfg.dff,
            "layer {l}: the FFN's columns split over the chips exactly once"
        );
        println!("real-width tensor-parallel decode layer {l} over {tp} chips: {} statements, {} sections", lowered.statements, lowered.sections);
    }
}

/// A decode layer at full width lowered with the weights the checkpoint stores Q8_0 (the generator's `q8w` keys), held
/// to what storing them promises: every stored weight is on the chip in exactly one form — its int8 values and
/// per-32-row scales (`W__q8`, `W__q8_scale`), every one of their column tiles (the last padded with zero columns, as a
/// gate's 48 heads are) read exactly once however many rows the layer carries; every activation value those products read is
/// reshaped into its blocks view ONCE; and the layer moves at most `bound`× the bytes of the same layer over bf16
/// weights. Returns (products folded, views, bytes stored, bytes dense).
/// Bytes of `tensor` the schedule reads (`writes = false`) or writes, every loop's count multiplied in.
fn moved(p: &logicaffeine_compile::spatial::ir::SpatialProgram, tensor: &str, writes: bool) -> u64 {
    use logicaffeine_compile::spatial::ir::{ChoreoOp, SpatialProgram, TaggedOp};
    use logicaffeine_compile::spatial::target::precision_bytes;
    fn walk(
        p: &SpatialProgram,
        ops: &[TaggedOp],
        mult: u64,
        tensor: &str,
        writes: bool,
        n: &mut u64,
    ) {
        let tile = (p.tile.rows * p.tile.cols) as u64;
        for t in ops {
            let hit = match &t.op {
                ChoreoOp::Loop { count, body, .. } => {
                    walk(p, body, mult * count.bound() as u64, tensor, writes, n);
                    None
                }
                ChoreoOp::ReadTiles {
                    tensor: id,
                    block_h,
                    block_w,
                    ..
                } if !writes => Some((*id, *block_h * *block_w)),
                ChoreoOp::WriteTiles {
                    tensor: id,
                    block_h,
                    block_w,
                    ..
                } if writes => Some((*id, *block_h * *block_w)),
                _ => None,
            };
            if let Some((id, tiles)) = hit.filter(|(id, _)| p.tensor(*id).name == tensor) {
                *n += mult * tiles as u64 * tile * precision_bytes(p.tensor(id).dtype) as u64;
            }
        }
    }
    let mut n = 0;
    for prog in &p.schedule.as_ref().expect("scheduled").programs {
        walk(p, &prog.ops, 1, tensor, writes, &mut n);
    }
    n
}

fn stored_layer_holds(
    src: &str,
    q8w: &std::collections::HashMap<String, String>,
    what: &str,
    bound: u64,
) -> (usize, usize, u64, u64) {
    use logicaffeine_compile::spatial::passes::from_q8::q8_rows_name;
    use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial_stored;
    use logicaffeine_compile::spatial::target::precision_bytes;
    use logicaffeine_compile::spatial::work::work_of;
    let target = RduChip::Sn50.facts().spatial_target();
    let q8: std::collections::BTreeSet<String> = q8w.keys().cloned().collect();
    let lowered = compile_program_to_spatial_stored(src, &target, &q8)
        .expect("the decode layer is a tensor program");
    let p = lowered.whole().unwrap_or_else(|| {
        panic!(
            "{what}: the stored decode layer composes and certifies: {}",
            lowered.refusal()
        )
    });
    assert!(
        p.certificate.as_ref().is_some_and(|c| c.recheck(p)),
        "{what}: and its certificate rechecks"
    );
    let decl = |name: &str| p.tensors.iter().find(|t| t.name == name);
    // The activation each product reads, from the program's own `Let y be x matmul W.` statements.
    let reads: std::collections::BTreeMap<&str, &str> = src
        .lines()
        .filter_map(|line| {
            let words: Vec<&str> = line
                .trim()
                .trim_end_matches('.')
                .split_whitespace()
                .collect();
            matches!(words.as_slice(), ["Let", _, "be", _, "matmul", _])
                .then(|| (words[5], words[3]))
        })
        .collect();
    let (mut folded, mut views) = (0usize, std::collections::BTreeSet::<&str>::new());
    for w in &q8 {
        let (values, scales) = (format!("{w}__q8"), format!("{w}__q8_scale"));
        match (decl(w), decl(&values), decl(&scales)) {
            (None, Some(v), Some(s)) => {
                let (k, n) = (v.elems.rows, v.elems.cols);
                assert_eq!(
                    (s.elems.rows, s.elems.cols),
                    (k / 32, n),
                    "{what}: `{w}` has one scale per 32 rows of each column"
                );
                // Its column tiles, the last padded with zero columns.
                let padded = v.tiles.cols_t * target.tile.cols;
                assert_eq!(
                    moved(p, &values, false),
                    (k * padded) as u64,
                    "{what}: every int8 of `{w}`'s column tiles is read once"
                );
                assert_eq!(
                    moved(p, &scales, false),
                    (4 * (k / 32) * padded) as u64,
                    "{what}: every scale of `{w}`'s column tiles is read once"
                );
                let x = reads
                    .get(w.as_str())
                    .unwrap_or_else(|| panic!("{what}: a product reads `{w}`"));
                views.insert(*x);
                folded += 1;
            }
            (Some(d), None, None) => assert!(
                d.elems.rows % 1024 != 0 || d.elems.cols % 32 != 0,
                "{what}: `{w}` ({} × {}) is whole K blocks and tiles, yet stays dense",
                d.elems.rows,
                d.elems.cols
            ),
            other => panic!(
                "{what}: `{w}` is on the chip in exactly one form, got {:?}",
                (other.0.is_some(), other.1.is_some(), other.2.is_some())
            ),
        }
    }
    assert!(
        folded >= 5,
        "{what}: the projections fold over their stored blocks ({folded} did)"
    );
    // A value's view is named for the value — the name and how often the program has written it (a stack is written
    // a row at a time) — so each activation the stored products read has exactly one.
    let stem = q8_rows_name("", 0);
    let stem = stem.trim_end_matches('0');
    let is_view_of = |name: &str, x: &str| {
        name.strip_prefix(x)
            .and_then(|rest| rest.strip_prefix(stem))
            .is_some_and(|v| v.parse::<usize>().is_ok())
    };
    for x in &views {
        let named: Vec<_> = p
            .tensors
            .iter()
            .filter(|t| is_view_of(&t.name, x))
            .collect();
        assert_eq!(
            named.len(),
            1,
            "{what}: `{x}` has one view: {:?}",
            named.iter().map(|t| &t.name).collect::<Vec<_>>()
        );
        let t = named[0];
        let once = (t.elems.rows.div_ceil(p.tile.rows)
            * t.elems.cols.div_ceil(p.tile.cols)
            * p.tile.rows
            * p.tile.cols) as u64
            * precision_bytes(t.dtype) as u64;
        assert_eq!(
            moved(p, &t.name, true),
            once,
            "{what}: `{}` is made once, however many products read it",
            t.name
        );
    }
    let made = p
        .tensors
        .iter()
        .filter(|t| {
            t.name
                .rsplit_once(stem)
                .is_some_and(|(x, _)| is_view_of(&t.name, x))
        })
        .count();
    assert_eq!(
        made,
        views.len(),
        "{what}: one view per activation value the stored products read"
    );
    let dense = compile_program_to_spatial(src, &target).expect("walk");
    let dense = dense
        .whole()
        .unwrap_or_else(|| panic!("{what}: the bf16 layer lowers: {}", dense.refusal()));
    let (stored_bytes, dense_bytes) = (
        work_of(p, &target).dram_bytes,
        work_of(dense, &target).dram_bytes,
    );
    assert!(100 * stored_bytes <= bound * dense_bytes, "{what}: the stored layer moves {stored_bytes} bytes against {dense_bytes} over bf16 weights");
    (folded, views.len(), stored_bytes, dense_bytes)
}

/// ── AND STORED THE WAY THE CHECKPOINT STORES IT. A decode step reads every weight once a token, so the bytes a weight
/// moves ARE the step. Both layer kinds of a decode step at full width hold [`stored_layer_holds`] at 0.65× the bf16
/// bytes.
#[test]
fn a_real_width_decode_layer_reads_its_q8_weights_as_stored() {
    use logicaffeine_compile::codegen_cuda::qwen38::gen_qwen38_decode_layer_lg;
    let cfg = real_width_layer();
    for l in [0, 3] {
        let (src, _, q8w) = gen_qwen38_decode_layer_lg(&cfg, l, "/w", 4096, 1);
        let (folded, views, stored, dense) =
            stored_layer_holds(&src, &q8w, &format!("layer {l}"), 65);
        println!("real-width decode layer {l} over Q8_0 weights: {folded} products fold, {views} views, {stored} bytes against {dense} over bf16");
    }
}

/// ── AND A BATCH OF THEM. A batched decode step's stacked rows fold over the same stored blocks: both layer kinds at
/// full width, 8 sequences, hold [`stored_layer_holds`] — every int8 and scale read ONCE for all 8 rows — at 0.76× the
/// bf16 bytes (each column group also reads the batch's view, which a single row's view barely weighs).
#[test]
fn a_real_width_batched_decode_layer_reads_its_q8_weights_once_for_the_batch() {
    use logicaffeine_compile::codegen_cuda::qwen38::gen_qwen38_decode_batch_layer_lg;
    let cfg = real_width_layer();
    for l in [0, 3] {
        let (src, _, q8w) = gen_qwen38_decode_batch_layer_lg(&cfg, l, "/w", 4096, &[4096; 8]);
        let (folded, views, stored, dense) =
            stored_layer_holds(&src, &q8w, &format!("batched layer {l}"), 76);
        println!("real-width batched decode layer {l} (batch 8) over Q8_0 weights: {folded} products fold, {views} views, {stored} bytes against {dense} over bf16");
    }
}

/// A tensor-parallel layer at full width lowered with the checkpoint's Q8_0 weights, held to what storing them across
/// the node promises: every chip's slice of a stored weight — its columns, or its rows when they are whole Q8_0 blocks
/// (a row-split output projection's 768, a down projection's 2,176, each ending in a tail) — is on ITS chip as int8
/// values and scales, every one of the values' column tiles (the last padded with zero columns, as a chip's 6 of a gate's
/// 48 heads are) read exactly once however many sequences the step carries, the dense slice never on
/// the chip; every chip reads only its own tensors and the replicated spine; and the layer moves at most `bound`× the
/// bytes of the same layer over bf16 slices.
fn stored_slices_hold(
    src: &str,
    q8w: &std::collections::HashMap<String, String>,
    m: &logicaffeine_compile::spatial::tensor_parallel::TpManifest,
    what: &str,
    bound: u64,
) -> (usize, u64, u64) {
    use logicaffeine_compile::spatial::ir::ChipId;
    use logicaffeine_compile::spatial::pipeline::{
        compile_program_to_spatial_tp, compile_program_to_spatial_tp_agreeing_stored,
    };
    use logicaffeine_compile::spatial::tensor_parallel::SliceAxis;
    use logicaffeine_compile::spatial::work::work_of;
    let target = RduChip::Sn50.facts().spatial_target();
    let q8: std::collections::BTreeSet<String> = m
        .slices
        .iter()
        .filter(|(_, s)| {
            q8w.contains_key(&s.of)
                && (s.axis == SliceAxis::Cols
                    || s.ranges.iter().all(|(a, n)| a % 32 == 0 && n % 32 == 0))
        })
        .map(|(name, _)| name.clone())
        .collect();
    assert!(
        q8.iter().any(|w| m.slices[w].axis == SliceAxis::Rows),
        "{what}: a row-split slice is stored too"
    );
    let lowered =
        compile_program_to_spatial_tp_agreeing_stored(src, &target, m, &Default::default(), &q8)
            .expect("the tensor-parallel layer is a tensor program");
    let p = lowered.whole().unwrap_or_else(|| {
        panic!(
            "{what}: the stored tensor-parallel layer composes and certifies: {}",
            lowered.refusal()
        )
    });
    assert!(
        p.certificate.as_ref().is_some_and(|c| c.recheck(p)),
        "{what}: and its certificate rechecks"
    );
    let decl = |name: &str| p.tensors.iter().find(|t| t.name == name);
    // A chip reads what lives on it and the replicated spine — never another chip's tensor (each chip makes its own
    // view of a replicated activation).
    for st in &p.stages {
        if let logicaffeine_compile::spatial::ir::StageKind::Load { tensor, .. } = &st.kind {
            let t = p.tensor(*tensor);
            if let (Some(site), Some(chip)) = (st.site, t.chip) {
                assert_eq!(
                    site.chip, chip.0,
                    "{what}: `{}` is read on chip {} and lives on chip {}",
                    t.name, site.chip, chip.0
                );
            }
        }
    }
    for w in &q8 {
        let (values, scales) = (format!("{w}__q8"), format!("{w}__q8_scale"));
        let (v, s) = (
            decl(&values).unwrap_or_else(|| panic!("{what}: `{values}`")),
            decl(&scales).unwrap_or_else(|| panic!("{what}: `{scales}`")),
        );
        assert!(
            decl(w).is_none(),
            "{what}: `{w}` is never on the chip dense"
        );
        let chip = Some(ChipId(m.chip_of[w]));
        assert_eq!(
            (v.chip, s.chip),
            (chip, chip),
            "{what}: `{w}`'s values and scales live on its chip"
        );
        assert_eq!(
            (v.elems.rows, v.elems.cols),
            m.slices[w].shape(),
            "{what}: `{values}` is the chip's slice"
        );
        assert_eq!(
            moved(p, &values, false),
            (v.elems.rows * v.tiles.cols_t * target.tile.cols) as u64,
            "{what}: every int8 of `{w}`'s column tiles is read once"
        );
    }
    let dense = compile_program_to_spatial_tp(src, &target, m).expect("walk");
    let dense = dense.whole().unwrap_or_else(|| {
        panic!(
            "{what}: the bf16 tensor-parallel layer lowers: {}",
            dense.refusal()
        )
    });
    let (stored_bytes, dense_bytes) = (
        work_of(p, &target).dram_bytes,
        work_of(dense, &target).dram_bytes,
    );
    assert!(100 * stored_bytes <= bound * dense_bytes, "{what}: the stored layer moves {stored_bytes} bytes against {dense_bytes} over bf16 slices");
    (q8.len(), stored_bytes, dense_bytes)
}

/// ── AND ACROSS THE NODE. Both layer kinds of a tensor-parallel decode step at full width over the 8 chips Qwen3.8-27B
/// admits hold [`stored_slices_hold`] at 0.65× the bf16 bytes.
#[test]
fn a_real_width_tensor_parallel_decode_layer_reads_its_q8_slices_as_stored() {
    use logicaffeine_compile::codegen_cuda::qwen38::gen_qwen38_tp_decode_layer_lg;
    let cfg = real_width_layer();
    for l in [0, 3] {
        let (src, _, q8w, m) = gen_qwen38_tp_decode_layer_lg(&cfg, l, "/w", 4096, 1, 8);
        let (stored, bytes, dense) = stored_slices_hold(&src, &q8w, &m, &format!("layer {l}"), 65);
        println!("real-width tensor-parallel decode layer {l} over 8 chips, Q8_0 slices: {stored} stored, {bytes} bytes against {dense} over bf16");
    }
}

/// ── AND SERVED: A BATCH ACROSS THE NODE. Both layer kinds of a batched tensor-parallel decode step at full width — 2
/// sequences over 8 chips, every chip's slice multiplying the stacked rows (a row slice's 2,176 rows are two whole
/// blocks and a tail, the batch's view then written in two parts) — hold [`stored_slices_hold`] at 0.76× the bf16
/// bytes.
#[test]
fn a_real_width_served_decode_layer_reads_its_q8_slices_once_for_the_batch() {
    use logicaffeine_compile::codegen_cuda::qwen38::gen_qwen38_decode_batch_tp_layer_lg;
    let cfg = real_width_layer();
    for l in [0, 3] {
        let (src, _, q8w, m) = gen_qwen38_decode_batch_tp_layer_lg(&cfg, l, "/w", 4096, &[1, 1], 8);
        let (stored, bytes, dense) =
            stored_slices_hold(&src, &q8w, &m, &format!("served layer {l}"), 76);
        println!("real-width served decode layer {l} (batch 2 over 8 chips), Q8_0 slices: {stored} stored, {bytes} bytes against {dense} over bf16");
    }
}

/// ── The widths that make it hard, named one at a time so a regression says WHICH one broke.
#[test]
fn the_row_ops_lower_at_every_width_the_model_uses() {
    let target = RduChip::Sn50.facts().spatial_target();
    // d, the GDN q/k width, the GDN v width, the attention query width, and dff — every row extent the
    // real forward actually produces.
    let real = Qwen38Cfg::real();
    for (label, width) in [
        ("d", real.d),
        ("gdn_qk", real.gdn_qk()),
        ("gdn_v", real.gdn_v()),
        ("attn_q", real.aq()),
        ("dff", real.dff),
    ] {
        let src = format!(
            "## Main\nLet x be a 1 by {width} tensor of Real.\nLet w be a 1 by {width} tensor of Real.\nLet y be rmsnorm of x with weight w.\n"
        );
        let l = compile_program_to_spatial(&src, &target).unwrap_or_else(|e| {
            panic!("an rmsnorm of width {width} ({label}) is a tensor program: {e:?}")
        });
        assert_eq!(
            l.lowered,
            l.statements,
            "★ rmsnorm at {label} = {width} ({} tiles) does not lower: {:?}",
            width / 32,
            l.missing.first().map(|m| m.why.as_str()).unwrap_or("")
        );
    }
}

/// ── The vocabulary projection is the widest tensor in the model by far.
#[test]
fn the_real_vocabulary_projection_lowers() {
    let real = Qwen38Cfg::real();
    let target = RduChip::Sn50.facts().spatial_target();
    let src = format!(
        "## Main\nLet h be a 1 by {} tensor of Real.\nLet wo be a {} by {} tensor of Real.\nLet logits be h matmul wo.\n",
        real.d, real.d, real.vocab
    );
    let l = compile_program_to_spatial(&src, &target).expect("a matmul is a tensor program");
    assert_eq!(
        l.lowered,
        l.statements,
        "★ the output projection {}x{} (vocab {}) does not lower: {:?}",
        real.d,
        real.vocab,
        real.vocab,
        l.missing.first().map(|m| m.why.as_str()).unwrap_or("")
    );
}

/// ── END TO END. The real model, all 64 layers. `#[ignore]`d for RUNTIME only.
#[test]
#[ignore = "runtime: the full 64-layer forward at real widths; run it on purpose"]
fn the_whole_real_forward_lowers() {
    let cfg = Qwen38Cfg::real();
    let l = lower(&cfg);
    assert_eq!(
        l.lowered,
        l.statements,
        "★ the real Qwen3.8-27B forward lowers {} of {} statements ({} sections). Missing: {:?}",
        l.lowered,
        l.statements,
        l.sections,
        l.missing
            .iter()
            .take(5)
            .map(|m| format!("{}: {}", m.kind, m.why))
            .collect::<Vec<_>>()
    );
    let p = l.whole().expect("the whole forward composes");
    assert!(
        p.certificate.is_some(),
        "the composed real forward must carry a certificate"
    );
}

// ── CORRECTNESS OF THE BLOCKED PATH ───────────────────────────────────────────────────────────────
//
// ★ THE BLOCKED FORM IS A DIFFERENT PROGRAM, SO IT OWES A DIFFERENT PROOF. Making a row op lower at
// `d = 5120` is worth nothing if the streamed arithmetic is not the arithmetic. `sum(x²)` becomes a
// matmul against a ones-vector and the scale is applied in a separate pass, so both the accumulation
// ORDER and the rounding POINTS move. This compares the blocked lowering against the VM at a width that
// forces blocking, element for element.

/// A width that is guaranteed to take the blocked path: 16 tiles against an eight-tile budget.
const BLOCKED_WIDTH: usize = 512;

fn rmsnorm_source(n: usize) -> String {
    let mut s = String::from("## Main\n");
    s.push_str(&format!("Let x be a 1 by {n} tensor of Real.\n"));
    s.push_str(&format!("Let w be a 1 by {n} tensor of Real.\n"));
    for i in 0..n {
        let xv = ((i as f64 * 37.0) % 19.0) / 19.0 - 0.5;
        let wv = ((i as f64 * 11.0) % 7.0) / 7.0 + 0.25;
        s.push_str(&format!("Set item {} of x to {xv:?}.\n", i + 1));
        s.push_str(&format!("Set item {} of w to {wv:?}.\n", i + 1));
    }
    s.push_str("Let y be rmsnorm of x with weight w.\n");
    s
}

#[test]
fn the_blocked_rmsnorm_computes_what_the_vm_computes() {
    use logicaffeine_compile::compile::vm_outcome;
    use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
    use logicaffeine_compile::spatial::ir::TensorRole;

    let n = BLOCKED_WIDTH;
    let mut src = rmsnorm_source(n);
    for i in 1..=n {
        src.push_str(&format!("Show round(item {i} of y * 1000000).\n"));
    }
    let vm = vm_outcome(&src);
    assert!(
        vm.error.is_none(),
        "the rmsnorm source runs on the VM: {:?}",
        vm.error
    );
    let want: Vec<i64> = vm
        .output
        .split_whitespace()
        .map(|t| t.parse().expect("an integer"))
        .collect();
    assert_eq!(want.len(), n, "the oracle prints the whole row");

    let target = RduChip::Sn50.facts().spatial_target();
    let l = compile_program_to_spatial(&src, &target).expect("a tensor program");
    assert_eq!(
        l.lowered, l.statements,
        "the blocked rmsnorm must lower whole: {:?}",
        l.missing
    );
    assert!(l.sections > 1, "a blocked rmsnorm is a COMPOSITION — {} section(s) means it took the whole-row path and this test proves nothing", l.sections);
    let p = l.whole().expect("it composes").clone();

    let mut inputs = Inputs::default();
    for t in p.tensors.iter().filter(|t| t.role != TensorRole::Output) {
        let cnt = t.elems.rows * t.elems.cols;
        if t.init.element_f64(0).is_some() {
            inputs.tensors.insert(
                t.id,
                Value::Float(
                    (0..cnt)
                        .map(|i| t.init.element_f64(i).unwrap_or(0.0))
                        .collect(),
                ),
            );
        }
    }
    let out = eval_choreography(&p, &inputs).expect("the blocked choreography evaluates");
    let yt = p
        .tensors
        .iter()
        .find(|t| t.name == "y")
        .expect("it writes `y`");
    let vals: Vec<f64> = match out.tensors.get(&yt.id).expect("`y` is written") {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    };
    let mine: Vec<i64> = vals
        .iter()
        .take(n)
        .map(|x| (x * 1_000_000.0).round() as i64)
        .collect();
    assert_eq!(mine.len(), n, "the blocked lowering writes the whole row");

    const REL: f64 = 0.02;
    let scale = want.iter().map(|x| x.abs()).max().unwrap_or(1).max(1) as f64;
    let worst = mine
        .iter()
        .zip(&want)
        .map(|(a, b)| ((a - b) as f64).abs() / scale)
        .fold(0.0, f64::max);
    assert!(
        worst <= REL,
        "★ the BLOCKED rmsnorm drifts {worst:.4} from the VM, past the {REL} a bf16 row reduction may. \
         The streamed form moves both the accumulation order and the rounding points, so this is exactly \
         where a wrong answer would hide — lowering is not correctness.\n  ours[..8]: {:?}\n  vm[..8]:   {:?}",
        &mine[..8.min(mine.len())],
        &want[..8.min(want.len())]
    );
}

/// ── The per-head L2 normalization, blocked per BAND, against the VM at Qwen's real head geometry.
#[test]
fn the_blocked_l2norm_computes_what_the_vm_computes() {
    use logicaffeine_compile::compile::vm_outcome;
    use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
    use logicaffeine_compile::spatial::ir::TensorRole;

    // Qwen3.8's GDN q/k geometry: 16 heads of 128. Flat that is 2048 elements = 64 tiles, far past the
    // eight-tile budget, while ONE head is four tiles and fits — which is the whole point of the band.
    let real = Qwen38Cfg::real();
    let (heads, hd) = (real.gdn_k_heads, real.gdn_head_dim);
    let n = heads * hd;
    let mut src = format!("## Main\nLet x be a {heads} by {hd} tensor of Real.\n");
    for i in 0..n {
        let v = ((i as f64 * 23.0) % 17.0) / 17.0 - 0.45;
        src.push_str(&format!("Set item {} of x to {v:?}.\n", i + 1));
    }
    src.push_str(&format!("Let y be l2norm of x across {heads} heads.\n"));
    for i in 1..=n {
        src.push_str(&format!("Show round(item {i} of y * 1000000).\n"));
    }

    let vm = vm_outcome(&src);
    assert!(
        vm.error.is_none(),
        "the l2norm source runs on the VM: {:?}",
        vm.error
    );
    let want: Vec<i64> = vm
        .output
        .split_whitespace()
        .map(|t| t.parse().expect("an integer"))
        .collect();
    assert_eq!(want.len(), n, "the oracle prints every element");

    let target = RduChip::Sn50.facts().spatial_target();
    let l = compile_program_to_spatial(&src, &target).expect("a tensor program");
    assert_eq!(
        l.lowered, l.statements,
        "the blocked l2norm must lower whole: {:?}",
        l.missing
    );
    let p = l.whole().expect("it composes").clone();
    // It must really be the banded form: one work item per head, not one for the flat row.
    let items = p.work.items.len();
    assert!(items >= heads, "★ {items} work item(s) for {heads} heads — this took the flat path and the test proves nothing");

    let mut inputs = Inputs::default();
    for t in p.tensors.iter().filter(|t| t.role != TensorRole::Output) {
        let cnt = t.elems.rows * t.elems.cols;
        if t.init.element_f64(0).is_some() {
            inputs.tensors.insert(
                t.id,
                Value::Float(
                    (0..cnt)
                        .map(|i| t.init.element_f64(i).unwrap_or(0.0))
                        .collect(),
                ),
            );
        }
    }
    let out = eval_choreography(&p, &inputs).expect("the banded choreography evaluates");
    let yt = p
        .tensors
        .iter()
        .find(|t| t.name == "y")
        .expect("it writes `y`");
    let vals: Vec<f64> = match out.tensors.get(&yt.id).expect("`y` is written") {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    };
    let mine: Vec<i64> = vals
        .iter()
        .take(n)
        .map(|x| (x * 1_000_000.0).round() as i64)
        .collect();

    const REL: f64 = 0.02;
    let scale = want.iter().map(|x| x.abs()).max().unwrap_or(1).max(1) as f64;
    let worst = mine
        .iter()
        .zip(&want)
        .map(|(a, b)| ((a - b) as f64).abs() / scale)
        .fold(0.0, f64::max);
    assert!(
        worst <= REL,
        "★ the BANDED l2norm drifts {worst:.4} from the VM, past {REL}. Each head must fold ONLY its own \
         {hd} elements — a band that folds the wrong extent is exactly the silent wrong answer here.\n  \
         ours[..8]: {:?}\n  vm[..8]:   {:?}",
        &mine[..8.min(mine.len())],
        &want[..8.min(want.len())]
    );
}

/// ── The causal convolution, blocked per CHANNEL, against the VM at Qwen's real conv width.
/// ★ THE ONE THAT COULD HAVE BEEN SILENTLY WRONG. Blocking re-declares the ring as `taps x chan` so a
/// channel tile carries its own history, and the tap's stride inside the loaded page becomes a TILE
/// WIDTH rather than `chan`. Get that stride wrong and every channel convolves against a neighbour's
/// history — a result that is correctly shaped, fully written, and wrong.
#[test]
fn the_blocked_causal_conv_computes_what_the_vm_computes() {
    use logicaffeine_compile::compile::vm_outcome;
    use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
    use logicaffeine_compile::spatial::ir::TensorRole;

    let real = Qwen38Cfg::real();
    let (taps, chan) = (real.conv_taps, 320usize); // 10 tiles > the 8-tile budget, so this takes the BLOCKED path
    let mut src = format!("## Main\nLet x be a 1 by {chan} tensor of Real.\n");
    src.push_str(&format!("Let wc be a {taps} by {chan} tensor of Real.\n"));
    src.push_str(&format!("Let cs be a {taps} by {chan} tensor of Real.\n"));
    for i in 0..chan {
        src.push_str(&format!(
            "Set item {} of x to {:?}.\n",
            i + 1,
            ((i as f64 * 13.0) % 11.0) / 11.0 - 0.5
        ));
    }
    for i in 0..taps * chan {
        src.push_str(&format!(
            "Set item {} of wc to {:?}.\n",
            i + 1,
            ((i as f64 * 7.0) % 5.0) / 5.0 - 0.4
        ));
        src.push_str(&format!(
            "Set item {} of cs to {:?}.\n",
            i + 1,
            ((i as f64 * 29.0) % 13.0) / 13.0 - 0.5
        ));
    }
    src.push_str(&format!(
        "Let y be causal conv of x with weight wc state cs over {taps} taps.\n"
    ));
    for i in 1..=chan {
        src.push_str(&format!("Show round(item {i} of y * 1000000).\n"));
    }

    let vm = vm_outcome(&src);
    assert!(
        vm.error.is_none(),
        "the conv source runs on the VM: {:?}",
        vm.error
    );
    let want: Vec<i64> = vm
        .output
        .split_whitespace()
        .map(|t| t.parse().expect("an integer"))
        .collect();
    assert_eq!(want.len(), chan, "the oracle prints every channel");

    let target = RduChip::Sn50.facts().spatial_target();
    let l = compile_program_to_spatial(&src, &target).expect("a tensor program");
    assert!(
        l.missing.is_empty(),
        "the blocked conv must lower whole: {:?}",
        l.missing
    );
    let p = l.whole().expect("it composes").clone();
    assert!(
        p.work.items.len() > 1,
        "★ one work item means this took the flat path and proves nothing"
    );

    let mut inputs = Inputs::default();
    for t in p.tensors.iter().filter(|t| t.role != TensorRole::Output) {
        let cnt = t.elems.rows * t.elems.cols;
        if t.init.element_f64(0).is_some() {
            inputs.tensors.insert(
                t.id,
                Value::Float(
                    (0..cnt)
                        .map(|i| t.init.element_f64(i).unwrap_or(0.0))
                        .collect(),
                ),
            );
        }
    }
    let out = eval_choreography(&p, &inputs).expect("the blocked conv evaluates");
    let yt = p
        .tensors
        .iter()
        .find(|t| t.name == "y")
        .expect("it writes `y`");
    let vals: Vec<f64> = match out.tensors.get(&yt.id).expect("`y` is written") {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    };
    let mine: Vec<i64> = vals
        .iter()
        .take(chan)
        .map(|x| (x * 1_000_000.0).round() as i64)
        .collect();
    const REL: f64 = 0.02;
    let scale = want.iter().map(|x| x.abs()).max().unwrap_or(1).max(1) as f64;
    let worst = mine
        .iter()
        .zip(&want)
        .map(|(a, b)| ((a - b) as f64).abs() / scale)
        .fold(0.0, f64::max);
    assert!(
        worst <= REL,
        "★ the BLOCKED causal conv drifts {worst:.4} from the VM, past {REL}. Each channel must fold its \
         OWN {taps} taps.\n  ours[..8]: {:?}\n  vm[..8]:   {:?}",
        &mine[..8.min(mine.len())],
        &want[..8.min(want.len())]
    );
}

// ── THE ATTENTION-LAYER OPS, BLOCKED PER HEAD ─────────────────────────────────────────────────────
//
// ★ WHY THESE WERE MISSED FOR SO LONG. Qwen3.8 is a HYBRID: three Gated-DeltaNet layers for every one
// of attention. Layer 0 is GDN, so a one-layer probe at real widths exercises none of the attention
// path — and every test above it ran exactly that. The full 64-layer model is what surfaced them: 80
// refusals, all in the sixteen attention layers, none reachable from a single layer.
//
// Both ops are per-head by construction — a rotary embedding never crosses a head, and a per-head norm
// reduces over exactly one head's `hd` elements — so the head is the work item and the page IS the head.

/// A width that forces blocking (32 tiles against a budget of 8) while staying inside the VM's frame.
const HEADS_T: usize = 4;
const HD_T: usize = 256;

fn filled(name: &str, n: usize, salt: f64) -> String {
    let mut s = String::new();
    for i in 0..n {
        let v = ((i as f64 * salt) % 13.0) / 13.0 - 0.5;
        s.push_str(&format!("Set item {} of {name} to {v:?}.\n", i + 1));
    }
    s
}

fn eval_vs_vm(src: &str, out_name: &str, n: usize, what: &str) {
    use logicaffeine_compile::compile::vm_outcome;
    use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
    use logicaffeine_compile::spatial::ir::TensorRole;

    let mut full = src.to_string();
    for i in 1..=n {
        full.push_str(&format!("Show round(item {i} of {out_name} * 1000000).\n"));
    }
    let vm = vm_outcome(&full);
    assert!(
        vm.error.is_none(),
        "{what}: the source runs on the VM: {:?}",
        vm.error
    );
    let want: Vec<i64> = vm
        .output
        .split_whitespace()
        .map(|t| t.parse().expect("an integer"))
        .collect();
    assert_eq!(want.len(), n, "{what}: the oracle prints every element");

    let target = RduChip::Sn50.facts().spatial_target();
    let l = compile_program_to_spatial(&full, &target).expect("a tensor program");
    assert_eq!(
        l.lowered, l.statements,
        "{what} must lower whole: {:?}",
        l.missing
    );
    let p = l.whole().expect("it composes").clone();
    assert!(
        p.work.items.len() > 1,
        "★ {what}: one work item means the flat path ran and this proves nothing"
    );

    let mut inputs = Inputs::default();
    for t in p.tensors.iter().filter(|t| t.role != TensorRole::Output) {
        let cnt = t.elems.rows * t.elems.cols;
        if t.init.element_f64(0).is_some() {
            inputs.tensors.insert(
                t.id,
                Value::Float(
                    (0..cnt)
                        .map(|i| t.init.element_f64(i).unwrap_or(0.0))
                        .collect(),
                ),
            );
        }
    }
    let out = eval_choreography(&p, &inputs)
        .unwrap_or_else(|e| panic!("{what}: the blocked choreography evaluates: {e:?}"));
    let yt = p
        .tensors
        .iter()
        .find(|t| t.name == out_name)
        .expect("the output tensor");
    let vals: Vec<f64> = match out.tensors.get(&yt.id).expect("written") {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    };
    let mine: Vec<i64> = vals
        .iter()
        .take(n)
        .map(|x| (x * 1_000_000.0).round() as i64)
        .collect();
    const REL: f64 = 0.02;
    let scale = want.iter().map(|x| x.abs()).max().unwrap_or(1).max(1) as f64;
    // ★ REPORT WHERE, NOT JUST HOW MUCH. These ops are per-head, so the INDEX of the worst element names
    // the head that went wrong — and a mapping bug shows up as a few bad heads among many right ones,
    // which a single drift number hides completely.
    let (wi, worst) = mine
        .iter()
        .zip(&want)
        .enumerate()
        .map(|(i, (a, b))| (i, ((a - b) as f64).abs() / scale))
        .fold((0usize, 0.0f64), |acc, x| if x.1 > acc.1 { x } else { acc });
    let bad: Vec<usize> = mine
        .iter()
        .zip(&want)
        .enumerate()
        .filter(|(_, (a, b))| ((*a - *b) as f64).abs() / scale > REL)
        .map(|(i, _)| i)
        .collect();
    assert!(
        worst <= REL,
        "★ the BLOCKED {what} drifts {worst:.4} from the VM at element {wi}, past {REL}.\n  \
         {} of {} elements are wrong; first bad indices: {:?}\n  \
         ours[{wi}]={} vm[{wi}]={}\n  ours[..6]: {:?}\n  vm[..6]:   {:?}",
        bad.len(),
        mine.len(),
        &bad[..8.min(bad.len())],
        mine[wi],
        want[wi],
        &mine[..6.min(mine.len())],
        &want[..6.min(want.len())]
    );
}

#[test]
fn the_blocked_per_head_rmsnorm_computes_what_the_vm_computes() {
    let (h, hd) = (HEADS_T, HD_T);
    let n = h * hd;
    let mut src = format!(
        "## Main\nLet x be a {h} by {hd} tensor of Real.\nLet w be a 1 by {hd} tensor of Real.\n"
    );
    src.push_str(&filled("x", n, 23.0));
    src.push_str(&filled("w", hd, 7.0));
    src.push_str(&format!(
        "Let y be rmsnorm of x with weight w across {h} heads.\n"
    ));
    eval_vs_vm(&src, "y", n, "per-head rmsnorm");
}

#[test]
fn the_blocked_rope_computes_what_the_vm_computes() {
    // Qwen3.8's PARTIAL rotary ratio (2*half < hd) at a head dim the VM oracle can carry: 16 heads of 64
    // is 32 tiles — four times the eight-tile budget, so the blocked path runs — with the same
    // rotate/rotate/pass-through band structure the real 24x256 has.
    let (h, hd) = (16usize, 64usize);
    let (n, half) = (h * hd, 16usize);
    // The rope operand is a FLAT activation — the output of the q/k projection chain — and `across
    // {h} heads` is what gives it head structure. A `R by C` declaration means something else: the
    // parser reads C as one row's width and divides IT by the head count.
    let mut src = format!("## Main\nLet x be a 1 by {n} tensor of Real.\nLet rc be a 1 by {half} tensor of Real.\nLet rs be a 1 by {half} tensor of Real.\n");
    src.push_str(&filled("x", n, 23.0));
    src.push_str(&filled("rc", half, 5.0));
    src.push_str(&filled("rs", half, 11.0));
    src.push_str(&format!(
        "Let y be rope of x with cos rc and sin rs across {h} heads.\n"
    ));
    eval_vs_vm(&src, "y", n, "rope");
}

/// ── Attention, blocked per head, against the VM.
///
/// ★ THE PART THAT COULD BE SILENTLY WRONG IS THE GQA MAPPING. Head `head` must read k-head
/// `head/group`, and the blocked form gets there by DECOMPOSING the head into the item's two
/// coordinates (`head = kvh·group + rep`) rather than by expanding `k`. If that decomposition is off,
/// every head still produces a score, the softmax still sums to one, and the context vector is still the
/// right shape — built from the wrong head's keys. Nothing downstream can tell.
#[test]
fn the_blocked_attention_computes_what_the_vm_computes() {
    // 8 heads of 64 over 2 k-heads: `h·hd = 512` is 16 tiles (twice the budget, so the blocked path
    // runs) while one head is 2 tiles, and `group = 4` means three of every four heads share their keys.
    let (h, kv, hd) = (8usize, 2usize, 64usize);
    let (qn, kn) = (h * hd, kv * hd);
    let mut src = format!(
        "## Main\nLet q be a 1 by {qn} tensor of Real.\nLet k be a 1 by {kn} tensor of Real.\nLet v be a 1 by {kn} tensor of Real.\n"
    );
    src.push_str(&filled("q", qn, 23.0));
    src.push_str(&filled("k", kn, 7.0));
    src.push_str(&filled("v", kn, 11.0));
    src.push_str(&format!(
        "Let ct be attention of q over k and v with {h} heads {kv} groups.\n"
    ));
    eval_vs_vm(&src, "ct", qn, "attention");
}

/// Every Qwen 3.5 and 3.8 Gated DeltaNet: sixteen key heads of 128, the value heads the size's own, four taps.
const fn gdn(v_heads: usize) -> Option<Gdn> {
    Some(Gdn { k_heads: 16, v_heads, head_dim: 128, taps: 4 })
}

/// The dense releases, each named as Qwen publishes it and as its `config.json` states it
/// (`every_size_is_its_published_configuration`).
const DENSE_SIZES: [Size; 6] = [
    Size { name: "Qwen3.5-0.8B", d: 1024, heads: 8, kv_heads: 2, head_dim: 256, dff: 3584, vocab: 248_320, tied: true, gdn: gdn(16), ..DENSE },
    Size { name: "Qwen3.5-2B", d: 2048, heads: 8, kv_heads: 2, head_dim: 256, dff: 6144, vocab: 248_320, tied: true, gdn: gdn(16), ..DENSE },
    Size { name: "Qwen3.5-4B", d: 2560, heads: 16, kv_heads: 4, head_dim: 256, dff: 9216, vocab: 248_320, tied: true, gdn: gdn(32), ..DENSE },
    Size { name: "Qwen3.5-9B", d: 4096, heads: 16, kv_heads: 4, head_dim: 256, dff: 12_288, vocab: 248_320, gdn: gdn(32), ..DENSE },
    Size { name: "Qwen3.5-27B", d: 5120, heads: 24, kv_heads: 4, head_dim: 256, dff: 17_408, vocab: 248_320, gdn: gdn(48), ..DENSE },
    Size { name: "Qwen3.8-27B", d: 5120, heads: 24, kv_heads: 4, head_dim: 256, dff: 17_408, vocab: 248_320, gdn: gdn(48), ..DENSE },
];

/// The mixtures, each named and stated as the dense releases are: 256 experts top 8, and 512 experts top 10.
const MIXTURE_SIZES: [Size; 4] = [
    Size {
        name: "Qwen3.5-35B-A3B",
        d: 2048,
        heads: 16,
        kv_heads: 2,
        head_dim: 256,
        vocab: 248_320,
        experts: Some(Experts { count: 256, top_k: 8, dff: 512, shared_dff: 512, groups: None }),
        gdn: gdn(32),
        ..DENSE
    },
    Size {
        name: "Qwen3.5-122B-A10B",
        d: 3072,
        heads: 32,
        kv_heads: 2,
        head_dim: 256,
        vocab: 248_320,
        experts: Some(Experts { count: 256, top_k: 8, dff: 1024, shared_dff: 1024, groups: None }),
        gdn: gdn(64),
        ..DENSE
    },
    Size {
        name: "Qwen3.5-397B-A17B",
        d: 4096,
        heads: 32,
        kv_heads: 2,
        head_dim: 256,
        vocab: 248_320,
        experts: Some(Experts { count: 512, top_k: 10, dff: 1024, shared_dff: 1024, groups: None }),
        gdn: gdn(64),
        ..DENSE
    },
    Size {
        name: "Qwen3.8-2.4T-A95B",
        d: 8192,
        heads: 64,
        kv_heads: 4,
        head_dim: 256,
        vocab: 248_320,
        experts: Some(Experts { count: 512, top_k: 10, dff: 2048, shared_dff: 2048, groups: None }),
        gdn: gdn(128),
        ..DENSE
    },
];

#[test]
fn the_tiny_bundle_equals_the_vm_and_the_references() {
    the_tiny_deploys("qwen35");
}

/// Qwen3.8-2.4T's layout (`Qwen3_5MoeForCausalLM`: the text model alone under `model.`, its experts stored fused) from
/// its twin's directory, deployed as the multimodal checkpoint is.
#[test]
fn the_text_model_layouts_tiny_bundle_equals_the_vm_and_the_references() {
    the_tiny_deploys("qwen35moe-causal");
}

#[test]
fn every_real_width_layer_kind_lowers_and_certifies() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let (dir, _) = common::hf_twins::twin("qwen35");
    let twin = ModelSpec::from_source(&*open_checkpoint(&dir).expect("the twin")).expect("Qwen 3.5's spec");
    every(DENSE_SIZES.iter().map(|s| (s.name.to_string(), s)), |size| every_layer_kind_lowers_and_certifies(&at_size(&twin, size), size.name));
}

#[test]
fn every_real_width_mixture_layer_kind_lowers_and_certifies() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let (dir, _) = common::hf_twins::twin("qwen35moe");
    let twin = ModelSpec::from_source(&*open_checkpoint(&dir).expect("the twin")).expect("Qwen 3.5's mixture's spec");
    every(MIXTURE_SIZES.iter().map(|s| (s.name.to_string(), s)), |size| every_layer_kind_lowers_and_certifies(&at_size(&twin, size), size.name));
}

/// Each size above is the release Qwen publishes under its name: the release's `config.json`
/// (`scripts/accel/fetch-hf-configs.sh`) states its width, heads, head width, vocabulary and tied head, its Gated
/// DeltaNet's key and value heads, their width and the convolution's taps, and its FFN's width or its experts'.
#[test]
fn every_size_is_its_published_configuration() {
    if !common::require_tool("hf-configs") {
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../external/hf-configs/Qwen");
    every(DENSE_SIZES.iter().chain(&MIXTURE_SIZES).map(|s| (s.name.to_string(), s)), |size| {
        let path = root.join(size.name).join("config.json");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let top: serde_json::Value = serde_json::from_str(&text).expect("config.json parses");
        let c = top.get("text_config").unwrap_or(&top);
        let n = |k: &str| c[k].as_u64().unwrap_or_else(|| panic!("`{k}` is a number")) as usize;
        assert_eq!(
            (size.d, size.heads, size.kv_heads, size.head_dim, size.vocab),
            (n("hidden_size"), n("num_attention_heads"), n("num_key_value_heads"), n("head_dim"), n("vocab_size")),
            "the width, the heads, the head width and the vocabulary"
        );
        assert_eq!(Some(size.tied), top["tie_word_embeddings"].as_bool(), "the tied head");
        let g = size.gdn.as_ref().expect("every size has Gated DeltaNet layers");
        assert_eq!(
            (g.k_heads, g.v_heads, g.head_dim, g.head_dim, g.taps),
            (
                n("linear_num_key_heads"),
                n("linear_num_value_heads"),
                n("linear_key_head_dim"),
                n("linear_value_head_dim"),
                n("linear_conv_kernel_dim")
            ),
            "the Gated DeltaNet"
        );
        match &size.experts {
            Some(e) => assert_eq!(
                (e.count, e.top_k, e.dff, e.shared_dff),
                (n("num_experts"), n("num_experts_per_tok"), n("moe_intermediate_size"), n("shared_expert_intermediate_size")),
                "the experts"
            ),
            None => assert_eq!(size.dff, n("intermediate_size"), "the FFN's width"),
        }
    });
}
