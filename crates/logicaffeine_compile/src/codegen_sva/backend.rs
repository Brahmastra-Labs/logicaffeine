//! The target-backend seam.
//!
//! Everything up to a placed [`Grid`] is target-agnostic (English → `LoopCell` IR → certified placement). Only
//! the final `Grid → bytes` step is backend-specific. This module names that seam as a [`TargetBackend`] trait
//! so a new hardware backend (e.g. the SambaNova RDU) is a tested extension point instead of a loose
//! convention. Emission is target-specific; the proof-carrying [`certify`](TargetBackend::certify) is
//! backend-agnostic (default impl), so even a backend that can't emit its vendor config yet still proves a
//! placement ≡ its source.

use super::fabric_grid::{emit_config_grid_verilog, emit_grid_verilog, Grid};
use super::fabric_rtl::FabricOp;
use super::fabric_target::FabricTarget;
use super::pnr_certificate::{certify_external_placement, recheck_external_placement, PlacementCertificate, PlacementFault};
use crate::spatial::lower::rdu::{neutral_envelope, GridBody, NeutralBody, GATED_REASON};
use crate::spatial::rdu_chip::{RduChip, RduChipFacts};

/// What a backend produces from a placed grid.
#[derive(Clone, Debug)]
pub enum BackendArtifact {
    /// Synthesizable RTL — our own Sky130/FPGA silicon path.
    Verilog(String),
    /// Vendor config emission is gated on the SDK. Carries the ready-to-bridge NEUTRAL placement (a
    /// deterministic dump of the placed grid), so the ONLY remaining work is the vendor's config format. This
    /// is never a faked config — it is an honest "the placement is ready; the config format is pending the
    /// SDK." The proof that this placement ≡ the source is available separately via
    /// [`TargetBackend::certify`], which needs no SDK at all.
    SdkGated {
        neutral_placement: String,
        reason: &'static str,
    },
}

impl BackendArtifact {
    pub fn as_verilog(&self) -> Option<&str> {
        match self {
            BackendArtifact::Verilog(s) => Some(s),
            _ => None,
        }
    }
    pub fn is_sdk_gated(&self) -> bool {
        matches!(self, BackendArtifact::SdkGated { .. })
    }
}

/// A named codegen backend for a described [`FabricTarget`].
pub trait TargetBackend {
    fn name(&self) -> &str;
    fn target(&self) -> &FabricTarget;
    /// Emit a single placed grid to this backend's artifact.
    fn emit_grid(&self, name: &str, g: &Grid) -> BackendArtifact;
    /// Emit an N-way reconfigurable array of placed grids to this backend's artifact.
    fn emit_config_grid(&self, name: &str, grids: &[&Grid], cfg_width: u32) -> BackendArtifact;
    /// Prove a placement computes `source` for every bounded input and return a re-checkable certificate (or
    /// `None`, fail-closed). Backend-agnostic — proof-carrying place-and-route is not tied to the emission
    /// format, so every backend inherits it. This is what lets an RDU backend add value before its SDK exists.
    fn certify(
        &self,
        g: &Grid,
        source: &[FabricOp],
        result: usize,
        width: u32,
    ) -> Option<PlacementCertificate> {
        let cert = certify_external_placement(g, source, result, width)?;
        self.recheck(&cert, g, source, result, width).ok()?;
        Some(cert)
    }
    /// The reader's side of [`Self::certify`]: `cert` held to the obligation rebuilt from the placement `g` and its
    /// `source`, never to the formula it carries, the refusal naming this backend and the placement.
    fn recheck(
        &self,
        cert: &PlacementCertificate,
        g: &Grid,
        source: &[FabricOp],
        result: usize,
        width: u32,
    ) -> Result<(), PlacementFault> {
        recheck_external_placement(cert, g, source, result, width).map_err(|f| f.within(self.name()))
    }
}

/// A deterministic, parseable neutral dump of a placed grid — the artifact the vendor-config bridge consumes.
/// Rows/cols are walked in order and every field is a `Copy` enum's `Debug`, so the output is stable (no
/// hashmap iteration). It is NOT any vendor's format; it is the ready-to-bridge placement.
pub fn serialize_grid(g: &Grid) -> String {
    let mut s = format!(
        "grid rows={} cols={} n_in={} out={:?} extra_outs={:?}\n",
        g.rows, g.cols, g.n_in, g.out, g.extra_outs
    );
    for (r, row) in g.tiles.iter().enumerate() {
        for (c, t) in row.iter().enumerate() {
            s.push_str(&format!(
                "  [{r},{c}] alu={:?} vs={:?} he={:?}\n",
                t.alu, t.vs, t.he
            ));
        }
    }
    s
}

/// The default backend: our Sky130/FPGA path. A faithful wrapper of the existing free emitters — byte-identical
/// to calling `emit_grid_verilog` / `emit_config_grid_verilog` directly.
pub struct SkyVerilogBackend {
    pub target: FabricTarget,
}

impl SkyVerilogBackend {
    pub fn new() -> Self {
        SkyVerilogBackend {
            target: FabricTarget::full(64, 64, 64),
        }
    }
}

impl Default for SkyVerilogBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl TargetBackend for SkyVerilogBackend {
    fn name(&self) -> &str {
        "sky130-verilog"
    }
    fn target(&self) -> &FabricTarget {
        &self.target
    }
    fn emit_grid(&self, name: &str, g: &Grid) -> BackendArtifact {
        BackendArtifact::Verilog(emit_grid_verilog(name, g))
    }
    fn emit_config_grid(&self, name: &str, grids: &[&Grid], cfg_width: u32) -> BackendArtifact {
        BackendArtifact::Verilog(emit_config_grid_verilog(name, grids, cfg_width))
    }
}

/// The SambaNova **RDU** slot — the placed-grid lane, for either RDU generation ([`RduChip`]). Its target is
/// the chip's [`FabricTarget`] derived from [`RduChipFacts`]. It cannot emit RDA/SambaFlow config today —
/// that needs the vendor SDK — so its emission is honestly [`BackendArtifact::SdkGated`]: it hands back the
/// `logos.spatial.neutral` v2 envelope under `"lane": "grid"`, the same envelope the spatial lane emits,
/// with the ready-to-bridge deterministic grid dump as its body. Its proof-carrying
/// [`certify`](TargetBackend::certify) works NOW (no SDK), so an RDU-bound placement already ships with a
/// machine-checkable proof it ≡ the source. When the SDK lands, only the `envelope → RDA config` translation
/// is added.
pub struct RduBackend {
    pub facts: RduChipFacts,
    pub target: FabricTarget,
}

impl RduBackend {
    /// The SN40L lane.
    pub fn new() -> Self {
        Self::for_chip(RduChip::Sn40l)
    }
    /// The lane for one RDU generation, its target derived from that chip's facts.
    pub fn for_chip(chip: RduChip) -> Self {
        Self::with_facts(chip.facts())
    }
    /// The lane for facts with numbers dropped in (an SDK reading, a probe).
    pub fn with_facts(facts: RduChipFacts) -> Self {
        RduBackend {
            target: facts.fabric_target(),
            facts,
        }
    }
    pub const GATED_REASON: &'static str = GATED_REASON;

    fn envelope(&self, body: GridBody) -> String {
        neutral_envelope(
            &self.facts.spatial_target(),
            &self.facts,
            NeutralBody::Grid(body),
            None,
        )
    }
}

impl Default for RduBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl TargetBackend for RduBackend {
    fn name(&self) -> &str {
        self.target.name
    }
    fn target(&self) -> &FabricTarget {
        &self.target
    }
    fn emit_grid(&self, name: &str, g: &Grid) -> BackendArtifact {
        let dump = serialize_grid(g);
        let body = GridBody {
            name: name.to_string(),
            pretty: dump.clone(),
            grids: vec![dump],
            cfg_width: None,
        };
        BackendArtifact::SdkGated {
            neutral_placement: self.envelope(body),
            reason: GATED_REASON,
        }
    }
    fn emit_config_grid(&self, name: &str, grids: &[&Grid], cfg_width: u32) -> BackendArtifact {
        let dumps: Vec<String> = grids.iter().map(|g| serialize_grid(g)).collect();
        let body = GridBody {
            name: name.to_string(),
            pretty: dumps.join("---\n"),
            grids: dumps,
            cfg_width: Some(cfg_width),
        };
        BackendArtifact::SdkGated {
            neutral_placement: self.envelope(body),
            reason: GATED_REASON,
        }
    }
}
