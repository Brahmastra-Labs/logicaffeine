//! The constructive 2.5D/3D stack ADL.
//!
//! A stack is COMPOSED, not monolithic: tiers (each an analog fabric, a digital power-map
//! descriptor, or a bare interposer) separated by gaps whose vertical interconnect is a
//! SCARCE, counted resource — TSVs conduct heat as well as signal (`g_w_per_k` is a design
//! lever the thermal phases measure, not an afterthought). Every physical figure carries a
//! [`FigureStatus`]: sourced (cited in `benchmarks/silicon/analog/thermal-constants.md`), derived
//! (arithmetic shown), or loudly modeled — never invented.
//!
//! Phase S1 is the SPINE + the degenerate embed: [`StackTarget::single`] wraps today's 2D
//! analog lane with zero behavioral change ([`analog_artifact_on_stack`] delegates and the
//! lock proves bit-identity), and [`StackTarget::sky130_stack_v1`] states the V1 stack —
//! the digital tier DELIBERATELY in the analog tier's heat path, so the thermal phases
//! (S3–S5) have something honest to refuse. Placement (S2), the certified temperature map
//! (S3), and cooling refusals (S5) build on this vocabulary without reshaping it.

use super::target::AnalogTarget;
use super::{analog_artifact_from_source, AnaLowerError, AnalogArtifact};

/// Provenance of a physical figure — the dossier discipline as a type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FigureStatus {
    /// A cited figure; the string names the source (full entry in the dossier).
    Sourced(&'static str),
    /// Computed from sourced inputs; the string shows the arithmetic.
    Derived(&'static str),
    /// OUR design parameter awaiting characterization; the string says so.
    Modeled(&'static str),
}

/// A tier's bulk material for lateral heat spreading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    pub name: &'static str,
    /// Bulk thermal conductivity, W/(m·K).
    pub k_w_per_m_k: f64,
    pub status: FigureStatus,
}

impl Material {
    /// Bulk silicon at 300 K (dossier: Incropera property tables).
    pub fn silicon() -> Material {
        Material {
            name: "silicon",
            k_w_per_m_k: 148.0,
            status: FigureStatus::Sourced(
                "Incropera & DeWitt property tables: silicon ~148 W/(m.K) at 300 K \
                 (benchmarks/silicon/analog/thermal-constants.md)",
            ),
        }
    }

    /// The organic bond/underfill background class (loudly modeled).
    pub fn underfill_class() -> Material {
        Material {
            name: "underfill-class",
            k_w_per_m_k: 0.5,
            status: FigureStatus::Modeled(
                "V1 bond-layer background conductance class; vendor filled underfills span \
                 0.3-3 W/(m.K) — OUR declared figure, not a citation",
            ),
        }
    }
}

/// What computes on a tier.
#[derive(Clone, Debug, PartialEq)]
pub enum TierFabric {
    /// An analog execution plane — the full characterized ADL rides along.
    Analog(AnalogTarget),
    /// A digital tier as a POWER DESCRIPTOR (heat source + name), NOT a codegen_sva import —
    /// the stack cares what it dissipates, the digital lane keeps its own regime.
    Digital(DigitalTierPower),
    /// Passive routing/spreading only.
    Interposer,
}

/// The digital tier's sourced power injection.
#[derive(Clone, Debug, PartialEq)]
pub struct DigitalTierPower {
    pub design: &'static str,
    /// Total dissipation, watts.
    pub power_w: f64,
    /// Carried VERBATIM from the measurement artifact (`benchmarks/silicon/sky130_power.json`).
    pub provenance: &'static str,
}

/// One tier of the stack, bottom-up.
#[derive(Clone, Debug, PartialEq)]
pub struct Tier {
    pub name: &'static str,
    pub fabric: TierFabric,
    pub material: Material,
    pub thickness_um: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerticalKind {
    MicroBump,
    HybridBond,
    Tsv,
}

/// A counted vertical interconnect class in one gap — SCARCE like `mul_tiles`. TSVs conduct
/// heat: `g_w_per_k` is per-via thermal conductance, a placement-dependent cooling lever.
#[derive(Clone, Debug, PartialEq)]
pub struct VerticalClass {
    pub kind: VerticalKind,
    pub pitch_um: f64,
    pub r_ohm: f64,
    pub c_ff: f64,
    pub g_w_per_k: f64,
    /// The scarce budget: how many of this class this gap provides.
    pub count: usize,
    pub status: FigureStatus,
}

impl VerticalClass {
    /// The V1 copper TSV class (thermal conductance DERIVED from sourced k_Cu; see dossier).
    pub fn tsv_v1(count: usize) -> VerticalClass {
        VerticalClass {
            kind: VerticalKind::Tsv,
            pitch_um: 40.0,
            r_ohm: 0.05,
            c_ff: 40.0,
            g_w_per_k: 3.15e-4,
            count,
            status: FigureStatus::Derived(
                "g = k_Cu*A/L = 401 * pi*(5e-6)^2 / 100e-6 ~ 3.15e-4 W/K per via (k_Cu SOURCED, \
                 CRC 401 W/(m.K)); R/C/pitch MODELED (benchmarks/silicon/analog/thermal-constants.md)",
            ),
        }
    }

    /// The V1 microbump class (modeled class figures).
    pub fn microbump_v1(count: usize) -> VerticalClass {
        VerticalClass {
            kind: VerticalKind::MicroBump,
            pitch_um: 50.0,
            r_ohm: 0.02,
            c_ff: 25.0,
            g_w_per_k: 1.0e-4,
            count,
            status: FigureStatus::Modeled(
                "V1 class figures (benchmarks/silicon/analog/thermal-constants.md)",
            ),
        }
    }
}

/// How the stack sheds heat at its faces. S1 states the boundary; S5 makes the refusals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoolingClass {
    NaturalConvection,
    ForcedAir,
    ColdPlate,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoolingBoundary {
    pub ambient_c: f64,
    pub top: CoolingClass,
    pub bottom: CoolingClass,
    pub t_junction_max_c: f64,
}

impl CoolingBoundary {
    /// The V1 default: forced air over the top face, board-side natural convection, the
    /// commodity 85 °C junction ceiling (modeled envelope, tightened by S5's refusal logic).
    pub fn forced_air_default() -> CoolingBoundary {
        CoolingBoundary {
            ambient_c: 25.0,
            top: CoolingClass::ForcedAir,
            bottom: CoolingClass::NaturalConvection,
            t_junction_max_c: 85.0,
        }
    }
}

/// A shape violation — a stack that cannot mean anything is refused with a name.
#[derive(Clone, Debug, PartialEq)]
pub enum StackShapeError {
    /// `gaps.len()` must be exactly `tiers.len() − 1`.
    GapCountMismatch {
        tiers: usize,
        gaps: usize,
    },
    NoTiers,
    /// A vertical class with `count == 0` is a phantom resource.
    EmptyVerticalClass {
        gap: usize,
    },
}

/// The composed stack — tiers bottom-up, gap `i` between tier `i` and tier `i+1`.
#[derive(Clone, Debug, PartialEq)]
pub struct StackTarget {
    pub name: &'static str,
    pub tiers: Vec<Tier>,
    pub gaps: Vec<Vec<VerticalClass>>,
    pub cooling: CoolingBoundary,
}

impl StackTarget {
    /// The 1-tier degenerate embed: today's 2D analog lane, unchanged — the lock proves the
    /// compiled artifact is bit-identical to the direct path.
    pub fn single(t: AnalogTarget) -> StackTarget {
        StackTarget {
            name: "single-tier",
            tiers: vec![Tier {
                name: "analog",
                fabric: TierFabric::Analog(t),
                material: Material::silicon(),
                thickness_um: 300.0,
            }],
            gaps: vec![],
            cooling: CoolingBoundary::forced_air_default(),
        }
    }

    /// The V1 stack: the sourced-power digital tier BELOW the analog tier — deliberately in
    /// its heat path, so the thermal phases have something honest to measure and refuse.
    pub fn sky130_stack_v1() -> StackTarget {
        StackTarget {
            name: "sky130-stack-v1",
            tiers: vec![
                Tier {
                    name: "digital",
                    fabric: TierFabric::Digital(DigitalTierPower {
                        design: "soft_core_scalar",
                        power_w: 0.0295,
                        provenance: "OpenLane Sky130 signoff, OpenSTA report_power on the post-route RC-extracted netlist the \
                                     ledger ships \
                                     (tools/OpenLane/designs/logos_soft_core_scalar/runs/RUN_2026.10.15_17.14.37, \
                                     31492840-byte GDS) at its 15 ns signoff clock, typical corner. Switching activity is \
                                     OpenSTA's propagated default (no workload VCD) — a signoff EDA power number, not a \
                                     bench-measured one.",
                    }),
                    material: Material::silicon(),
                    thickness_um: 300.0,
                },
                Tier {
                    name: "analog",
                    fabric: TierFabric::Analog(AnalogTarget::sky130_fpaa()),
                    material: Material::silicon(),
                    thickness_um: 100.0,
                },
            ],
            gaps: vec![vec![VerticalClass::tsv_v1(64), VerticalClass::microbump_v1(256)]],
            cooling: CoolingBoundary::forced_air_default(),
        }
    }

    /// Structural coherence — refused with a name, never a meaningless stack.
    pub fn validate(&self) -> Result<(), StackShapeError> {
        if self.tiers.is_empty() {
            return Err(StackShapeError::NoTiers);
        }
        if self.gaps.len() + 1 != self.tiers.len() {
            return Err(StackShapeError::GapCountMismatch {
                tiers: self.tiers.len(),
                gaps: self.gaps.len(),
            });
        }
        for (i, gap) in self.gaps.iter().enumerate() {
            if gap.iter().any(|v| v.count == 0) {
                return Err(StackShapeError::EmptyVerticalClass { gap: i });
            }
        }
        Ok(())
    }

    /// The stack's FIRST analog tier, if any (the region-compilation anchor; placement spans
    /// all analog tiers via `stack_place`).
    pub fn analog_tier(&self) -> Option<(usize, &AnalogTarget)> {
        self.tiers
            .iter()
            .enumerate()
            .find_map(|(i, t)| match &t.fabric {
                TierFabric::Analog(a) => Some((i, a)),
                _ => None,
            })
    }
}

/// Compile an analog region ONTO a stack: resolve the analog tier and delegate to the 2D
/// lane — the degenerate embed is bit-identical by construction (locked, not assumed).
pub fn analog_artifact_on_stack(
    src: &str,
    fn_name: &str,
    stack: &StackTarget,
) -> Result<AnalogArtifact, AnaLowerError> {
    if let Err(e) = stack.validate() {
        return Err(AnaLowerError::UnloweredOp(format!(
            "malformed stack: {e:?}"
        )));
    }
    let Some((_, target)) = stack.analog_tier() else {
        return Err(AnaLowerError::Unsupported("stack has no analog tier"));
    };
    analog_artifact_from_source(src, fn_name, target)
}
