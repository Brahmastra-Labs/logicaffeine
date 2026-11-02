//! The calibrated cycle model — what makes "streamed vs multicast vs blocked" an EMERGENT plan decision.
//!
//! Every quantity is computed FROM a candidate plan's structure (tile-MACs, DRAM tile reads, circular-buffer
//! hand-offs, multicast blocks, semaphores, kernel classes); the model turns them into predicted device cycles
//! with a handful of constants. On Wormhole those constants are FITTED to the rows of the ttsim GEMM ledger
//! (`benchmarks/silicon/tenstorrent_gemm.json`, written only by `scripts/tt-gemm-ledger.py` from real runs —
//! never typed), within matched-knob groups only. Other targets carry an `uncalibrated` model whose absolute
//! numbers are never asserted — only its structural monotonicity is. No strategy enum exists anywhere: the
//! `tile` pass enumerates candidate plans and keeps the minimum of this function, and `fuse_readers`
//! re-derives the same comparison for the fused/unfused pair.

use serde::{Deserialize, Serialize};

use super::ir::{AccPolicy, BlockPlan, BlockShape, EnginePacking, Granularity, InnerLoop};
use super::target::{Fidelity, MeshShape, SpatialTarget};

/// Path of the ttsim GEMM ledger, relative to the repo root.
pub const CALIBRATION_JSON: &str = "benchmarks/silicon/tenstorrent_gemm.json";

/// Per-cycle-model constants. Names mirror the terms of [`CycleModel::cycles`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CycleModel {
    /// Fitted against measured points (true) or structural placeholders (false).
    pub calibrated: bool,
    pub c_kernel: f64,
    /// Per-semaphore set-up. Collinear with `c_kernel` in the LOSS — and restoring it anyway, because
    /// `n_sems` is what separates a multicast plan from a non-multicast one, so this is the only term that
    /// prices the choice. Dropping it on loss-redundancy grounds moved the 512³ GEMM off its measured
    /// multicast plan and cost 4% of its cycles. See the note on `unexercised`.
    pub c_sem_setup: f64,
    pub c_site_setup: f64,
    /// Cycles per tile-MAC at HiFi4; the other fidelities scale it.
    pub c_mac: f64,
    pub f_hifi3: f64,
    pub f_hifi2: f64,
    pub f_lofi: f64,
    pub f_fp32: f64,
    pub c_acq: f64,
    pub c_reload: f64,
    /// Cycles the packer spends per tile. The packer and the math unit are separate pipelines that overlap
    /// across sub-blocks, so the engine's time is the LONGER of the two, not their sum.
    pub c_pack: f64,
    /// Cycles a packer-accumulate drain round-trip costs the compute engine.
    pub c_drain: f64,
    pub c_dram_tile: f64,
    /// Cycles of shared DRAM occupancy per tile of WHOLE-PROGRAM traffic.
    ///
    /// ★ This looks ~87x too small against Wormhole's DRAM roof
    /// ([`Roofs::wormhole`](super::roofs::Roofs::wormhole): a 2048 B tile against 288 B/cycle is ~7.1
    /// cycles/tile, not 0.08), and pricing it that way makes the planner prefer a multicast plan moving 768
    /// tiles over the blocked one it picks today at 4,352 — a 5.7x traffic difference.
    ///
    /// It is deliberately NOT changed, because the multicast plan that pricing selects DOES NOT RUN:
    /// forced at 128³ it fails to complete under ttsim in nine minutes, where the blocked plan at the same
    /// shape finishes in seconds at PCC 1.0. This constant was fitted to MEASUREMENTS, so what looks like
    /// mispricing is partly the fit correctly encoding that our multicast lowering is catastrophic.
    ///
    /// Fix the multicast hang first; then bandwidth-derived pricing is safe and unlocks roughly 4x less
    /// DRAM traffic than ttnn.
    pub c_dram_shared: f64,
    pub c_cb: f64,
    /// Per-multicast-block synchronization: the semaphore round trip and the multicast set-up, which the
    /// model can only ever charge TOGETHER — a block pays both or neither, so no measurement can separate
    /// them and carrying two constants would leave one direction of the fit unconstrained.
    pub c_mcast_block: f64,
    pub c_noc_tile: f64,
}

/// Which resource sets the pipeline's pace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaceBound {
    Compute,
    Movement(u8),
    Sync,
}

/// A plan's cost, term by term, with the initiation interval its steady state runs at.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CostBreakdown {
    pub setup: f64,
    pub compute: f64,
    pub move_engine: (f64, f64),
    pub sync: f64,
    pub fill: f64,
    pub drain: f64,
    pub steady: f64,
    /// Cycles between successive iterations of the steady-state loop.
    pub ii: f64,
    pub bound: PaceBound,
    pub total: f64,
}

/// The structural quantities of one candidate plan (per the busiest site, plus whole-program totals).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanQuantities {
    /// The problem in tiles `(mt, nt, kt)`; points that share a problem, `fp32_acc` and `fidelity` are
    /// measured alternatives of one another and must be ranked as the simulator ranked them.
    pub problem: (usize, usize, usize),
    pub sites_used: usize,
    pub kernel_classes: usize,
    pub n_sems: usize,
    pub tile_macs: u64,
    pub acquires: u64,
    pub reload_tiles: u64,
    pub dram_tiles_site: u64,
    /// The site's DRAM traffic split across its movement engines, which run CONCURRENTLY: the engine with
    /// the most work sets the pace, so the packing that balances them wins. One entry per engine.
    pub dram_tiles_engine: (u64, u64),
    /// Tiles the packer writes (every spill and the final output).
    pub pack_tiles: u64,
    /// Tile-MACs of ONE sub-block: the math the compute pipeline must finish before the packer has anything
    /// to overlap with, so a bigger sub-block costs more fill however few of them there are.
    pub fill_macs: u64,
    /// Page round-trips the packer's in-place accumulate needs to return its write pointer.
    pub drains: u64,
    /// The packer and the math unit overlap. They can only do so when a sub-block fits in HALF the register
    /// file, because the engine works one half while the packer drains the other; a sub-block that fills the
    /// whole file forces the two to take turns.
    pub pack_overlaps: bool,
    pub dram_tiles_all: u64,
    pub cb_handoffs: u64,
    pub mcast_blocks: u64,
    pub mcast_tiles: u64,
    /// Prologue tiles per movement engine: the first block's operands, which the engines read CONCURRENTLY.
    /// Summing them would charge a split arrangement for a serial read it never performs.
    pub fill_tiles_engine: (u64, u64),
    pub fill_tiles: u64,
    pub drain_tiles: u64,
    pub fp32_acc: bool,
    pub fidelity: Fidelity,
    /// Steady-state iterations the site runs: one per K block per work item. The pipeline's cost is its
    /// initiation interval times this, so a term that does not scale with it is setup, not throughput.
    pub iterations: u64,
}

impl CycleModel {
    /// Structural placeholder constants (never asserted absolutely).
    pub fn uncalibrated() -> CycleModel {
        CycleModel {
            calibrated: false,
            c_kernel: 1250.0,
            c_sem_setup: 50.0,
            c_site_setup: 0.0,
            c_mac: 6.5,
            f_hifi3: 0.8,
            f_hifi2: 0.6,
            f_lofi: 0.4,
            f_fp32: 1.1,
            c_acq: 26.1824650701,
            c_reload: 7.8319197229,
            c_pack: 1.0,
            c_drain: 30.0,
            c_dram_tile: 18.0,
            c_dram_shared: 0.0815085028,
            c_cb: 1.5,
            c_mcast_block: 310.0,
            c_noc_tile: 2.5,
        }
    }

    /// The Wormhole model, fitted by [`fit`](Self::fit) over the Wormhole rows of
    /// `benchmarks/silicon/tenstorrent_gemm.json` at the knobs the compiler emits (HiFi4, fp32 accumulate
    /// on and off). These constants reproduce NINETEEN OF TWENTY of the ledger's compiler-chosen plans,
    /// which is the property that actually matters — see the note on
    /// [`unexercised`](Self::unexercised) for why a lower-loss set of constants is not automatically a
    /// better model.
    ///
    /// `f_hifi3`, `f_hifi2`, `f_lofi`, `c_mcast_block` and `c_noc_tile` sit at their structural priors and
    /// must stay there: every measured row is HiFi4, and `steady` is a `max` whose synchronization branch
    /// never binds on any of them, so nothing in this ledger can see those five.
    /// `spatial_cost_identifiability_lock` enforces it.
    pub fn wormhole_ttsim() -> CycleModel {
        CycleModel {
            calibrated: true,
            c_kernel: 783.2813960073,
            c_sem_setup: 106.1814811064,
            c_site_setup: 2.4232492248,
            c_mac: 4.0014415764,
            f_hifi3: 0.8,
            f_hifi2: 0.6,
            f_lofi: 0.4,
            f_fp32: 1.1,
            c_acq: 26.1824650701,
            c_reload: 7.8319197229,
            c_pack: 1.0,
            c_drain: 30.0,
            c_dram_tile: 20.4947563410,
            // Fitted, not the DRAM roof: see the field's note.
            c_dram_shared: 0.0815085028,
            c_cb: 0.0,
            c_mcast_block: 310.0,
            c_noc_tile: 2.5,
        }
    }

    /// Every constant, named, paired with its slot — the ONE place the fit's index order is defined.
    /// [`constants`](Self::constants) and [`set`](Self::set) both derive from this, so an index can never
    /// mean one constant to the reader and another to the writer.
    fn fields_mut(&mut self) -> Vec<(&'static str, &mut f64)> {
        vec![
            ("c_kernel", &mut self.c_kernel),
            ("c_sem_setup", &mut self.c_sem_setup),
            ("c_site_setup", &mut self.c_site_setup),
            ("c_mac", &mut self.c_mac),
            ("f_hifi3", &mut self.f_hifi3),
            ("f_hifi2", &mut self.f_hifi2),
            ("f_lofi", &mut self.f_lofi),
            ("f_fp32", &mut self.f_fp32),
            ("c_acq", &mut self.c_acq),
            ("c_reload", &mut self.c_reload),
            ("c_pack", &mut self.c_pack),
            ("c_drain", &mut self.c_drain),
            ("c_dram_tile", &mut self.c_dram_tile),
            ("c_dram_shared", &mut self.c_dram_shared),
            ("c_cb", &mut self.c_cb),
            ("c_mcast_block", &mut self.c_mcast_block),
            ("c_noc_tile", &mut self.c_noc_tile),
        ]
    }

    /// The constants by name, in the fit's index order.
    pub fn constants(&self) -> Vec<(&'static str, f64)> {
        let mut c = self.clone();
        c.fields_mut().into_iter().map(|(n, v)| (n, *v)).collect()
    }

    fn set(&mut self, i: usize, v: f64) {
        *self.fields_mut().swap_remove(i).1 = v;
    }

    /// Constant `i` changed to `v` — the perturbation the identifiability lock searches with.
    pub fn with_constant(&self, i: usize, v: f64) -> CycleModel {
        let mut m = self.clone();
        m.set(i, v);
        m
    }

    fn fidelity_factor(&self, f: Fidelity) -> f64 {
        match f {
            Fidelity::HiFi4 | Fidelity::Exact => 1.0,
            Fidelity::HiFi3 => self.f_hifi3,
            Fidelity::HiFi2 => self.f_hifi2,
            Fidelity::LoFi => self.f_lofi,
        }
    }

    /// The compute engine's time for a plan: the math unit and the packer pipeline against each other
    /// across sub-blocks, so the engine runs at the pace of the busier one; only the FIRST sub-block's math
    /// has nothing to overlap with, which is what makes many small sub-blocks beat few large ones. The
    /// acquire/release handshake belongs to the PACK side: when a sub-block fits in half the register file
    /// the two halves pipeline and the handshake hides behind the math, and when it does not the whole thing
    /// serialises and every handshake is exposed.
    pub fn compute_cycles(&self, q: &PlanQuantities) -> f64 {
        let fp32 = if q.fp32_acc { self.f_fp32 } else { 1.0 };
        let mac = self.c_mac * self.fidelity_factor(q.fidelity) * fp32;
        let math = q.tile_macs as f64 * mac;
        let pack = q.pack_tiles as f64 * self.c_pack + q.acquires as f64 * self.c_acq;
        let engine = if q.pack_overlaps {
            math.max(pack)
        } else {
            math + pack
        };
        engine
            + q.fill_macs as f64 * mac
            + q.reload_tiles as f64 * self.c_reload
            + q.drains as f64 * self.c_drain
    }

    /// The movement engines' time: they run concurrently, so the busier one sets the pace, not their sum.
    pub fn move_cycles(&self, q: &PlanQuantities) -> f64 {
        let engine_tiles = q.dram_tiles_engine.0.max(q.dram_tiles_engine.1);
        engine_tiles as f64 * self.c_dram_tile
            + q.dram_tiles_all as f64 * self.c_dram_shared
            + q.cb_handoffs as f64 * self.c_cb
    }

    /// Predicted whole-program device cycles for a candidate plan.
    /// The model's terms, named — what the plan spends and which resource sets its pace.
    ///
    /// A spatial plan's cost is not one number but a pipeline: each engine has a rate, and the slowest of
    /// them is the initiation interval the steady state runs at. Reporting the terms is what lets the
    /// compiler say a plan is movement-bound on its second engine rather than merely quoting a total.
    pub fn explain(&self, q: &PlanQuantities) -> CostBreakdown {
        let iters = q.iterations.max(1) as f64;
        let t_compute = self.compute_cycles(q);
        let t_move_0 = q.dram_tiles_engine.0 as f64 * self.c_dram_tile;
        let t_move_1 = q.dram_tiles_engine.1 as f64 * self.c_dram_tile;
        let t_shared =
            q.dram_tiles_all as f64 * self.c_dram_shared + q.cb_handoffs as f64 * self.c_cb;
        let t_sync =
            q.mcast_blocks as f64 * self.c_mcast_block + q.mcast_tiles as f64 * self.c_noc_tile;
        // No free intercept. Every program dispatches at least one kernel class, so a constant per PROGRAM
        // and a constant per KERNEL CLASS are the same column whenever the measured rows share a class
        // count — and the ledger's do. Removing it is safe for a reason that has nothing to do with the fit:
        // an intercept is IDENTICAL across every candidate of one program, so it can never change which
        // plan wins. `c_sem_setup` is the opposite and stays: `n_sems` is what distinguishes a multicast
        // plan from a non-multicast one, so it is the only term that prices that choice.
        let setup = self.c_kernel * q.kernel_classes as f64
            + self.c_sem_setup * q.n_sems as f64
            + self.c_site_setup * q.sites_used as f64;
        let fill = q.fill_tiles_engine.0.max(q.fill_tiles_engine.1) as f64 * self.c_dram_tile;
        let drain = q.drain_tiles as f64 * self.c_dram_tile;
        let steady = t_compute.max(t_move_0 + t_shared).max(t_move_1).max(t_sync);
        let bound = if steady == t_compute {
            PaceBound::Compute
        } else if steady == t_sync {
            PaceBound::Sync
        } else if steady == t_move_1 {
            PaceBound::Movement(1)
        } else {
            PaceBound::Movement(0)
        };
        // A pipelined loop costs its prologue read, then its initiation interval once per iteration — the
        // LAST block's compute still follows its own read, so the fill is a prologue on top of every
        // iteration rather than a substitute for one — then its drain.
        let ii = steady / iters;
        CostBreakdown {
            setup,
            compute: t_compute,
            move_engine: (t_move_0 + t_shared, t_move_1),
            sync: t_sync,
            fill,
            drain,
            steady,
            ii,
            bound,
            total: setup + fill + steady + drain,
        }
    }

    pub fn cycles(&self, q: &PlanQuantities) -> f64 {
        self.explain(q).total
    }

    /// Sum of squared relative errors over measured points.
    pub fn loss(&self, points: &[(PlanQuantities, u64)]) -> f64 {
        points
            .iter()
            .map(|(q, measured)| {
                let e = (self.cycles(q) - *measured as f64) / *measured as f64;
                e * e
            })
            .sum()
    }

    /// How much worse the data term must get when a constant is frozen wrong, for that constant to count as
    /// MEASURED. Below this, the rest of the model absorbs it and its value is an artifact of the search.
    pub const DETERMINED_MARGIN: f64 = 1e-3;

    /// The canonical point at which identifiability is judged: the unconstrained minimizer of the data
    /// term. Determinedness is a property of a model AND a point — asking the question at two different
    /// points gives two different answers — so every caller must ask it here.
    pub fn reference(points: &[(PlanQuantities, u64)]) -> CycleModel {
        let prior = CycleModel::uncalibrated();
        let regularized = CycleModel::descend(prior.clone(), points, Some(&prior), &[]);
        CycleModel::fit_data(points, &regularized, &[])
    }

    /// How many contiguous folds the ledger is split into for the stability report, and how far a constant
    /// may move when one fold is withheld before it is worth mentioning.
    pub const JACKKNIFE_FOLDS: usize = 6;
    pub const STABLE_MARGIN: f64 = 0.25;

    /// The constants a withheld fold of the ledger moves by more than [`Self::STABLE_MARGIN`] — a DIAGNOSTIC,
    /// reported by `spatial_cost_identifiability_lock` rather than enforced by it.
    ///
    /// Fold spread is real information about how thin the evidence for a constant is. It is deliberately
    /// not a gate: the objective is piecewise (that `max`), so refits on different subsets legitimately
    /// land in different basins, and gating on it demotes constants that carry the ledger's plan decisions.
    pub fn unstable(&self, points: &[(PlanQuantities, u64)]) -> Vec<&'static str> {
        let base = self.constants();
        let mut bad: Vec<&'static str> = Vec::new();
        for f in 0..CycleModel::JACKKNIFE_FOLDS {
            let subset: Vec<(PlanQuantities, u64)> = points
                .iter()
                .enumerate()
                .filter(|(i, _)| i % CycleModel::JACKKNIFE_FOLDS != f)
                .map(|(_, p)| *p)
                .collect();
            if subset.len() < 4 {
                continue;
            }
            let refit = CycleModel::fit(&subset);
            for (i, (n, v)) in base.iter().enumerate() {
                let w = refit.constants()[i].1;
                if (v - w).abs() / v.abs().max(w.abs()).max(1e-12) > CycleModel::STABLE_MARGIN
                    && !bad.contains(n)
                {
                    bad.push(n);
                }
            }
        }
        bad
    }

    /// ★ A LOSS-BASED redundancy test used to live here, and it measured the wrong thing. Recorded because
    /// the mistake is an easy one to make twice.
    ///
    /// It asked whether freezing a constant at a wrong value could be absorbed by refitting the rest, and
    /// by that criterion six of these sixteen constants looked undetermined. Returning those to their
    /// priors and re-pinning changed almost nothing about predicted CYCLES — and changed TEN OF TWENTY
    /// measured plan DECISIONS, the calibrated 512³ GEMM among them. The objective's valley is flat in
    /// loss and is NOT flat in the choices the compiler makes inside it, so loss-redundancy says nothing
    /// about whether a constant matters. The operational question is the one
    /// `spatial_cost_model_lock::the_synchronization_terms_are_load_bearing` already asks: does changing
    /// it change a DECISION?
    ///
    /// What survives from that work, because it is provable rather than empirical, is
    /// [`unexercised`](Self::unexercised) — the constants no measured row can see at all.

    /// The constants NO measured row can see — the ones whose term never binds the pace on any point, so
    /// the loss is flat along them. A value the data cannot determine is not a measurement, and `fit`
    /// leaves these at their structural prior rather than reporting whatever the search last touched.
    ///
    /// This is a property of the model AND the measurements together: a constant becomes exercised the
    /// moment a row is measured whose plan it paces. `spatial_cost_identifiability_lock` requires every
    /// unexercised constant to still equal its prior, so a structural guess can never be published as a fit.
    pub fn unexercised(&self, points: &[(PlanQuantities, u64)]) -> Vec<&'static str> {
        self.constants()
            .into_iter()
            .enumerate()
            .filter(|(i, (_, v))| {
                let probe = if *v == 0.0 { 1.0 } else { v * 2.0 + 1.0 };
                let moved = self.with_constant(*i, probe);
                points
                    .iter()
                    .all(|(q, _)| (self.cycles(q) - moved.cycles(q)).abs() < 1e-9)
            })
            .map(|(_, (n, _))| n)
            .collect()
    }

    /// Deterministic fit of the constants to measured points. The objective is the sum of squared relative
    /// errors, plus a ranking hinge over every pair of points that share a problem (tile shape, `fp32_acc`,
    /// fidelity) so the model orders measured alternatives as the simulator did, plus a small ridge toward
    /// the structural priors that pins the constants no point constrains.
    ///
    /// A constant no measured row can SEE cannot move from its prior here, because nothing in the objective
    /// responds to it — see [`unexercised`](Self::unexercised), which names them, and
    /// `spatial_cost_identifiability_lock`, which requires the published model to leave them alone.
    ///
    /// Same points → same constants, bit for bit.
    pub fn fit(points: &[(PlanQuantities, u64)]) -> CycleModel {
        let prior = CycleModel::uncalibrated();
        if points.is_empty() {
            return prior;
        }
        let mut m = CycleModel::descend(prior.clone(), points, Some(&prior), &[]);
        m.calibrated = true;
        m
    }

    /// The minimizer of the DATA term alone, from `start`, optionally holding constant `hold` fixed.
    ///
    /// The regularized [`fit`](Self::fit) is what the compiler ships, but identifiability is a question
    /// about what the MEASUREMENTS determine — and the ridge exists precisely to pull unconstrained
    /// directions back to the prior, so probing under it would measure the regularizer instead of the data.
    /// `spatial_cost_identifiability_lock` uses this entry, and `hold` is how it asks whether the rest of
    /// the model can absorb a constant it has been denied.
    pub fn fit_data(
        points: &[(PlanQuantities, u64)],
        start: &CycleModel,
        hold: &[usize],
    ) -> CycleModel {
        // MULTI-START, because the objective is not convex: `steady` is a `max` over four branches, so the
        // surface is piecewise and a descent can only ever find the basin it started in. A single start
        // made the answer depend on where the search began — the very thing the pinned constants must not
        // do. The starts are fixed, so the result is still deterministic.
        let held = |m: &CycleModel| {
            let mut m = m.clone();
            for &i in hold {
                m = m.with_constant(i, start.constants()[i].1);
            }
            m
        };
        let mut starts = vec![start.clone(), held(&CycleModel::uncalibrated())];
        for f in [0.25f64, 4.0] {
            let mut s = start.clone();
            for i in 0..s.constants().len() {
                if !hold.contains(&i) {
                    s = s.with_constant(i, s.constants()[i].1 * f);
                }
            }
            starts.push(s);
        }
        starts
            .into_iter()
            .map(|s| CycleModel::descend(s, points, None, hold))
            .min_by(|a, b| a.loss(points).total_cmp(&b.loss(points)))
            .expect("at least one start")
    }

    /// One descent on the data term from `start`, holding `hold` fixed — the probe both the model and
    /// `spatial_cost_identifiability_lock` judge identifiability with.
    ///
    /// Single-start on purpose: the multi-start in [`fit_data`](Self::fit_data) exists to FIND the basin,
    /// and identifiability is a question asked inside it. Probing with a second multi-start lets the two
    /// sides of the comparison land in different basins, so the answer reports which search got lucky
    /// rather than whether the constant carries information.
    pub fn refit_from(
        points: &[(PlanQuantities, u64)],
        start: &CycleModel,
        hold: &[usize],
    ) -> CycleModel {
        CycleModel::descend(start.clone(), points, None, hold)
    }

    /// Coordinate descent with **Hooke-Jeeves pattern moves**.
    ///
    /// Plain coordinate descent stalls in a narrow diagonal valley — which is exactly the shape correlated
    /// constants make — and where it stalls then depends on where it started. That is not a cosmetic
    /// flaw: it made the fit's answer an artifact of the search path, so re-fitting the same ledger from a
    /// different point produced different constants and the pinned literals recorded a coincidence. After
    /// each sweep the net displacement is extrapolated along, which walks the valley instead of zig-zagging
    /// across it.
    fn descend(
        start: CycleModel,
        points: &[(PlanQuantities, u64)],
        prior: Option<&CycleModel>,
        hold: &[usize],
    ) -> CycleModel {
        let n = start.constants().len();
        let mut m = start;
        let mut best = m.objective(points, prior);
        for _ in 0..80 {
            let before = m.clone();
            let mut improved = false;
            for i in 0..n {
                if hold.contains(&i) {
                    continue;
                }
                let (v, l) = m.line_search(points, prior, i);
                if l + 1e-12 < best {
                    best = l;
                    m = m.with_constant(i, v);
                    improved = true;
                }
            }
            if !improved {
                break;
            }
            let now = m.constants();
            let was = before.constants();
            for &step in &[3.0, 2.0, 1.0, 0.5] {
                let mut trial = m.clone();
                for i in 0..n {
                    if hold.contains(&i) {
                        continue;
                    }
                    trial =
                        trial.with_constant(i, (now[i].1 + (now[i].1 - was[i].1) * step).max(0.0));
                }
                let l = trial.objective(points, prior);
                if l + 1e-12 < best {
                    best = l;
                    m = trial;
                }
            }
        }
        m
    }

    fn objective(&self, points: &[(PlanQuantities, u64)], prior: Option<&CycleModel>) -> f64 {
        match prior {
            Some(p) => self.fit_objective(points, p),
            None => self.loss(points),
        }
    }

    /// The best value of constant `i`, holding the others fixed: a LOG-spaced sweep over nine decades
    /// (plus exact zero) to bracket the minimum, then ternary refinement inside the bracketing cell.
    ///
    /// Log-spaced because a compensating value routinely sits one or two orders of magnitude from where the
    /// constant started: a linear span wide enough to reach it has no resolution near the origin, and one
    /// fine enough near the origin never reaches it. The earlier geometric ladder around the current value
    /// could only land on the rungs it happened to have, and stopped measurably short of the optimum.
    fn line_search(
        &self,
        points: &[(PlanQuantities, u64)],
        prior: Option<&CycleModel>,
        i: usize,
    ) -> (f64, f64) {
        let at = |v: f64| self.with_constant(i, v).objective(points, prior);
        let cur = self.constants()[i].1;
        let s = if cur.abs() < 1e-12 { 1.0 } else { cur.abs() };
        const N: usize = 180;
        let mut best = (0.0f64, at(0.0));
        for k in 0..=N {
            let v = s * 10f64.powf(-6.0 + 9.0 * k as f64 / N as f64);
            let l = at(v);
            if l < best.1 {
                best = (v, l);
            }
        }
        if best.0 == 0.0 {
            return best;
        }
        let (mut a, mut b) = (best.0 / 1.13, best.0 * 1.13);
        for _ in 0..90 {
            let (m1, m2) = (a + (b - a) / 3.0, b - (b - a) / 3.0);
            if at(m1) < at(m2) {
                b = m2;
            } else {
                a = m1;
            }
        }
        let v = 0.5 * (a + b);
        let l = at(v);
        if l < best.1 {
            (v, l)
        } else {
            best
        }
    }

    /// The fit objective: squared relative errors + pairwise ranking hinge + ridge toward `prior`.
    fn fit_objective(&self, points: &[(PlanQuantities, u64)], prior: &CycleModel) -> f64 {
        let mut obj = self.loss(points);
        for (i, (qi, mi)) in points.iter().enumerate() {
            for (qj, mj) in points.iter().skip(i + 1) {
                let same_problem = qi.problem == qj.problem
                    && qi.fp32_acc == qj.fp32_acc
                    && qi.fidelity == qj.fidelity;
                if !same_problem || mi == mj {
                    continue;
                }
                let (winner, loser, loser_measured) = if mi < mj {
                    (qi, qj, *mj)
                } else {
                    (qj, qi, *mi)
                };
                let gap = (self.cycles(winner) - self.cycles(loser)) / loser_measured as f64;
                if gap > 0.0 {
                    obj += 4.0 * gap * gap;
                }
            }
        }
        for ((_, c), (_, p)) in self.constants().into_iter().zip(prior.constants()) {
            let d = (c - p) / p.max(1.0);
            obj += 1e-4 * d * d;
        }
        obj
    }
}

/// A candidate plan for a GEMM of `mt × nt × kt` tiles: the streaming point family or a blocked point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GemmPlan {
    pub plan: BlockPlan,
    pub streaming: bool,
}

/// The streaming plan (one tile per work item, per-tile FIFOs).
pub fn streaming_plan(mt: usize, nt: usize, kt: usize, acc: AccPolicy) -> BlockPlan {
    BlockPlan {
        per_site_m: 1,
        per_site_n: 1,
        grid: (mt, nt),
        inner: InnerLoop {
            num_blocks: kt,
            block_k_tiles: 1,
        },
        block: BlockShape {
            osh: 1,
            osw: 1,
            ibw: 1,
        },
        acc,
        pack_accumulate: false,
        engines: EnginePacking::LoadsTogether,
        granularity: Granularity::PerTile,
    }
}

pub fn is_streaming(plan: &BlockPlan) -> bool {
    plan.per_site_m == 1
        && plan.per_site_n == 1
        && plan.block
            == BlockShape {
                osh: 1,
                osw: 1,
                ibw: 1,
            }
}

/// Structural quantities of a candidate plan, analytically from the plan (the same formulas
/// `PlanSummary::dram_tiles_read` reproduces from the built structure).
pub fn quantities_for_gemm(
    mt: usize,
    nt: usize,
    kt: usize,
    plan: &BlockPlan,
    fused: bool,
    fidelity: Fidelity,
    target: &SpatialTarget,
) -> PlanQuantities {
    let sites = target.sites.sites().max(1);
    let fp32_acc = plan.acc == AccPolicy::F32;
    if is_streaming(plan) {
        let items = mt * nt;
        let sites_used = sites.min(items).max(1);
        let w = items.div_ceil(sites_used) as u64;
        let kt64 = kt as u64;
        return PlanQuantities {
            problem: (mt, nt, kt),
            sites_used,
            kernel_classes: 3,
            n_sems: 0,
            iterations: w * kt64,
            tile_macs: w * kt64,
            acquires: w,
            reload_tiles: 0,
            dram_tiles_site: 2 * w * kt64 + w,
            dram_tiles_engine: (2 * w * kt64, w),
            pack_tiles: w,
            fill_macs: 1,
            drains: 0,
            pack_overlaps: true,
            dram_tiles_all: 2 * (mt * nt * kt) as u64 + (mt * nt) as u64,
            cb_handoffs: 2 * w * kt64 + w,
            mcast_blocks: 0,
            mcast_tiles: 0,
            fill_tiles_engine: (1, 1),
            fill_tiles: 2,
            drain_tiles: 1,
            fp32_acc,
            fidelity,
        };
    }
    let (pm, pn) = (plan.per_site_m, plan.per_site_n);
    let (osh, osw, ibw) = (plan.block.osh, plan.block.osw, plan.block.ibw);
    let (ny, nx) = plan.grid;
    let blocks = ny * nx;
    let sites_used = sites.min(blocks).max(1);
    let items = blocks.div_ceil(sites_used) as u64;
    let nb = (kt / ibw.max(1)).max(1) as u64;
    let sub = ((pm / osh.max(1)) * (pn / osw.max(1))) as u64;
    let (pm64, pn64, kt64) = (pm as u64, pn as u64, kt as u64);
    let tile_macs = pm64 * pn64 * kt64 * items;
    let acquires = sub * nb * items;
    // The packer's L1 accumulate keeps the partial sum in local memory, so only the LAST block reloads the
    // register file; without it every block after the first does.
    let reload_tiles = if plan.pack_accumulate {
        pm64 * pn64 * items
    } else {
        (nb - 1) * pm64 * pn64 * items
    };
    // Every hand-off the choreography makes: operand pages in, spills out, reloads back, and — under the
    // packer's in-place accumulate — the drain that returns the intermediate write pointer to the page it is
    // accumulating into, once per block but the last two.
    // The drain is a compute-engine round-trip, not a movement one: it is scored with the compute pipeline.
    let drains = if plan.pack_accumulate {
        nb.saturating_sub(2) * sub * items
    } else {
        0
    };
    let cb_handoffs = (2 * nb + sub + (nb - 1) * sub) * items;
    let pack_tiles = sub * nb * (osh * osw) as u64 * items;
    let fill_macs = (osh * osw * ibw) as u64;
    let pack_overlaps = 2 * osh * osw <= target.effective_acc_tiles(plan.acc, fp32_acc).max(1);
    let a_shared = fused && nx > 1;
    let b_shared = fused && ny > 1;
    let (a_tiles, b_tiles, c_tiles) = (
        pm64 * kt64 * items,
        pn64 * kt64 * items,
        pm64 * pn64 * items,
    );
    let dram_tiles_site = a_tiles + b_tiles + c_tiles;
    // Which engine carries which stream. Both loads together leaves the store an engine to itself and makes
    // the two operand reads serialise; pairing the second operand with the store overlaps a block's output
    // write with the next block's reads.
    let dram_tiles_engine = match plan.engines {
        EnginePacking::LoadsTogether => (a_tiles + b_tiles, c_tiles),
        EnginePacking::OperandWithStore => (a_tiles, b_tiles + c_tiles),
    };
    let a_all = if a_shared {
        (mt * kt) as u64
    } else {
        (nx * mt * kt) as u64
    };
    let b_all = if b_shared {
        (nt * kt) as u64
    } else {
        (ny * nt * kt) as u64
    };
    let dram_tiles_all = a_all + b_all + (mt * nt) as u64;
    let mcast_blocks = ((a_shared as u64) + (b_shared as u64)) * nb * items;
    let mcast_tiles = ((if a_shared { pm64 * ibw as u64 } else { 0 })
        + (if b_shared { ibw as u64 * pn64 } else { 0 }))
        * nb
        * items;
    let groups = (a_shared as usize) + (b_shared as usize);
    let kernel_classes = match groups {
        0 => 3,
        1 => 4,
        _ => 6,
    };
    PlanQuantities {
        problem: (mt, nt, kt),
        sites_used,
        kernel_classes,
        iterations: nb * items,
        n_sems: 2 * groups,
        tile_macs,
        acquires,
        reload_tiles,
        dram_tiles_site,
        dram_tiles_engine,
        pack_tiles,
        fill_macs,
        drains,
        pack_overlaps,
        dram_tiles_all,
        cb_handoffs,
        mcast_blocks,
        mcast_tiles,
        fill_tiles_engine: match plan.engines {
            EnginePacking::LoadsTogether => ((pm * ibw + ibw * pn) as u64, 0),
            EnginePacking::OperandWithStore => ((pm * ibw) as u64, (ibw * pn) as u64),
        },
        fill_tiles: (pm * ibw + ibw * pn) as u64,
        drain_tiles: (osh * osw) as u64,
        fp32_acc,
        fidelity,
    }
}

/// The plan a ledger row printed.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PlanDesc {
    pub streaming: bool,
    pub fused: bool,
    /// The site count the program ran on when it is not the whole mesh (the single-core example, a streamed
    /// program with fewer items than sites).
    pub sites: Option<usize>,
    /// The deployment mesh `(x, y)` the program was placed onto, when the row records it (a vendor example runs
    /// on the simulator's whole grid; an IR-lowered program prints `mesh=`).
    pub mesh: Option<(usize, usize)>,
    /// `(n_y, n_x)` blocks.
    pub grid: (usize, usize),
    /// `(per_site_m, per_site_n)`.
    pub per_site: (usize, usize),
    pub block: Option<BlockShape>,
    /// The row ran the SPLIT movement packing: the second operand's loader shares an engine with the store,
    /// so a block's output write overlaps the next block's reads. Recorded because it changes the per-engine
    /// movement the pace depends on — a reconstruction that drops it cannot tell two measured plans apart.
    pub split: bool,
}

/// One ttsim ledger row as a calibration point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalibrationPoint {
    pub id: String,
    pub shape: (usize, usize, usize),
    pub chip: String,
    pub stack: String,
    pub program: String,
    pub strategy: String,
    pub knobs: String,
    pub fidelity: Fidelity,
    pub fp32: bool,
    pub l1acc: bool,
    /// fp32 packing ⇔ the program kept its partials exact.
    pub exact: bool,
    pub plan: PlanDesc,
    pub cycles: u64,
    /// What the machine actually did, when the row was measured on the instrumented simulator.
    ///
    /// ★ THIS, NOT `cycles`, IS WHAT RANKS TWO MEASURED PLANS. `cycles` is ttsim's bracket counter and
    /// `ttsim_instrument_audit.json` shows it is not a function of the program — eight times the
    /// reduction depth moves it 0.08%. A row carries it for provenance; nothing may order plans by it.
    pub work: Option<MeasuredWork>,
    pub pcc: f64,
    pub pcc_floor: f64,
    pub reproduce: String,
    pub provenance: String,
}

/// The work meter recorded beside a calibration point: what the machine did, in counts it can report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeasuredWork {
    pub matmul_retired: u64,
    pub tensix_inst: u64,
    pub noc_txns: u64,
    pub noc_bytes: u64,
}

fn parse_pair(s: &str) -> Option<(usize, usize)> {
    let (a, b) = s.split_once('x')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

fn fidelity_of(s: &str) -> Option<Fidelity> {
    match s {
        "LoFi" => Some(Fidelity::LoFi),
        "HiFi2" => Some(Fidelity::HiFi2),
        "HiFi3" => Some(Fidelity::HiFi3),
        "HiFi4" => Some(Fidelity::HiFi4),
        _ => None,
    }
}

/// The blocking the tt-metal `matmul_multicore_reuse[_mcast]` programming examples choose for a shape: a
/// transcription of `bmm_op_utils::get_large_matmul_params` (the examples' shared `bmm_op.hpp` helper under
/// tt_metal/programming_examples/matmul) at the examples' fixed `in0_block_w = 2`, so those ledger rows (whose host
/// prints no config line) become fully specified calibration points. Returns (per_core_M, per_core_N,
/// out_subblock_h, out_subblock_w); `None` where the vendor returns all zeros.
pub fn vendor_example_blocking(
    mt: usize,
    nt: usize,
    num_cores_y: usize,
    num_cores_x: usize,
) -> Option<(usize, usize, usize, usize)> {
    const IN0_BLOCK_W: usize = 2;
    const SUBBLOCK_HW_CHOICES: [(usize, usize); 20] = [
        (4, 2),
        (2, 4),
        (8, 1),
        (1, 8),
        (7, 1),
        (1, 7),
        (3, 2),
        (2, 3),
        (6, 1),
        (1, 6),
        (5, 1),
        (1, 5),
        (2, 2),
        (4, 1),
        (1, 4),
        (3, 1),
        (1, 3),
        (2, 1),
        (1, 2),
        (1, 1),
    ];
    fn prime_factors(mut n: usize) -> Vec<usize> {
        let mut f = Vec::new();
        let mut d = 2;
        while d * d <= n {
            while n % d == 0 {
                f.push(d);
                n /= d;
            }
            d += 1;
        }
        if n > 1 {
            f.push(n);
        }
        f
    }
    fn possible_products(factors: &[usize]) -> Vec<usize> {
        let mut products = vec![1usize];
        for &f in factors {
            let mut next = products.clone();
            for p in &products {
                next.push(p * f);
            }
            products = next;
        }
        products.sort();
        products.dedup();
        products
    }
    fn maximum_block_dim(block_dim: usize) -> usize {
        let other = 400i64 - 2 * IN0_BLOCK_W as i64 * block_dim as i64;
        let other = other / (2 * IN0_BLOCK_W as i64 + block_dim as i64);
        other.max(0) as usize
    }
    let pick_sub = |mpc: usize, npc: usize| {
        SUBBLOCK_HW_CHOICES
            .iter()
            .find(|(h, w)| mpc % h == 0 && npc % w == 0)
            .map(|&(h, w)| (mpc, npc, h, w))
    };
    let mut nt_fac = prime_factors(nt);
    let mut mt_fac = prime_factors(mt);
    let mut npc_min = 1;
    let mut mpc_min = 1;
    nt_fac.retain(|&e| {
        if e > num_cores_x {
            npc_min *= e;
            false
        } else {
            true
        }
    });
    mt_fac.retain(|&e| {
        if e > num_cores_y {
            mpc_min *= e;
            false
        } else {
            true
        }
    });
    if npc_min > maximum_block_dim(mpc_min) {
        return None;
    }
    if mpc_min > 1 {
        let mut npc = npc_min;
        let npc_max = maximum_block_dim(mpc_min);
        for ele in possible_products(&nt_fac) {
            if ele * npc_min <= npc_max {
                npc = ele * npc_min;
            } else {
                break;
            }
        }
        if mt / mpc_min > num_cores_y || nt / npc > num_cores_x {
            return None;
        }
        return pick_sub(mpc_min, npc);
    }
    if npc_min > 1 {
        let mut mpc = mpc_min;
        let mpc_max = maximum_block_dim(npc_min);
        for ele in possible_products(&mt_fac) {
            if ele * mpc_min <= mpc_max {
                mpc = ele * mpc_min;
            } else {
                break;
            }
        }
        if mt / mpc > num_cores_y || nt / npc_min > num_cores_x {
            return None;
        }
        return pick_sub(mpc, npc_min);
    }
    let mpc_choices = possible_products(&mt_fac);
    for npc in possible_products(&nt_fac) {
        let mpc_max = maximum_block_dim(npc);
        let mut mpc = 1;
        for &ele in &mpc_choices {
            if ele <= mpc_max {
                mpc = ele;
            }
        }
        if mt / mpc > num_cores_y || nt / npc > num_cores_x {
            continue;
        }
        if let Some(found) = pick_sub(mpc, npc) {
            return Some(found);
        }
    }
    None
}

/// Parse the ledger's CURRENT rows into calibration points. A row whose printed plan the model cannot
/// quantify (a vendor example with an unknown blocking, a ttnn op) is kept with `block: None` and excluded
/// from the fit by [`quantities_for_point`].
pub fn load_calibration_points(json: &str) -> Result<Vec<CalibrationPoint>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let rows = v
        .get("rows")
        .and_then(|r| r.as_array())
        .ok_or("ledger has no rows array")?;
    let ledger_provenance = v
        .get("provenance")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let mut out = Vec::new();
    for r in rows {
        if r.get("current").and_then(|c| c.as_bool()) == Some(false) {
            continue;
        }
        let shape = r.get("shape").ok_or("row without shape")?;
        let dim = |k: &str| {
            shape
                .get(k)
                .and_then(|d| d.as_u64())
                .map(|d| d as usize)
                .ok_or(format!("shape without {k}"))
        };
        let shape = (dim("m")?, dim("n")?, dim("k")?);
        let knobs = r.get("knobs").ok_or("row without knobs")?;
        let fid = knobs
            .get("fidelity")
            .and_then(|f| f.as_str())
            .and_then(fidelity_of)
            .ok_or("row without fidelity")?;
        let fp32 = knobs
            .get("fp32_dest_acc")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let l1acc = knobs
            .get("packer_l1_acc")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let knobs_str = format!(
            "{}-fp32_{}-l1acc_{}",
            knobs.get("fidelity").and_then(|f| f.as_str()).unwrap_or(""),
            fp32 as u8,
            l1acc as u8
        );
        let stack = r
            .get("stack")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let program = r
            .get("program")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let strategy = r
            .get("strategy")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let cfg = r.get("cfg").cloned().unwrap_or(serde_json::Value::Null);
        let lower = strategy.to_lowercase();
        let streaming = lower.contains("simple")
            || lower.contains("stream")
            || program == "matmul_multi_core"
            || program == "matmul_single_core";
        let fused = lower.contains("mcast")
            || lower.contains("multicast")
            || program == "matmul_multicore_reuse_mcast";
        let split = lower.contains("split");
        let chip_str = r.get("chip").and_then(|s| s.as_str()).unwrap_or("");
        let sim = super::target::sim_grid(chip_str);
        let (grid, per_site, block) = if cfg.is_object() {
            let g = cfg
                .get("grid")
                .and_then(|s| s.as_str())
                .and_then(parse_pair)
                .map(|(x, y)| (y, x))
                .unwrap_or((0, 0));
            let pc = cfg
                .get("per_core")
                .and_then(|s| s.as_str())
                .and_then(parse_pair)
                .unwrap_or((0, 0));
            let sb = cfg
                .get("subblock")
                .and_then(|s| s.as_str())
                .and_then(parse_pair);
            let ibw = cfg
                .get("in0_block_w")
                .and_then(|n| n.as_u64())
                .map(|n| n as usize);
            let block = match (sb, ibw) {
                (Some((osh, osw)), Some(ibw)) => Some(BlockShape { osh, osw, ibw }),
                _ => None,
            };
            (g, pc, block)
        } else if streaming {
            (
                (shape.0.div_ceil(32), shape.1.div_ceil(32)),
                (1, 1),
                Some(BlockShape {
                    osh: 1,
                    osw: 1,
                    ibw: 1,
                }),
            )
        } else if program == "matmul_multicore_reuse" || program == "matmul_multicore_reuse_mcast" {
            let (mt, nt) = (shape.0.div_ceil(32), shape.1.div_ceil(32));
            let (cy, cx) = sim.map(|m| (m.y, m.x)).unwrap_or((8, 8));
            match vendor_example_blocking(mt, nt, cy, cx) {
                Some((pm, pn, osh, osw)) => (
                    (mt / pm, nt / pn),
                    (pm, pn),
                    Some(BlockShape { osh, osw, ibw: 2 }),
                ),
                None => ((0, 0), (0, 0), None),
            }
        } else {
            ((0, 0), (0, 0), None)
        };
        let cfg_mesh = cfg
            .get("mesh")
            .and_then(|s| s.as_str())
            .and_then(parse_pair);
        let cfg_sites = cfg
            .get("sites")
            .and_then(|n| n.as_u64())
            .map(|n| n as usize);
        let sites = if program == "matmul_single_core" {
            Some(1)
        } else {
            cfg_sites
        };
        let mesh = cfg_mesh.or_else(|| {
            if program == "logos_spatial" {
                None
            } else {
                sim.map(|m| (m.x, m.y))
            }
        });
        let plan = PlanDesc {
            streaming,
            fused,
            sites,
            mesh,
            grid,
            per_site,
            block,
            split,
        };
        out.push(CalibrationPoint {
            id: r
                .get("id")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
            shape,
            chip: r
                .get("chip")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
            stack,
            program,
            strategy,
            knobs: knobs_str,
            fidelity: fid,
            fp32,
            l1acc,
            exact: fp32,
            plan,
            cycles: r.get("cycles").and_then(|c| c.as_u64()).unwrap_or(0),
            work: r.get("work").and_then(|w| {
                let g = |k: &str| w.get(k).and_then(|v| v.as_u64());
                Some(MeasuredWork {
                    matmul_retired: g("matmul_retired")?,
                    tensix_inst: g("tensix_inst")?,
                    noc_txns: g("noc_txns")?,
                    noc_bytes: g("noc_bytes")?,
                })
            }),
            pcc: r.get("pcc").and_then(|c| c.as_f64()).unwrap_or(0.0),
            pcc_floor: r.get("pcc_floor").and_then(|c| c.as_f64()).unwrap_or(1.0),
            reproduce: r
                .get("reproduce")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
            provenance: r
                .get("provenance")
                .and_then(|s| s.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| ledger_provenance.clone()),
        });
    }
    Ok(out)
}

/// The quantities of a recorded point on `target`, when its plan is fully specified AND its knobs are ones
/// the compiler emits on this target (the target's declared fidelity; fp32 accumulate on or off). A row at a
/// fidelity the pipeline never chooses is a comparison, not a calibration point.
pub fn quantities_for_point(
    p: &CalibrationPoint,
    target: &SpatialTarget,
) -> Option<PlanQuantities> {
    if p.stack == "ttnn_production" || target.matmul_fidelity() != Some(p.fidelity) {
        return None;
    }
    // The row's deployment grid is the mesh it records; a smaller recorded SITE COUNT means the program used
    // only part of that mesh (the single-core example), and that is what sets its parallelism.
    let mesh = p
        .plan
        .mesh
        .map(|(x, y)| MeshShape { x, y })
        .unwrap_or(target.sites);
    let restricted;
    let target = match p.plan.sites {
        Some(n) if n < mesh.sites() => {
            restricted = SpatialTarget {
                sites: MeshShape {
                    x: n.min(mesh.x),
                    y: n.div_ceil(n.min(mesh.x)),
                },
                ..target.clone()
            };
            &restricted
        }
        _ if mesh != target.sites => {
            restricted = SpatialTarget {
                sites: mesh,
                ..target.clone()
            };
            &restricted
        }
        _ => target,
    };
    let (mt, nt, kt) = (
        p.shape.0.div_ceil(target.tile.rows),
        p.shape.1.div_ceil(target.tile.cols),
        p.shape.2.div_ceil(target.tile.cols),
    );
    let acc = AccPolicy::F32;
    let plan = if p.plan.streaming {
        streaming_plan(mt, nt, kt, acc)
    } else {
        let block = p.plan.block?;
        let (pm, pn) = p.plan.per_site;
        if pm == 0
            || pn == 0
            || block.ibw == 0
            || mt % pm != 0
            || nt % pn != 0
            || kt % block.ibw != 0
        {
            return None;
        }
        // The knobs the row RAN, not defaults: packing and in-place accumulate are exactly what separates
        // two rows of the same shape and knobs, so reconstructing them as defaults asks the model to rank
        // plans it has been made blind to.
        let engines = if p.plan.split {
            EnginePacking::OperandWithStore
        } else {
            EnginePacking::LoadsTogether
        };
        BlockPlan {
            per_site_m: pm,
            per_site_n: pn,
            grid: (mt / pm, nt / pn),
            inner: InnerLoop {
                num_blocks: kt / block.ibw,
                block_k_tiles: block.ibw,
            },
            block,
            acc,
            pack_accumulate: p.l1acc,
            engines,
            granularity: Granularity::PerTile,
        }
    };
    let mut q = quantities_for_gemm(mt, nt, kt, &plan, p.plan.fused, p.fidelity, target);
    q.fp32_acc = p.fp32;
    Some(q)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/spatial/cost/vendor_blocking_tests.rs"]
mod vendor_blocking_tests;

/// How many DRAM tile-reads an arrangement costs, given the output tile grid and the reduction depth.
///
/// ★ THE ARRANGEMENT IS AN INPUT, NOT A NAMED PLAN. The retired version took an enum of three
/// human-chosen plans, and `spatial_cost_model_lock::no_strategy_enum_input_exists` forbids that
/// vocabulary anywhere under `spatial/` — including in a comment like this one, which is why this
/// paragraph describes it rather than naming it. (It caught the first draft of this very comment.)
///
/// What actually determines the traffic is how operands are SHARED: no sharing re-streams both
/// operands per output tile; per-core-block reuse reads each once per core column and row; a multicast
/// fetches each tile exactly once. Those are properties of the placement the passes CHOSE, so they
/// arrive here as parameters and the cost model reads them off the plan.
///
/// Salvaged from the retired emitter; `codegen_tt`
/// keeps the public NAME as a thin wrapper so callers are unmoved.
pub fn dram_tiles(mt: u64, nt: u64, kt: u64, share: OperandSharing) -> u64 {
    match share {
        // Every one of mt·nt output tiles re-streams kt A-tiles and kt B-tiles.
        OperandSharing::None => 2 * mt * nt * kt,
        // A is read once per core-column, B once per core-row.
        OperandSharing::PerCoreBlock { grid_x, grid_y } => grid_x * mt * kt + grid_y * nt * kt,
        // Each tile is fetched once and multicast to everyone who needs it.
        OperandSharing::Multicast => (mt + nt) * kt,
    }
}

/// How an arrangement shares its operands across the mesh — the thing that decides DRAM traffic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperandSharing {
    None,
    PerCoreBlock { grid_x: u64, grid_y: u64 },
    Multicast,
}
