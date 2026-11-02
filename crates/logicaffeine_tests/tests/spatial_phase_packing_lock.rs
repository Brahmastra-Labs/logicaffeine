//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! INDEPENDENT SECTIONS RUN AT ONCE, ON DISJOINT SITES — A BARRIER ONLY WHERE DATA FLOWS.
//!
//! A composed program's sections used to run strictly one after another, a barrier after each: a decode layer's four
//! projections of one normalized row, each on a few dozen of the SN50's 1,040 sites, took four phases while the rest
//! of the fabric idled. On a chip whose sites are interchangeable (a crossbar, no multicast geometry), sections that
//! share no data — neither reads what the other writes, nor writes what the other touches — run in ONE PHASE, packed
//! onto disjoint sites; a barrier closes each phase. Disjoint sites are disjoint local memories, so sections of a phase
//! never share a buffer; the certificate still demands a barrier chain between every writer and every reader.
//!
//! Pinned:
//!   1. Three products of one row share one phase, each on its own sites; a chain of three keeps a barrier between
//!      each pair. Two sections of one phase that DO share a site (placed there, no barrier between them) hold disjoint
//!      bytes of it — composition lays them one past another — and a program that puts them over the same bytes is
//!      refused by name (`MemoryOverlap`, the overlap rule scoped to the PHASE, the unit a barrier separates).
//!   2. A phase never holds more sites than the mesh: on a small mesh the same products take as many phases as their
//!      sites need.
//!   3. Packed programs certify and compute the VM's values (the two-part rule of the ledger) — and a write waits for
//!      the reads before it: a row written into `x` after a product read `x` lands in a later phase, and a product after
//!      the write reads the new row.
//!   4. At real width a Gated-DeltaNet decode layer runs in fewer phases than it has sections, its four projections of
//!      the normalized row in one phase.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_phase_packing_lock`
//!
//! ★ MUTATION PROBES (run 2026-09-27 in a private copy; logs `logs/sn50/probe_pack_*.txt`):
//!   • every section packed into phase 0 (dependences ignored) → the CERTIFICATE refuses the chain and the row write
//!     (`ActivationOrder` / `StateOrder`: no barrier chain) — the packing's safety is the proof's, not only its own;
//!   • a write placed without waiting for the reads before it → the row write refused (`StateOrder`);
//!   • a phase filled past the mesh → the small-mesh test (no free site left to rename onto);
//!   • sections left on their own sites → composition lays the phase's sections on a shared site one past another
//!     (`separate_phase_memory`), so it computes; this lock's disjointness check reds. (Before the overlap rule was
//!     scoped to the phase, two sections of a phase over the same bytes composed, certified and computed by luck.)
//!   • the overlap rule scoped to the section again → two sections of a phase over the same bytes validate (`Ok`);
//!   • composition not separating a phase's memory → `MemoryOverlap` at composition, here and in `spatial_compose_lock`.
//! ════════════════════════════════════════════════════════════════════════════════════════════════
#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::eval_choreography;
use logicaffeine_compile::spatial::ir::{Phase, Site, SpatialProgram};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs};

fn sn50() -> SpatialTarget {
    SpatialTarget::sn50_rdu()
}

fn fp32_twin(t: &SpatialTarget) -> SpatialTarget {
    let mut t = t.clone();
    for c in &mut t.compute {
        if let ComputeCap::MatmulTile { precisions, .. }
        | ComputeCap::Elementwise { precisions }
        | ComputeCap::RowReduce { precisions } = c
        {
            if precisions.remove(&Precision::Bf16) {
                precisions.insert(Precision::Fp32);
            }
        }
    }
    t.precisions.insert(Precision::Fp32);
    t
}

/// A value exact in bf16: a multiple of 1/64 in `[-1, 1]`.
fn val(p: usize, salt: usize) -> f64 {
    ((((p + salt) * 7919 + 13) % 129) as f64 - 64.0) / 64.0
}

fn declare(name: &str, rows: usize, cols: usize, salt: usize) -> String {
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Real.\n");
    for p in 0..rows * cols {
        s += &format!("Set item {} of {name} to {:?}.\n", p + 1, val(p, salt));
    }
    s
}

/// `x` (1 × 64) times three 64 × 64 weights — side by side (each reads `x`) or chained (each reads the one before).
fn products(chained: bool) -> String {
    let mut s = String::from("## Main\n");
    s += &declare("x", 1, 64, 1);
    for w in 1..=3 {
        s += &declare(&format!("W{w}"), 64, 64, 10 * w);
    }
    for w in 1..=3 {
        let input = if chained && w > 1 {
            format!("y{}", w - 1)
        } else {
            "x".to_string()
        };
        s += &format!("Let y{w} be {input} matmul W{w}.\n");
    }
    // A chain's result is its last product; side by side, each product is a result.
    for w in if chained { 3..=3 } else { 1..=3 } {
        for c in [1, 17, 64] {
            s += &format!("Show round(item {c} of y{w} * 1000000).\n");
        }
    }
    s
}

fn composed(src: &str, target: &SpatialTarget) -> SpatialProgram {
    let l = compile_program_to_spatial(src, target).expect("a tensor program");
    let p = l
        .program
        .unwrap_or_else(|| panic!("the program composes and certifies: {:?}", l.missing));
    assert_eq!(p.phase(), Phase::Verified, "it certifies");
    assert!(
        p.certificate.as_ref().is_some_and(|c| c.recheck(&p)),
        "its certificate rechecks"
    );
    p
}

/// The program's phases: runs of sections closed by a barrier, each the sections' indices.
fn phases(p: &SpatialProgram) -> Vec<Vec<usize>> {
    let mut out = vec![Vec::new()];
    for (k, s) in p.sections.iter().enumerate() {
        out.last_mut().unwrap().push(k);
        if s.barrier_after.is_some() {
            out.push(Vec::new());
        }
    }
    out.retain(|ph| !ph.is_empty());
    out
}

fn sites_of(p: &SpatialProgram, section: usize) -> BTreeSet<Site> {
    let (a, b) = p.sections[section].stages;
    p.stages[a..b].iter().filter_map(|s| s.site).collect()
}

// ───────────────────────────── 1. independent share a phase, a chain does not ─────────────────────────────

#[test]
fn independent_products_share_a_phase_on_disjoint_sites_and_a_chain_keeps_its_barriers() {
    let side = composed(&products(false), &sn50());
    let ph = phases(&side);
    assert_eq!(
        ph.len(),
        1,
        "three products of one row run in one phase: {ph:?}"
    );
    for (i, a) in ph[0].iter().enumerate() {
        for b in &ph[0][i + 1..] {
            assert!(
                sites_of(&side, *a).is_disjoint(&sites_of(&side, *b)),
                "sections {a} and {b} of one phase hold disjoint sites"
            );
        }
    }
    let chain = composed(&products(true), &sn50());
    assert_eq!(
        phases(&chain).len(),
        3,
        "a chain of three keeps a barrier between each pair: {:?}",
        phases(&chain)
    );
}

#[test]
fn a_phase_on_one_site_holds_disjoint_bytes_and_sharing_one_is_refused_by_name() {
    use logicaffeine_compile::spatial::ir::IrViolation;
    use logicaffeine_compile::spatial::passes::compose::{compose_phased, SectionPlacement};
    use logicaffeine_compile::spatial::pipeline::compile_to_spatial;
    let one = |w: usize| {
        let mut s = String::from("## Main\n");
        s += &declare("x", 1, 64, 1);
        s += &declare(&format!("W{w}"), 64, 64, 10 * w);
        s += &format!("Let y{w} be x matmul W{w}.\n");
        compile_to_spatial(&s, &sn50()).unwrap_or_else(|e| panic!("a product lowers: {e:?}"))
    };
    let (a, b) = (one(1), one(2));
    // Both on the sites their own placements chose — the same ones — and no barrier between them: one phase.
    let two = compose_phased(
        vec![a, b.clone()],
        &[],
        &[],
        SectionPlacement::Stacked,
        Some(&[false]),
        &sn50(),
    )
    .expect("composes");
    assert!(two.sections[0].barrier_after.is_none(), "one phase");
    let (s0, s1) = (sites_of(&two, 0), sites_of(&two, 1));
    let site = *s0
        .intersection(&s1)
        .next()
        .expect("the two sections share a site");
    two.validate()
        .expect("composition laid the phase's sections one past another on the site they share");
    // The second section's pages back where it laid them out alone: over the first's.
    let mut overlapped = two.clone();
    let (b0, _) = overlapped.sections[1].buffers;
    let alone = &b.memory.as_ref().expect("laid out").buffer_at;
    overlapped.memory.as_mut().expect("laid out").buffer_at[b0..b0 + alone.len()]
        .copy_from_slice(alone);
    match overlapped.validate() {
        Err(IrViolation::MemoryOverlap { site: at, .. }) => assert_eq!(at, site, "the refusal names the site"),
        other => panic!("two sections of one phase over the same bytes of {site:?} must be refused, got {other:?}"),
    }
}

// ───────────────────────────── 2. a phase fits the mesh ─────────────────────────────

#[test]
fn a_phase_never_holds_more_sites_than_the_mesh() {
    let one = composed(&products(false), &sn50());
    let need = sites_of(&one, 0).len();
    // A mesh that holds two products' sites but not three.
    let small = sn50().with_sites(2 * need, 1);
    let p = composed(&products(false), &small);
    let ph = phases(&p);
    assert_eq!(
        ph.len(),
        2,
        "three products of {need} sites each on a mesh of {}: two phases, {ph:?}",
        2 * need
    );
    for phase in &ph {
        let used: usize = phase.iter().map(|k| sites_of(&p, *k).len()).sum();
        assert!(
            used <= 2 * need,
            "a phase holds {used} sites on a mesh of {}",
            2 * need
        );
    }
}

// ───────────────────────────── 3. packed programs compute ─────────────────────────────

#[test]
fn packed_programs_compute_the_vms_values() {
    for chained in [false, true] {
        let src = products(chained);
        let vm = vm_outcome(&src);
        assert_eq!(vm.error, None, "{:?}", vm.error);
        let want = oracle_ints(&vm.output).expect("ints");
        let scale = want.iter().map(|w| w.abs()).max().unwrap_or(1) as f64;
        for (target, chip) in [(fp32_twin(&sn50()), false), (sn50(), true)] {
            let p = composed(&src, &target);
            let got = shown_from_outputs(
                &src,
                &p,
                &eval_choreography(&p, &inputs_from_fills(&p).expect("fills")).expect("evaluates"),
            )
            .expect("shown");
            // WIRING: the fp32 twin within 2⁻¹⁶ of each value; CHIP: bf16 within `sections + 1` ulps of the values' scale.
            for (i, (g, w)) in got.iter().zip(&want).enumerate() {
                let bound = if chip {
                    (p.sections.len() as f64 + 1.0) * scale * 2f64.powi(-8)
                } else {
                    ((*w).abs() as f64 / 65536.0).max(1.0)
                };
                assert!(
                    ((g - w).abs() as f64) <= bound,
                    "{} chained={chained} Show #{}: {g} vs VM {w} (bound {bound:.0})",
                    if chip { "CHIP" } else { "WIRING" },
                    i + 1
                );
            }
        }
    }
}

// ───────────────────────────── 4. at real width ─────────────────────────────

#[test]
fn a_real_width_decode_layer_runs_its_independent_sections_at_once() {
    use logicaffeine_compile::codegen_cuda::qwen38::{gen_qwen38_decode_layer_lg, Qwen38Cfg};
    let mut cfg = Qwen38Cfg::real();
    cfg.layers = 1;
    cfg.vocab = 512;
    let (src, _, _) = gen_qwen38_decode_layer_lg(&cfg, 0, "/w", 4096, 1);
    let p = composed(&src, &sn50());
    let ph = phases(&p);
    assert!(
        ph.len() < p.sections.len(),
        "the layer's {} sections run in fewer phases, got {}",
        p.sections.len(),
        ph.len()
    );
    let projections: BTreeSet<usize> = (0..p.sections.len())
        .filter(|k| {
            p.sections[*k].name.starts_with("matmul_1x") && p.sections[*k].name.ends_with("x5120")
        })
        .collect();
    let phase_of = |k: usize| {
        ph.iter()
            .position(|phase| phase.contains(&k))
            .expect("every section is in a phase")
    };
    let wide: BTreeSet<usize> = projections
        .iter()
        .filter(|k| {
            p.sections[**k].name.contains("x2048x") || p.sections[**k].name.contains("x6144x")
        })
        .map(|k| phase_of(*k))
        .collect();
    assert_eq!(
        wide.len(),
        1,
        "the four projections of the normalized row share a phase: {:?}",
        projections
            .iter()
            .map(|k| (&p.sections[*k].name, phase_of(*k)))
            .collect::<Vec<_>>()
    );
    println!(
        "real-width decode layer 0: {} sections in {} phases",
        p.sections.len(),
        ph.len()
    );
}

#[test]
fn a_write_waits_for_the_reads_before_it() {
    let mut src = String::from("## Main\n");
    src += &declare("x", 1, 64, 1);
    src += &declare("v", 1, 64, 2);
    src += &declare("W1", 64, 64, 10);
    src += &declare("W2", 64, 64, 20);
    src +=
        "Let pos be 1.\nLet y1 be x matmul W1.\nSet row pos of x to v.\nLet y2 be x matmul W2.\n";
    for out in ["y1", "y2"] {
        for c in [1, 17, 64] {
            src += &format!("Show round(item {c} of {out} * 1000000).\n");
        }
    }
    let p = composed(&src, &sn50());
    let ph = phases(&p);
    let phase_of = |name: &str| {
        ph.iter()
            .position(|phase| phase.iter().any(|k| p.sections[*k].name.contains(name)))
            .unwrap_or_else(|| {
                panic!(
                    "a `{name}` section: {:?}",
                    p.sections.iter().map(|s| &s.name).collect::<Vec<_>>()
                )
            })
    };
    let (read, write) = (
        ph.iter()
            .position(|phase| phase.contains(&0))
            .expect("the first product's phase"),
        phase_of("set_row"),
    );
    assert!(
        write > read,
        "the row write lands after the product that reads `x`: phases {ph:?}"
    );
    let vm = vm_outcome(&src);
    assert_eq!(vm.error, None, "{:?}", vm.error);
    let want = oracle_ints(&vm.output).expect("ints");
    let got = shown_from_outputs(
        &src,
        &p,
        &eval_choreography(&p, &inputs_from_fills(&p).expect("fills")).expect("evaluates"),
    )
    .expect("shown");
    let scale = want.iter().map(|w| w.abs()).max().unwrap_or(1) as f64;
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        let bound = (p.sections.len() as f64 + 1.0) * scale * 2f64.powi(-8);
        assert!(
            ((g - w).abs() as f64) <= bound,
            "Show #{}: {g} vs VM {w} — y1 reads the old row, y2 the new",
            i + 1
        );
    }
}
