//! SVA/PSL Code Generation for Hardware Verification
//!
//! Generates SystemVerilog Assertions (SVA), Property Specification Language (PSL),
//! and Rust runtime monitors from temporal property specifications.

pub mod arbiter_rtl;
pub mod autopipe;
pub mod backend;
pub mod bignum_rtl;
pub mod bitblast;
pub mod block_engine_rtl;
pub mod caravel_project;
pub mod caravel_rtl;
pub mod cdc;
pub mod chan_rtl;
pub mod chip_protocol;
pub mod choreography;
pub mod ci;
pub mod computedram_rtl;
pub mod controller_gen;
pub mod coproc_core;
pub mod coproc_soc;
pub mod coverage;
pub mod crdt_rtl;
pub mod dae;
pub mod ddr2_dfi;
pub mod ddr2_jedec;
pub mod ddr2_phy;
pub mod ddr2_traffic;
pub mod ddr4_jedec;
pub mod decision_workload;
pub mod dram;
pub mod engine_build;
pub mod fabric_config;
pub mod fabric_grid;
pub mod fabric_pipe;
pub mod fabric_rtl;
pub mod fabric_soc;
pub mod fabric_target;
pub mod fabric_tile;
pub mod fanout;
pub mod flow_config;
pub mod fol_to_sva;
pub mod fol_to_verify;
pub mod floorplan;
pub mod fp_hard;
pub mod fp_hypotheses;
pub mod fp_oracle;
pub mod fp_receipt;
pub mod full_asm;
pub mod fusion;
pub mod gpu_simt_rtl;
pub mod handshake_rtl;
pub mod hart;
pub mod hw_pipeline;
pub mod infinichip_rtl;
pub mod keccak_rtl;
pub mod lutmap;
pub mod memsched;
pub mod metapipeline;
pub mod onboard;
pub mod mul_tree;
pub mod netlist;
pub mod pdk_target;
pub mod plasticine;
pub mod plasticine_place;
pub mod plasticine_report;
pub mod pmu;
pub mod pnr_certificate;
pub mod power;
pub mod preflight;
pub mod protocols;
pub mod psram;
pub mod qor_harness;
pub mod quartus;
pub mod reconfig_fabric_rtl;
pub mod reverse_module;
pub mod rtl;
pub mod rtl_extract;
pub mod rtl_kg;
pub mod scalar_asm;
pub mod scratchpad_rtl;
pub mod select_hoist;
pub mod sequencer_rtl;
pub mod shuttle_wrapper;
pub mod signal_design;
pub mod signoff_blocks;
pub mod simt_ports;
pub mod simd;
pub mod soft_core_model;
pub mod spatial_emit;
pub mod storage_controller_rtl;
pub mod sufficiency;
pub mod sva_model;
pub mod sva_to_proof;
pub mod sva_to_verify;
pub mod sva_vacuity;
pub mod synthesis_refine;
pub mod synthesize;
pub mod systolic;
pub mod terminal_protocol;
pub mod terminal_rtl;
pub mod timing_graph;
pub mod traffic_flow;
pub mod unified_placement;
pub mod verified_compiler;
pub mod verify_to_kernel;
pub mod wpe_caravel;
pub mod wpe_floorplan;
pub mod wpe_offchip;
pub mod wprog_engine_rtl;
pub mod xpu_soc_rtl;
pub mod z3_synth;

#[cfg(feature = "verification")]
pub mod decompose;
#[cfg(feature = "verification")]
pub mod invariants;
#[cfg(feature = "verification")]
pub mod spec_health;
#[cfg(feature = "verification")]
pub mod testgen;
#[cfg(feature = "verification")]
pub mod waveform;

/// Assertion type — determines the SVA wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SvaAssertionKind {
    /// `assert property` — hard safety requirement.
    Assert,
    /// `cover property` — liveness / expected behavior.
    Cover,
    /// `assume property` — environment constraint.
    Assume,
}

/// A single SVA property ready for emission.
#[derive(Debug, Clone)]
pub struct SvaProperty {
    pub name: String,
    pub clock: String,
    pub body: String,
    pub kind: SvaAssertionKind,
}

/// Sanitize a human-readable property name into a valid SVA identifier.
/// "Data Integrity" → "p_data_integrity"
pub fn sanitize_property_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("p_{}", sanitized.trim_matches('_'))
}

/// Emit a single SVA property with its assertion wrapper.
pub fn emit_sva_property(prop: &SvaProperty) -> String {
    let wrapper = match prop.kind {
        SvaAssertionKind::Assert => "assert property",
        SvaAssertionKind::Cover => "cover property",
        SvaAssertionKind::Assume => "assume property",
    };

    let error_action = match prop.kind {
        SvaAssertionKind::Assert => {
            format!(" else $error(\"{} violation\")", prop.name)
        }
        _ => String::new(),
    };

    format!(
        "property {};\n    @(posedge {}) {};\nendproperty\n{} ({}){};",
        prop.name, prop.clock, prop.body, wrapper, prop.name, error_action
    )
}

/// Emit multiple SVA properties as a complete module.
pub fn emit_sva_module(props: &[SvaProperty]) -> String {
    props
        .iter()
        .map(|p| emit_sva_property(p))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Emit a single PSL property.
pub fn emit_psl_property(prop: &SvaProperty) -> String {
    let kind = match prop.kind {
        SvaAssertionKind::Assert => "assert always",
        SvaAssertionKind::Cover => "cover",
        SvaAssertionKind::Assume => "assume always",
    };
    format!("-- {}\n{} ({});", prop.name, kind, prop.body)
}

/// Emit a Rust runtime monitor scaffold for a property.
///
/// Generates a struct with a `check()` method stub. The generated `check()`
/// always returns `true` — callers must fill in the signal-dependent logic
/// for their specific use case. This is a code generation template, not a
/// complete monitor implementation.
pub fn emit_rust_monitor(prop: &SvaProperty) -> String {
    let struct_name = prop
        .name
        .replace("p_", "")
        .split('_')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect::<String>()
        + "Monitor";

    format!(
        r#"pub struct {struct_name} {{
    cycle: u64,
}}

impl {struct_name} {{
    pub fn new() -> Self {{
        Self {{ cycle: 0 }}
    }}

    /// Check the property for one clock cycle.
    /// Returns true if the property holds.
    pub fn check(&mut self) -> bool {{
        self.cycle += 1;
        // Property: {body}
        // Scaffold: fill in signal-dependent check logic for your design
        true
    }}
}}"#,
        struct_name = struct_name,
        body = prop.body,
    )
}
