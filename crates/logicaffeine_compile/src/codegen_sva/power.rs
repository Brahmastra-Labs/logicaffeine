//! Power-Aware Formal Verification
//!
//! Power domains introduce isolation, retention, and level shifting requirements.
//! Missing isolation cells cause functional bugs in power-managed designs.

use super::{SvaProperty, SvaAssertionKind};

/// Power domain state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PowerState {
    On,
    Off,
    Retention,
}

/// A power domain definition.
#[derive(Debug, Clone)]
pub struct PowerDomain {
    pub name: String,
    pub state: PowerState,
    pub signals: Vec<String>,
    pub always_on: bool,
}

/// A signal crossing between power domains.
#[derive(Debug, Clone)]
pub struct PowerCrossing {
    pub signal: String,
    pub source_domain: String,
    pub dest_domain: String,
    pub has_isolation: bool,
    pub has_level_shifter: bool,
}

/// Power violation type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PowerViolationType {
    MissingIsolation,
    MissingRetention,
    MissingLevelShifter,
    IncorrectSequence,
}

/// A power violation.
#[derive(Debug, Clone)]
pub struct PowerViolation {
    pub signal: String,
    pub violation_type: PowerViolationType,
    pub message: String,
}

/// Power analysis report.
#[derive(Debug, Clone)]
pub struct PowerReport {
    pub domains: Vec<PowerDomain>,
    pub crossings: Vec<PowerCrossing>,
    pub violations: Vec<PowerViolation>,
}

/// Analyze power domains and crossings for violations.
pub fn analyze_power(
    domains: &[PowerDomain],
    crossings: &[PowerCrossing],
) -> PowerReport {
    let mut violations = Vec::new();

    for crossing in crossings {
        // Find source domain
        let source = domains.iter().find(|d| d.name == crossing.source_domain);

        // Check isolation requirement
        if let Some(src) = source {
            if !src.always_on && !crossing.has_isolation {
                violations.push(PowerViolation {
                    signal: crossing.signal.clone(),
                    violation_type: PowerViolationType::MissingIsolation,
                    message: format!(
                        "Signal '{}' crosses from '{}' to '{}' without isolation cell",
                        crossing.signal, crossing.source_domain, crossing.dest_domain
                    ),
                });
            }
        }

        // Check level shifter
        if !crossing.has_level_shifter {
            // Only flag if domains have different implied voltage levels
            // Simplified: flag if explicitly missing
            violations.push(PowerViolation {
                signal: crossing.signal.clone(),
                violation_type: PowerViolationType::MissingLevelShifter,
                message: format!(
                    "Signal '{}' crosses domains without level shifter",
                    crossing.signal
                ),
            });
        }
    }

    // Check retention
    for domain in domains {
        if domain.state == PowerState::Retention {
            let has_retention_signals = !domain.signals.is_empty();
            if !has_retention_signals {
                violations.push(PowerViolation {
                    signal: domain.name.clone(),
                    violation_type: PowerViolationType::MissingRetention,
                    message: format!("Domain '{}' in retention but has no signals", domain.name),
                });
            }
        }
    }

    PowerReport {
        domains: domains.to_vec(),
        crossings: crossings.to_vec(),
        violations,
    }
}

/// Generate SVA properties for power isolation verification.
pub fn verify_isolation(domain: &PowerDomain) -> Vec<SvaProperty> {
    let mut props = Vec::new();

    if !domain.always_on {
        for sig in &domain.signals {
            props.push(SvaProperty {
                name: format!("power_isolation_{}", sig),
                clock: "clk".into(),
                body: format!(
                    "({}_power_off) |-> ({}_iso == 1'b0)",
                    domain.name, sig
                ),
                kind: SvaAssertionKind::Assert,
            });
        }
    }

    props
}

/// Generate SVA properties for power sequencing.
pub fn power_sequence_properties(domains: &[PowerDomain]) -> Vec<SvaProperty> {
    let mut props = Vec::new();

    for domain in domains {
        if !domain.always_on {
            props.push(SvaProperty {
                name: format!("power_seq_{}", domain.name),
                clock: "clk".into(),
                body: format!(
                    "({name}_power_on) |-> ({name}_iso_release)",
                    name = domain.name
                ),
                kind: SvaAssertionKind::Assert,
            });
        }
    }

    props
}
