//! The tiling search stated as a SPECIFICATION, so the planner can be replaced without being trusted.
//!
//! `codegen_tt::tune_tiling` and `spatial::passes::tile::candidates` search the same object — the divisor
//! lattice of a GEMM's tile counts, filtered by what the part can hold. The P5 salvage retires the former
//! in favour of the latter, and the whole risk in that swap is that the two planners quietly disagree:
//! a different tiling is not a compile error, it is a slower kernel that nobody attributes to the change.
//!
//! So this file does not pin the implementation's answer and call that a test. It restates the search from
//! the DOCUMENTED definition — the lattice, the three legality rules, and the ranking model — and requires
//! `tune_tiling` to return a point that is legal and cost-minimal *under that independent restatement*.
//! `tiling_cost` and `l1_block_tiles` are private to the implementation on purpose; the reference below is
//! written from their doc comments, never from their bodies. If the two ever disagree, the instruction is
//! the one `spatial_algebra_semantics_lock` already carries: FIX THE COMPILER, NEVER THE REFERENCE.
//!
//! What that buys the salvage: a bridge to the spatial planner that searches the same lattice under the same
//! rules passes this file by construction, and one that changes the ranking fails it loudly and by name —
//! which is the measurement, taken before the change rather than after a regression.
//!
//! NON-VACUITY PROVEN: `a_perturbed_tiling_is_caught` walks each returned config off the lattice and off the
//! optimum and requires every rung to reject it, so a rule that stopped discriminating cannot pass silently.

use logicaffeine_compile::codegen_tt::matmul_lower::{
    tune_tiling, tune_tiling_with, ChipKnobs, ComputeKnobs, MathFidelity, TileConfig,
};
use logicaffeine_compile::tensor_recognize::MatmulSpec;

/// Shapes with real provenance: the calibrated 512³ the ledger measures at 4986 cycles, the P4a row shapes,
/// a prime-ish tile count whose lattice is nearly trivial, and sizes that outgrow one Wormhole grid.
const CORPUS: &[(usize, usize, usize)] = &[
    (32, 32, 32),
    (64, 64, 64),
    (128, 128, 128),
    (256, 256, 256),
    (512, 512, 512),
    (1024, 1024, 1024),
    (2048, 2048, 2048),
    (224, 224, 224),
    (512, 256, 1024),
    (1024, 512, 256),
    (96, 160, 320),
    (32, 2048, 64),
    (2048, 32, 64),
];

fn spec(m: usize, n: usize, k: usize) -> MatmulSpec {
    MatmulSpec {
        m,
        n,
        k,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    }
}

fn chips() -> Vec<ChipKnobs> {
    vec![ChipKnobs::wormhole(), ChipKnobs::blackhole()]
}

fn knob_set() -> Vec<ComputeKnobs> {
    vec![
        ComputeKnobs::default(),
        ComputeKnobs {
            fidelity: MathFidelity::HiFi4,
            fp32_dest_acc: false,
            packer_l1_acc: false,
            pcc_floor: 0.99,
        },
        ComputeKnobs {
            fidelity: MathFidelity::HiFi4,
            fp32_dest_acc: true,
            packer_l1_acc: false,
            pcc_floor: 0.99,
        },
        ComputeKnobs {
            fidelity: MathFidelity::LoFi,
            fp32_dest_acc: true,
            packer_l1_acc: false,
            pcc_floor: 0.9,
        },
    ]
}

// ── The independent reference — written from the doc comments, never from the bodies ────────────────────

fn divisors(n: usize) -> Vec<usize> {
    (1..=n).filter(|d| n % d == 0).collect()
}

/// "Rough L1 footprint (in tiles) of the resident double-buffered A/B blocks" — the A and B panels a core
/// holds while it streams one inner block, double-buffered, plus its resident output tiles.
fn ref_l1_block_tiles(c: &TileConfig) -> usize {
    2 * (c.per_core_m * c.in0_block_w + c.per_core_n * c.in0_block_w) + c.per_core_m * c.per_core_n
}

/// "A heuristic RANKING model, not an absolute cycle count", capturing exactly three levers:
/// **parallelism** (per-core output work, dominant), **reuse** (DST acquires per core), **reload**
/// (the spill/reload traffic that only exists once `Kt` is blocked).
fn ref_tiling_cost(kt: usize, c: &TileConfig) -> u64 {
    let per_core_out = (c.per_core_m * c.per_core_n) as u64;
    let num_blocks = (kt / c.in0_block_w) as u64;
    let acquires = (c.per_core_m / c.out_subblock_h) as u64
        * (c.per_core_n / c.out_subblock_w) as u64
        * num_blocks;
    let reload = if num_blocks > 1 { acquires } else { 0 };
    per_core_out * 1_000_000 + acquires * 100 + reload
}

/// "Rough L1 tile budget: bf16 tile = 2 KiB, so ~ l1_kib/2 tiles, held back for output/overhead."
fn ref_l1_budget(chip: &ChipKnobs) -> usize {
    (chip.l1_kib / 2).saturating_sub(8)
}

/// The three legality rules the doc comment names: "grid fits, DST holds the sub-block, blocks fit L1" —
/// over the lattice "`per_core_m | Mt`, `per_core_n | Nt`, `out_subblock_h | per_core_m`,
/// `out_subblock_w | per_core_n`, `in0_block_w | Kt`".
fn ref_is_legal(
    c: &TileConfig,
    s: &MatmulSpec,
    chip: &ChipKnobs,
    dst_budget: usize,
) -> Result<(), String> {
    let (mt, nt, kt) = (s.mt(), s.nt(), s.kt());
    let on_lattice = c.per_core_m != 0
        && c.per_core_n != 0
        && c.in0_block_w != 0
        && c.out_subblock_h != 0
        && c.out_subblock_w != 0
        && mt % c.per_core_m == 0
        && nt % c.per_core_n == 0
        && kt % c.in0_block_w == 0
        && c.per_core_m % c.out_subblock_h == 0
        && c.per_core_n % c.out_subblock_w == 0;
    if !on_lattice {
        return Err(format!(
            "{c:?} is off the divisor lattice of ({mt},{nt},{kt})"
        ));
    }
    if c.grid_y != mt / c.per_core_m || c.grid_x != nt / c.per_core_n {
        return Err(format!(
            "{c:?} states a grid that is not the block count of its own per-core tile"
        ));
    }
    if c.grid_y > chip.grid_y || c.grid_x > chip.grid_x {
        return Err(format!(
            "{c:?} needs a {}x{} grid; {} has {}x{}",
            c.grid_x, c.grid_y, chip.name, chip.grid_x, chip.grid_y
        ));
    }
    if c.out_subblock_h * c.out_subblock_w > dst_budget {
        return Err(format!(
            "{c:?} needs {} DST tiles; the budget is {dst_budget}",
            c.dst_tiles()
        ));
    }
    if ref_l1_block_tiles(c) > ref_l1_budget(chip) {
        return Err(format!(
            "{c:?} needs {} L1 tiles; the budget is {}",
            ref_l1_block_tiles(c),
            ref_l1_budget(chip)
        ));
    }
    Ok(())
}

/// The whole legal lattice, enumerated independently of the implementation's loop order.
fn ref_lattice(s: &MatmulSpec, chip: &ChipKnobs, dst_budget: usize) -> Vec<TileConfig> {
    let (mt, nt, kt) = (s.mt(), s.nt(), s.kt());
    let mut out = Vec::new();
    for pcm in divisors(mt) {
        for pcn in divisors(nt) {
            for osh in divisors(pcm) {
                for osw in divisors(pcn) {
                    for ibw in divisors(kt) {
                        let c = TileConfig {
                            grid_x: nt / pcn,
                            grid_y: mt / pcm,
                            per_core_m: pcm,
                            per_core_n: pcn,
                            in0_block_w: ibw,
                            out_subblock_h: osh,
                            out_subblock_w: osw,
                        };
                        if ref_is_legal(&c, s, chip, dst_budget).is_ok() {
                            out.push(c);
                        }
                    }
                }
            }
        }
    }
    out
}

// ── The obligations ─────────────────────────────────────────────────────────────────────────────────────

/// Rung 1 — whatever the planner returns must be a point a core can actually run: on the lattice, inside the
/// grid, inside DST, inside L1. This is the rung that survives ANY replacement planner unchanged, because it
/// is a property of the hardware rather than of the search.
#[test]
fn every_tuned_tiling_is_a_legal_lattice_point() {
    let mut checked = 0usize;
    for &(m, n, k) in CORPUS {
        let s = spec(m, n, k);
        for chip in chips() {
            for knobs in knob_set() {
                let cfg = tune_tiling_with(&s, &chip, &knobs);
                let budget = knobs.dst_tiles(&chip);
                ref_is_legal(&cfg, &s, &chip, budget).unwrap_or_else(|e| {
                    panic!(
                        "tune_tiling returned an ILLEGAL tiling for {m}x{n}x{k} on {}: {e}",
                        chip.name
                    )
                });
                checked += 1;
            }
        }
    }
    assert_eq!(
        checked,
        CORPUS.len() * 2 * 4,
        "the corpus sweep must be complete, not sampled"
    );
    assert!(
        checked >= 100,
        "a legality rung this cheap has no excuse to be narrow (was {checked})"
    );
}

/// Rung 2 — THE SPECIFICATION. The planner must return a cost-MINIMAL point over the independently
/// enumerated legal lattice. Equality of cost, not of config: ties are broken by iteration order, which is
/// an implementation detail and deliberately not pinned here, so a planner that enumerates the lattice in a
/// different order still passes. What it may not do is return a point some other legal point beats.
#[test]
fn the_tuned_tiling_is_cost_minimal_over_the_whole_legal_lattice() {
    for &(m, n, k) in CORPUS {
        let s = spec(m, n, k);
        for chip in chips() {
            for knobs in knob_set() {
                let budget = knobs.dst_tiles(&chip);
                let lattice = ref_lattice(&s, &chip, budget);
                assert!(
                    !lattice.is_empty(),
                    "the lattice always contains the trivial point; {m}x{n}x{k} on {} found none",
                    chip.name
                );
                let best = lattice
                    .iter()
                    .map(|c| ref_tiling_cost(s.kt(), c))
                    .min()
                    .expect("non-empty");
                let cfg = tune_tiling_with(&s, &chip, &knobs);
                let got = ref_tiling_cost(s.kt(), &cfg);
                assert_eq!(
                    got, best,
                    "tune_tiling picked a DOMINATED tiling for {m}x{n}x{k} on {} (fp32={}): {cfg:?} costs \
                     {got}, but a legal point costs {best}. The reference is the specification — fix the \
                     planner, never this file.",
                    chip.name, knobs.fp32_dest_acc
                );
            }
        }
    }
}

/// Rung 3 — the chip knobs must actually steer the search. A planner that ignored them would pass both rungs
/// above on Wormhole alone; this requires the larger part to be at least as good everywhere, and strictly
/// better somewhere, which is the claim `ChipKnobs`' own doc comment makes ("the SAME matmul compiles to a
/// DIFFERENT (faster) kernel on a different part").
#[test]
fn a_larger_part_is_never_tuned_worse_and_is_sometimes_tuned_better() {
    let (wh, bh) = (ChipKnobs::wormhole(), ChipKnobs::blackhole());
    let mut strictly_better = 0usize;
    for &(m, n, k) in CORPUS {
        let s = spec(m, n, k);
        let cw = ref_tiling_cost(s.kt(), &tune_tiling(&s, &wh));
        let cb = ref_tiling_cost(s.kt(), &tune_tiling(&s, &bh));
        assert!(
            cb <= cw,
            "blackhole tuned WORSE than wormhole on {m}x{n}x{k}: {cb} vs {cw}"
        );
        if cb < cw {
            strictly_better += 1;
        }
    }
    assert!(
        strictly_better > 0,
        "the chip knobs never changed the answer — the search is ignoring them"
    );
}

/// Rung 4 — fp32 dest-accumulate halves the DST budget, so it can never unlock a sub-block bf16 could not
/// hold. This is the one knob interaction that silently over-promises if it is dropped: an emitted kernel
/// claiming fp32 accumulation while sized for the bf16 register file overruns DST on the part.
#[test]
fn fp32_dest_accumulate_never_widens_the_sub_block() {
    let bf16 = ComputeKnobs::default();
    let fp32 = ComputeKnobs {
        fp32_dest_acc: true,
        ..ComputeKnobs::default()
    };
    for &(m, n, k) in CORPUS {
        let s = spec(m, n, k);
        for chip in chips() {
            let wide = tune_tiling_with(&s, &chip, &bf16).dst_tiles();
            let narrow = tune_tiling_with(&s, &chip, &fp32).dst_tiles();
            assert!(
                narrow <= wide,
                "fp32 dest-acc widened the sub-block on {m}x{n}x{k}/{}: {narrow} > {wide}",
                chip.name
            );
            assert!(
                narrow <= fp32.dst_tiles(&chip),
                "the fp32 tiling for {m}x{n}x{k}/{} needs {narrow} DST tiles but only {} are addressable",
                chip.name,
                fp32.dst_tiles(&chip)
            );
        }
    }
}

/// NON-VACUITY. Every rung above must actually reject something. Each returned config is walked off the
/// lattice and off the optimum, and the reference is required to catch each mutation — otherwise the rungs
/// are shaped like assertions but discriminate nothing.
#[test]
fn a_perturbed_tiling_is_caught() {
    let chip = ChipKnobs::wormhole();
    let knobs = ComputeKnobs::default();
    let budget = knobs.dst_tiles(&chip);
    let mut off_lattice = 0usize;
    let mut dominated = 0usize;
    let mut over_grid = 0usize;

    for &(m, n, k) in CORPUS {
        let s = spec(m, n, k);
        let cfg = tune_tiling_with(&s, &chip, &knobs);
        let best = ref_tiling_cost(s.kt(), &cfg);

        // Off the lattice: a per-core height that no longer divides Mt.
        if s.mt() % (cfg.per_core_m + 1) != 0 {
            let bad = TileConfig {
                per_core_m: cfg.per_core_m + 1,
                ..cfg
            };
            assert!(
                ref_is_legal(&bad, &s, &chip, budget).is_err(),
                "an off-lattice {bad:?} was accepted"
            );
            off_lattice += 1;
        }
        // Over the grid: claim more cores than the part has.
        let bad = TileConfig {
            grid_x: chip.grid_x + 1,
            ..cfg
        };
        assert!(
            ref_is_legal(&bad, &s, &chip, budget).is_err(),
            "an over-grid {bad:?} was accepted"
        );
        over_grid += 1;

        // Dominated: shrink the sub-block to 1x1, which is always on the lattice and never cheaper.
        let worse = TileConfig {
            out_subblock_h: 1,
            out_subblock_w: 1,
            ..cfg
        };
        if worse != cfg {
            assert!(
                ref_tiling_cost(s.kt(), &worse) >= best,
                "a 1x1 sub-block beat the tuned one on {m}x{n}x{k} — the ranking model is inverted"
            );
            if ref_tiling_cost(s.kt(), &worse) > best {
                dominated += 1;
            }
        }
    }
    assert!(
        off_lattice >= 5,
        "the lattice rule was exercised only {off_lattice} times"
    );
    assert!(
        over_grid >= CORPUS.len(),
        "the grid rule must be exercised on every shape"
    );
    assert!(
        dominated >= 3,
        "the ranking model never separated two points ({dominated}) — it discriminates nothing"
    );
}
