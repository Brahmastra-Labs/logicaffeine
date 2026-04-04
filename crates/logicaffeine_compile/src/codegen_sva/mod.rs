//! SVA/PSL Code Generation for Hardware Verification
//!
//! Generates SystemVerilog Assertions (SVA), Property Specification Language (PSL),
//! and Rust runtime monitors from temporal property specifications.

pub mod sva_model;
pub mod sva_to_verify;
pub mod fol_to_verify;
pub mod hw_pipeline;
pub mod rtl_extract;
pub mod fol_to_sva;
pub mod coverage;
pub mod sufficiency;
pub mod rtl_kg;
pub mod synthesis_refine;
pub mod protocols;
pub mod verify_to_kernel;
pub mod ci;
pub mod power;
pub mod cdc;
pub mod verified_compiler;
pub mod z3_synth;
pub mod synthesize;

#[cfg(feature = "verification")]
pub mod waveform;
#[cfg(feature = "verification")]
pub mod testgen;
#[cfg(feature = "verification")]
pub mod decompose;
#[cfg(feature = "verification")]
pub mod invariants;
#[cfg(feature = "verification")]
pub mod spec_health;

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
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
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

/// Emit a Rust runtime monitor for a property.
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
        // TODO: wire signal inputs
        true
    }}
}}"#,
        struct_name = struct_name,
        body = prop.body,
    )
}
