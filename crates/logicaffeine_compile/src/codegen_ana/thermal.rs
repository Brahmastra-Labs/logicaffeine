//! The certified thermal map — `G·ΔT = P` over exact ℚ, judged independently.
//!
//! The stack becomes a steady-state conductance network at tile granularity: analog tiers
//! coarsen to a small thermal grid (a MODELED fidelity statement, not physics — the knob is
//! in [`ThermalOpts`]), digital and interposer tiers are single slabs, lateral conductance is
//! the sourced bulk `k` per square (`k·t`), the gap between tiers carries the bond-layer
//! background PLUS the provisioned vertical interconnect — with the crossings the placement
//! ACTUALLY consumed attached at their mapped cells, so cooling is placement-dependent (the
//! TSV-as-heat-lever, measured by the lock, not asserted). Cooling classes ground the top
//! and bottom faces through the dossier's carried film coefficients; the digital tier
//! injects its SOURCED signoff power; occupied analog tiles inject the MODELED bias power.
//!
//! The solve is exact Gaussian elimination over ℚ, and the emitted
//! [`ThermalClaim`] is judged by `logicaffeine_proof::thermal_replay::check_thermal_claim`,
//! which reassembles the Laplacian itself — the same one-residual-checker trust pattern as
//! the relaxation lane. A network whose heat has nowhere to go refuses
//! [`AnaLowerError::ThermalModelDisconnected`], never a fantasy equilibrium.

use logicaffeine_base::numeric::Rational;
use logicaffeine_proof::thermal_replay::ThermalClaim;

use super::cell::AnalogNetlist;
use super::stack::{CoolingClass, StackTarget, TierFabric};
use super::stack_place::StackPlacement;
use super::AnaLowerError;

/// The modeled-fidelity knobs. Every figure is a design parameter of OUR model, labeled in
/// [`THERMAL_MODEL_PROVENANCE`] and anchored in `benchmarks/silicon/analog/thermal-constants.md`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThermalOpts {
    /// Analog tiers coarsen to this thermal grid (rows, cols).
    pub analog_grid: (usize, usize),
    /// Thermal cell edge, µm.
    pub cell_edge_um: f64,
    /// Bond-layer thickness, µm.
    pub bond_thickness_um: f64,
    /// Bias power per occupied analog tile, watts.
    pub tile_bias_w: f64,
}

/// The V1 defaults.
pub fn thermal_default_opts() -> ThermalOpts {
    ThermalOpts {
        analog_grid: (4, 4),
        cell_edge_um: 500.0,
        bond_thickness_um: 10.0,
        tile_bias_w: 1.0e-4,
    }
}

/// Provenance of the model's figures — pinned by `thermal_provenance_lock`.
pub const THERMAL_MODEL_PROVENANCE: &str =
    "Sourced: bulk k per Material (Incropera/CRC, benchmarks/silicon/analog/thermal-constants.md); digital \
     tier power from benchmarks/silicon/sky130_power.json (OpenLane signoff, carried verbatim); \
     TSV per-via g DERIVED from sourced k_Cu. Modeled (OUR design parameters, never silicon): \
     film-coefficient carried points h = 10 / 100 / 2000 W/(m2.K) for natural / forced-air / \
     cold-plate (Incropera Table 1.1 RANGES are the sourced part), analog-tier coarsening grid, \
     500 um thermal cell edge, 10 um bond thickness, underfill-class bond k, 1e-4 W per-tile \
     bias power, cross-tier via attachment at mapped coarse cells. Full dossier: \
     benchmarks/silicon/analog/thermal-constants.md";

/// The carried film-coefficient point for a cooling class, W/(m²·K).
pub fn film_coefficient_w_m2k(class: CoolingClass) -> f64 {
    match class {
        CoolingClass::NaturalConvection => 10.0,
        CoolingClass::ForcedAir => 100.0,
        CoolingClass::ColdPlate => 2000.0,
    }
}

/// A certified-solvable thermal map: the exact claim plus f64 conveniences for consumers.
#[derive(Clone, Debug, PartialEq)]
pub struct ThermalMap {
    pub claim: ThermalClaim,
    /// ΔT above ambient per node, °C (== K), f64 view of the exact claim.
    pub delta_t_c: Vec<f64>,
    /// Which stack tier each node belongs to.
    pub node_tier: Vec<usize>,
    pub hottest: usize,
}

/// The model states its figures on an exact picoscale grid (10⁻¹² W/K, W): these are MODELED
/// design parameters, and the snap IS part of the model's statement — the checker certifies
/// the STATED network exactly, and small shared denominators keep the ℚ elimination fast.
fn rat(x: f64) -> Result<Rational, AnaLowerError> {
    if !x.is_finite() {
        return Err(AnaLowerError::ThermalModelDisconnected);
    }
    let scaled = (x * 1.0e12).round() as i64;
    Ok(Rational::from_i64(scaled)
        .div(&Rational::from_i64(1_000_000_000_000))
        .expect("nonzero denominator"))
}

/// Build and exactly solve the stack's steady-state thermal network for one placed region.
pub fn thermal_map(
    stack: &StackTarget,
    netlist: &AnalogNetlist,
    placement: &StackPlacement,
    opts: &ThermalOpts,
) -> Result<ThermalMap, AnaLowerError> {
    if let Err(e) = stack.validate() {
        return Err(AnaLowerError::UnloweredOp(format!(
            "malformed stack: {e:?}"
        )));
    }
    let (gr, gc) = opts.analog_grid;
    assert!(
        gr > 0 && gc > 0,
        "a zero-size thermal grid is not a coarsening"
    );
    let cell_edge_m = opts.cell_edge_um * 1.0e-6;
    let cell_area_m2 = cell_edge_m * cell_edge_m;

    // ── Node layout: analog tiers get gr×gc grids, other tiers one slab node. ──
    let mut base = Vec::with_capacity(stack.tiers.len());
    let mut node_tier = Vec::new();
    for (ti, tier) in stack.tiers.iter().enumerate() {
        base.push(node_tier.len());
        let count = match tier.fabric {
            TierFabric::Analog(_) => gr * gc,
            _ => 1,
        };
        node_tier.extend(std::iter::repeat(ti).take(count));
    }
    let n = node_tier.len();
    let grid_node = |ti: usize, r: usize, c: usize| base[ti] + r * gc + c;
    let coarse_of = |ti: usize, row: usize, col: usize| -> (usize, usize) {
        let TierFabric::Analog(a) = &stack.tiers[ti].fabric else {
            return (0, 0);
        };
        (
            (row * gr / a.rows.max(1)).min(gr - 1),
            (col * gc / a.cols.max(1)).min(gc - 1),
        )
    };
    let tier_rep_node = |ti: usize| -> usize {
        match stack.tiers[ti].fabric {
            TierFabric::Analog(_) => grid_node(ti, gr / 2, gc / 2),
            _ => base[ti],
        }
    };

    let mut edges: Vec<(usize, usize, Rational)> = Vec::new();
    let mut grounds: Vec<(usize, Rational)> = Vec::new();
    let mut injections: Vec<(usize, Rational)> = Vec::new();

    // ── Lateral spreading: k·t per square between adjacent grid cells. ──
    for (ti, tier) in stack.tiers.iter().enumerate() {
        if let TierFabric::Analog(_) = tier.fabric {
            let g_lat = rat(tier.material.k_w_per_m_k * tier.thickness_um * 1.0e-6)?;
            for r in 0..gr {
                for c in 0..gc {
                    if r + 1 < gr {
                        edges.push((grid_node(ti, r, c), grid_node(ti, r + 1, c), g_lat.clone()));
                    }
                    if c + 1 < gc {
                        edges.push((grid_node(ti, r, c), grid_node(ti, r, c + 1), g_lat.clone()));
                    }
                }
            }
        }
    }

    // ── Vertical: bond background per overlapping cell pair + provisioned interconnect. ──
    for (gi, gap) in stack.gaps.iter().enumerate() {
        let (lo, hi) = (gi, gi + 1);
        let k_bond = super::stack::Material::underfill_class().k_w_per_m_k;
        let g_bond_cell = rat(k_bond * cell_area_m2 / (opts.bond_thickness_um * 1.0e-6))?;
        let pair_nodes: Vec<(usize, usize)> = match (
            matches!(stack.tiers[lo].fabric, TierFabric::Analog(_)),
            matches!(stack.tiers[hi].fabric, TierFabric::Analog(_)),
        ) {
            (true, true) => (0..gr)
                .flat_map(|r| (0..gc).map(move |c| (r, c)))
                .map(|(r, c)| (grid_node(lo, r, c), grid_node(hi, r, c)))
                .collect(),
            (true, false) => (0..gr)
                .flat_map(|r| (0..gc).map(move |c| (r, c)))
                .map(|(r, c)| (grid_node(lo, r, c), base[hi]))
                .collect(),
            (false, true) => (0..gr)
                .flat_map(|r| (0..gc).map(move |c| (r, c)))
                .map(|(r, c)| (base[lo], grid_node(hi, r, c)))
                .collect(),
            (false, false) => vec![(base[lo], base[hi])],
        };
        for (a, b) in &pair_nodes {
            edges.push((*a, *b, g_bond_cell.clone()));
        }
        // Provisioned vertical interconnect: the crossings the placement consumed attach at
        // their mapped cells; the unconsumed remainder spreads uniformly. Copper conducts
        // whether or not a signal uses it — total conductance is conserved.
        let per_via_g: f64 = gap.first().map(|v| v.g_w_per_k).unwrap_or(0.0);
        let provisioned: usize = gap.iter().map(|v| v.count).sum();
        let consumed = placement
            .vertical_used
            .get(gi)
            .copied()
            .unwrap_or(0)
            .min(provisioned);
        if per_via_g > 0.0 && provisioned > 0 {
            let mut attached = 0usize;
            for (i, cell_site) in placement.site.iter().enumerate() {
                for o in super::stack_place::operand_cells_of(&netlist.cells[i]) {
                    let so = placement.site[o];
                    let (t1, t2) = (cell_site.tier.min(so.tier), cell_site.tier.max(so.tier));
                    if t1 <= lo && hi <= t2 && t1 != t2 && attached < consumed {
                        let na = match stack.tiers[lo].fabric {
                            TierFabric::Analog(_) => {
                                let (r, c) = coarse_of(lo, cell_site.row, cell_site.col);
                                grid_node(lo, r, c)
                            }
                            _ => base[lo],
                        };
                        let nb = match stack.tiers[hi].fabric {
                            TierFabric::Analog(_) => {
                                let (r, c) = coarse_of(hi, so.row, so.col);
                                grid_node(hi, r, c)
                            }
                            _ => base[hi],
                        };
                        edges.push((na, nb, rat(per_via_g)?));
                        attached += 1;
                    }
                }
            }
            let dark = provisioned - attached;
            if dark > 0 {
                let g_dark_total = rat(per_via_g * dark as f64)?;
                let share = g_dark_total
                    .div(&Rational::from_i64(pair_nodes.len() as i64))
                    .expect("pair_nodes is non-empty");
                for (a, b) in &pair_nodes {
                    edges.push((*a, *b, share.clone()));
                }
            }
        }
    }

    // ── Cooling boundaries: top face of the top tier, bottom face of the bottom tier. ──
    let h_top = film_coefficient_w_m2k(stack.cooling.top);
    let h_bot = film_coefficient_w_m2k(stack.cooling.bottom);
    let top = stack.tiers.len() - 1;
    match stack.tiers[top].fabric {
        TierFabric::Analog(_) => {
            let g = rat(h_top * cell_area_m2)?;
            for r in 0..gr {
                for c in 0..gc {
                    grounds.push((grid_node(top, r, c), g.clone()));
                }
            }
        }
        _ => grounds.push((base[top], rat(h_top * cell_area_m2 * (gr * gc) as f64)?)),
    }
    match stack.tiers[0].fabric {
        TierFabric::Analog(_) => {
            let g = rat(h_bot * cell_area_m2)?;
            for r in 0..gr {
                for c in 0..gc {
                    grounds.push((grid_node(0, r, c), g.clone()));
                }
            }
        }
        _ => grounds.push((base[0], rat(h_bot * cell_area_m2 * (gr * gc) as f64)?)),
    }

    // ── Injections: sourced digital power; modeled bias per occupied analog tile. ──
    for (ti, tier) in stack.tiers.iter().enumerate() {
        if let TierFabric::Digital(d) = &tier.fabric {
            injections.push((tier_rep_node(ti), rat(d.power_w)?));
        }
    }
    let bias = rat(opts.tile_bias_w)?;
    for s in &placement.site {
        if matches!(stack.tiers[s.tier].fabric, TierFabric::Analog(_)) {
            let (r, c) = coarse_of(s.tier, s.row, s.col);
            injections.push((grid_node(s.tier, r, c), bias.clone()));
        }
    }

    // ── Assemble the Laplacian and solve exactly. ──
    let mut g_mat = vec![vec![Rational::zero(); n]; n];
    for (a, b, g) in &edges {
        g_mat[*a][*a] = g_mat[*a][*a].add(g);
        g_mat[*b][*b] = g_mat[*b][*b].add(g);
        g_mat[*a][*b] = g_mat[*a][*b].sub(g);
        g_mat[*b][*a] = g_mat[*b][*a].sub(g);
    }
    for (node, g) in &grounds {
        g_mat[*node][*node] = g_mat[*node][*node].add(g);
    }
    let mut p = vec![Rational::zero(); n];
    for (node, w) in &injections {
        p[*node] = p[*node].add(w);
    }
    let delta_t = exact_solve(&mut g_mat, p).ok_or(AnaLowerError::ThermalModelDisconnected)?;

    let delta_t_c: Vec<f64> = delta_t.iter().map(|t| t.to_f64()).collect();
    let hottest = (0..n)
        .max_by(|a, b| delta_t[*a].cmp(&delta_t[*b]))
        .expect("n > 0 — a stack has tiers");
    Ok(ThermalMap {
        claim: ThermalClaim {
            n,
            edges,
            grounds,
            injections,
            delta_t,
        },
        delta_t_c,
        node_tier,
        hottest,
    })
}

/// The name of a cooling class, for refusal messages.
pub fn cooling_class_name(class: CoolingClass) -> &'static str {
    match class {
        CoolingClass::NaturalConvection => "natural-convection",
        CoolingClass::ForcedAir => "forced-air",
        CoolingClass::ColdPlate => "cold-plate",
    }
}

/// Judge the CERTIFIED map against the stack's DECLARED cooling budget: every junction must
/// sit under `t_junction_max_c` at the stated ambient. The refusal names the hottest node,
/// its tier, both temperatures, and the cooling class — the same program may compile under a
/// stronger class; that verdict belongs to the checker's map, not the producer's optimism.
pub fn check_cooling(map: &ThermalMap, stack: &StackTarget) -> Result<(), AnaLowerError> {
    let t_junction_c = stack.cooling.ambient_c + map.delta_t_c[map.hottest];
    if t_junction_c > stack.cooling.t_junction_max_c {
        return Err(AnaLowerError::ThermalBudgetExceeded {
            node: map.hottest,
            tier: map.node_tier[map.hottest],
            t_junction_c,
            t_max_c: stack.cooling.t_junction_max_c,
            cooling: cooling_class_name(stack.cooling.top),
        });
    }
    Ok(())
}

/// Exact solve by FRACTION-FREE Bareiss elimination. The model states every figure on the
/// 10⁻¹² grid, so clearing that denominator makes the working matrix integer-valued and
/// Bareiss keeps it that way (entries bounded by minor determinants — no fraction blow-up,
/// which is the difference between milliseconds and minutes at stack scale). `None` iff
/// singular — for a Laplacian-plus-grounds matrix that means a floating island.
fn exact_solve(a: &mut [Vec<Rational>], b: Vec<Rational>) -> Option<Vec<Rational>> {
    let n = b.len();
    let grid = Rational::from_i64(1_000_000_000_000);
    let mut m: Vec<Vec<Rational>> = a
        .iter()
        .map(|row| row.iter().map(|x| x.mul(&grid)).collect())
        .collect();
    let mut rhs: Vec<Rational> = b.iter().map(|x| x.mul(&grid)).collect();

    let mut prev = Rational::one();
    for k in 0..n {
        let p = (k..n).find(|r| !m[*r][k].is_zero())?;
        m.swap(p, k);
        rhs.swap(p, k);
        let pivot = m[k][k].clone();
        for i in (k + 1)..n {
            for j in (k + 1)..n {
                let num = m[i][j].mul(&pivot).sub(&m[i][k].mul(&m[k][j]));
                m[i][j] = num.div(&prev).expect("Bareiss division is exact");
            }
            let num = rhs[i].mul(&pivot).sub(&m[i][k].mul(&rhs[k]));
            rhs[i] = num.div(&prev).expect("Bareiss division is exact");
            m[i][k] = Rational::zero();
        }
        prev = pivot;
    }
    let mut x = vec![Rational::zero(); n];
    for row in (0..n).rev() {
        let mut acc = rhs[row].clone();
        for c in (row + 1)..n {
            acc = acc.sub(&m[row][c].mul(&x[c]));
        }
        x[row] = acc.div(&m[row][row])?;
    }
    Some(x)
}
