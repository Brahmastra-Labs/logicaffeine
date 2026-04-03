//! Clock Domain Crossing (CDC) Formal Verification
//!
//! CDC bugs are among the hardest to find in simulation. Metastability,
//! data coherence, and synchronizer correctness require formal analysis.

use super::{SvaProperty, SvaAssertionKind};
use super::rtl_extract::RtlModule;

/// A clock domain for CDC analysis.
#[derive(Debug, Clone)]
pub struct CdcClockDomain {
    pub name: String,
    pub clock_signal: String,
}

/// CDC synchronization pattern.
#[derive(Debug, Clone, PartialEq)]
pub enum CdcPattern {
    TwoFlopSync { source_domain: String, dest_domain: String },
    ThreeFlopSync { source_domain: String, dest_domain: String },
    GrayCode { width: u32 },
    HandshakeCdc { req: String, ack: String },
    PulseSynchronizer,
    AsyncFifo,
}

/// A signal crossing between clock domains.
#[derive(Debug, Clone)]
pub struct CdcCrossing {
    pub signal: String,
    pub source_domain: String,
    pub dest_domain: String,
    pub pattern: Option<CdcPattern>,
    pub safe: bool,
}

/// CDC violation type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CdcViolationType {
    MissingSynchronizer,
    Reconvergence,
    GlitchRisk,
    BusWithoutEncoding,
}

/// A CDC violation.
#[derive(Debug, Clone)]
pub struct CdcViolation {
    pub signal: String,
    pub source_domain: String,
    pub dest_domain: String,
    pub violation_type: CdcViolationType,
    pub message: String,
}

/// CDC analysis report.
#[derive(Debug, Clone)]
pub struct CdcReport {
    pub crossings: Vec<CdcCrossing>,
    pub violations: Vec<CdcViolation>,
    pub patterns: Vec<CdcPattern>,
}

/// Analyze an RTL module for clock domain crossing issues.
pub fn analyze_cdc(
    rtl: &RtlModule,
    domains: &[CdcClockDomain],
) -> CdcReport {
    let mut crossings = Vec::new();
    let mut violations = Vec::new();
    let mut patterns = Vec::new();

    if domains.len() < 2 {
        return CdcReport { crossings, violations, patterns };
    }

    // For each signal, determine which domain it belongs to
    for signal in &rtl.signals {
        // Check if this signal is used across domains
        for i in 0..domains.len() {
            for j in (i + 1)..domains.len() {
                let src = &domains[i];
                let dst = &domains[j];

                // Heuristic: signal name contains domain reference
                let in_src = signal.name.contains(&src.name) || is_in_domain(&signal.name, src);
                let in_dst = signal.name.contains(&dst.name) || is_in_domain(&signal.name, dst);

                if in_src || in_dst {
                    // Check for synchronizer pattern
                    let pattern = detect_pattern(&signal.name, rtl, src, dst);
                    let safe = pattern.is_some();

                    if let Some(ref pat) = pattern {
                        patterns.push(pat.clone());
                    }

                    crossings.push(CdcCrossing {
                        signal: signal.name.clone(),
                        source_domain: src.name.clone(),
                        dest_domain: dst.name.clone(),
                        pattern: pattern.clone(),
                        safe,
                    });

                    if !safe {
                        violations.push(CdcViolation {
                            signal: signal.name.clone(),
                            source_domain: src.name.clone(),
                            dest_domain: dst.name.clone(),
                            violation_type: CdcViolationType::MissingSynchronizer,
                            message: format!(
                                "Signal '{}' crosses from '{}' to '{}' without synchronizer",
                                signal.name, src.name, dst.name
                            ),
                        });
                    }
                }
            }
        }
    }

    // Check for multi-bit crossings without gray code
    for crossing in &crossings {
        let sig = rtl.signals.iter().find(|s| s.name == crossing.signal);
        if let Some(s) = sig {
            if s.width > 1 && crossing.safe {
                if let Some(CdcPattern::TwoFlopSync { .. }) = &crossing.pattern {
                    violations.push(CdcViolation {
                        signal: crossing.signal.clone(),
                        source_domain: crossing.source_domain.clone(),
                        dest_domain: crossing.dest_domain.clone(),
                        violation_type: CdcViolationType::BusWithoutEncoding,
                        message: format!(
                            "Multi-bit signal '{}' (width {}) uses 2-flop sync without gray code",
                            crossing.signal, s.width
                        ),
                    });
                }
            }
        }
    }

    CdcReport { crossings, violations, patterns }
}

/// Generate SVA properties for CDC verification.
pub fn cdc_sva_properties(report: &CdcReport) -> Vec<SvaProperty> {
    let mut props = Vec::new();

    for crossing in &report.crossings {
        if let Some(CdcPattern::TwoFlopSync { ref dest_domain, .. }) = crossing.pattern {
            props.push(SvaProperty {
                name: format!("cdc_2flop_{}", crossing.signal),
                clock: format!("clk_{}", dest_domain),
                body: format!(
                    "{sig}_sync1 |=> {sig}_sync2",
                    sig = crossing.signal
                ),
                kind: SvaAssertionKind::Assert,
            });
        }

        if let Some(CdcPattern::HandshakeCdc { ref req, ref ack }) = crossing.pattern {
            props.push(SvaProperty {
                name: format!("cdc_handshake_{}", crossing.signal),
                clock: "clk".into(),
                body: format!("{} |-> s_eventually({})", req, ack),
                kind: SvaAssertionKind::Assert,
            });
        }
    }

    props
}

fn is_in_domain(signal_name: &str, domain: &CdcClockDomain) -> bool {
    signal_name.starts_with(&format!("{}_", domain.name))
}

fn detect_pattern(
    signal_name: &str,
    rtl: &RtlModule,
    src: &CdcClockDomain,
    dst: &CdcClockDomain,
) -> Option<CdcPattern> {
    // Check for 3-flop sync FIRST (before 2-flop, since 3-flop implies sync1+sync2+sync3)
    let sync1 = format!("{}_sync1", signal_name);
    let sync2 = format!("{}_sync2", signal_name);
    let sync3 = format!("{}_sync3", signal_name);
    if rtl.signals.iter().any(|s| s.name == sync1)
        && rtl.signals.iter().any(|s| s.name == sync2)
        && rtl.signals.iter().any(|s| s.name == sync3)
    {
        return Some(CdcPattern::ThreeFlopSync {
            source_domain: src.name.clone(),
            dest_domain: dst.name.clone(),
        });
    }

    // Check for 2-flop sync pattern: signal_sync1, signal_sync2
    if rtl.signals.iter().any(|s| s.name == sync1) && rtl.signals.iter().any(|s| s.name == sync2) {
        return Some(CdcPattern::TwoFlopSync {
            source_domain: src.name.clone(),
            dest_domain: dst.name.clone(),
        });
    }

    // Check for handshake pattern
    if signal_name.contains("req") {
        let ack_name = signal_name.replace("req", "ack");
        if rtl.signals.iter().any(|s| s.name == ack_name) {
            return Some(CdcPattern::HandshakeCdc {
                req: signal_name.into(),
                ack: ack_name,
            });
        }
    }

    // Check for gray code
    if signal_name.contains("gray") || signal_name.contains("grey") {
        let sig = rtl.signals.iter().find(|s| s.name == signal_name)?;
        return Some(CdcPattern::GrayCode { width: sig.width });
    }

    // Check for FIFO
    if signal_name.contains("fifo") || signal_name.contains("async_fifo") {
        return Some(CdcPattern::AsyncFifo);
    }

    None
}
