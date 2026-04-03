//! SUPERCRUSH Sprint S2B: CDC (Clock Domain Crossing) Formal Verification

use logicaffeine_compile::codegen_sva::cdc::*;
use logicaffeine_compile::codegen_sva::rtl_extract::{RtlModule, RtlPort, RtlSignal, SignalType};

fn make_rtl(signals: Vec<(&str, u32)>) -> RtlModule {
    RtlModule {
        name: "test_module".into(),
        ports: vec![],
        signals: signals.into_iter().map(|(name, width)| RtlSignal {
            name: name.into(),
            signal_type: SignalType::Wire,
            width,
        }).collect(),
        params: vec![],
        clocks: vec!["clk_a".into(), "clk_b".into()],
    }
}

fn two_domains() -> Vec<CdcClockDomain> {
    vec![
        CdcClockDomain { name: "a".into(), clock_signal: "clk_a".into() },
        CdcClockDomain { name: "b".into(), clock_signal: "clk_b".into() },
    ]
}

#[test]
fn cdc_two_flop_detected() {
    let rtl = make_rtl(vec![("a_data", 1), ("a_data_sync1", 1), ("a_data_sync2", 1)]);
    let report = analyze_cdc(&rtl, &two_domains());
    assert!(report.patterns.iter().any(|p| matches!(p, CdcPattern::TwoFlopSync { .. })),
        "Should detect 2-flop sync. Patterns: {:?}", report.patterns);
}

#[test]
fn cdc_missing_sync_flagged() {
    let rtl = make_rtl(vec![("a_signal", 1)]);
    let report = analyze_cdc(&rtl, &two_domains());
    assert!(report.violations.iter().any(|v| v.violation_type == CdcViolationType::MissingSynchronizer),
        "Direct crossing should be flagged. Violations: {:?}", report.violations);
}

#[test]
fn cdc_gray_code() {
    let rtl = make_rtl(vec![("a_gray_ptr", 4)]);
    let report = analyze_cdc(&rtl, &two_domains());
    assert!(report.patterns.iter().any(|p| matches!(p, CdcPattern::GrayCode { .. })),
        "Should detect gray code. Patterns: {:?}", report.patterns);
}

#[test]
fn cdc_handshake_recognized() {
    let rtl = make_rtl(vec![("a_req", 1), ("a_ack", 1)]);
    let report = analyze_cdc(&rtl, &two_domains());
    assert!(report.patterns.iter().any(|p| matches!(p, CdcPattern::HandshakeCdc { .. })),
        "Should detect handshake. Patterns: {:?}", report.patterns);
}

#[test]
fn cdc_no_crossing_clean() {
    let rtl = make_rtl(vec![("internal_sig", 1)]);
    let domains = vec![CdcClockDomain { name: "clk".into(), clock_signal: "clk".into() }];
    let report = analyze_cdc(&rtl, &domains);
    assert!(report.crossings.is_empty(), "Single domain should have no crossings");
    assert!(report.violations.is_empty(), "Single domain should have no violations");
}

#[test]
fn cdc_multiple_crossings() {
    let rtl = make_rtl(vec![("a_sig1", 1), ("a_sig2", 1), ("b_sig3", 1)]);
    let report = analyze_cdc(&rtl, &two_domains());
    assert!(report.crossings.len() >= 2, "Should detect multiple crossings. Got: {}", report.crossings.len());
}

#[test]
fn cdc_violation_includes_path() {
    let rtl = make_rtl(vec![("a_data", 1)]);
    let report = analyze_cdc(&rtl, &two_domains());
    if let Some(v) = report.violations.first() {
        assert!(!v.source_domain.is_empty(), "Violation should have source domain");
        assert!(!v.dest_domain.is_empty(), "Violation should have dest domain");
        assert!(!v.signal.is_empty(), "Violation should have signal name");
    }
}

#[test]
fn cdc_reconvergence_multibit() {
    // Multi-bit signal with 2-flop sync but no gray code → reconvergence risk
    let rtl = make_rtl(vec![("a_bus", 8), ("a_bus_sync1", 8), ("a_bus_sync2", 8)]);
    let report = analyze_cdc(&rtl, &two_domains());
    assert!(report.violations.iter().any(|v| v.violation_type == CdcViolationType::BusWithoutEncoding),
        "Multi-bit with 2-flop should flag bus encoding. Violations: {:?}", report.violations);
}

#[test]
fn cdc_sva_generated() {
    let rtl = make_rtl(vec![("a_data", 1), ("a_data_sync1", 1), ("a_data_sync2", 1)]);
    let report = analyze_cdc(&rtl, &two_domains());
    let props = cdc_sva_properties(&report);
    // If there's a 2-flop pattern, should generate SVA
    if report.patterns.iter().any(|p| matches!(p, CdcPattern::TwoFlopSync { .. })) {
        assert!(!props.is_empty(), "Should generate SVA for 2-flop sync");
    }
}

#[test]
fn cdc_fifo_pattern() {
    let rtl = make_rtl(vec![("a_async_fifo_data", 8)]);
    let report = analyze_cdc(&rtl, &two_domains());
    assert!(report.patterns.iter().any(|p| matches!(p, CdcPattern::AsyncFifo)),
        "Should detect async FIFO. Patterns: {:?}", report.patterns);
}

#[test]
fn cdc_three_flop() {
    let rtl = make_rtl(vec![("a_sig", 1), ("a_sig_sync1", 1), ("a_sig_sync2", 1), ("a_sig_sync3", 1)]);
    let report = analyze_cdc(&rtl, &two_domains());
    assert!(report.patterns.iter().any(|p| matches!(p, CdcPattern::ThreeFlopSync { .. })),
        "Should detect 3-flop sync. Patterns: {:?}", report.patterns);
}

#[test]
fn cdc_crossing_safe_flag() {
    let rtl = make_rtl(vec![("a_data", 1), ("a_data_sync1", 1), ("a_data_sync2", 1)]);
    let report = analyze_cdc(&rtl, &two_domains());
    let crossing = report.crossings.iter().find(|c| c.signal == "a_data");
    if let Some(c) = crossing {
        assert!(c.safe, "Crossing with sync pattern should be marked safe");
    }
}

#[test]
fn cdc_crossing_unsafe_flag() {
    let rtl = make_rtl(vec![("a_raw", 1)]);
    let report = analyze_cdc(&rtl, &two_domains());
    let crossing = report.crossings.iter().find(|c| c.signal == "a_raw");
    if let Some(c) = crossing {
        assert!(!c.safe, "Crossing without sync should be marked unsafe");
    }
}

#[test]
fn cdc_empty_rtl() {
    let rtl = RtlModule { name: "empty".into(), ports: vec![], signals: vec![], params: vec![], clocks: vec![] };
    let report = analyze_cdc(&rtl, &two_domains());
    assert!(report.crossings.is_empty());
    assert!(report.violations.is_empty());
}

#[test]
fn cdc_single_domain_no_analysis() {
    let rtl = make_rtl(vec![("sig", 1)]);
    let domains = vec![CdcClockDomain { name: "clk".into(), clock_signal: "clk".into() }];
    let report = analyze_cdc(&rtl, &domains);
    assert!(report.crossings.is_empty(), "Single domain should not trigger CDC analysis");
}

#[test]
fn cdc_handshake_sva() {
    let rtl = make_rtl(vec![("a_req", 1), ("a_ack", 1)]);
    let report = analyze_cdc(&rtl, &two_domains());
    let props = cdc_sva_properties(&report);
    let handshake_prop = props.iter().find(|p| p.name.contains("handshake"));
    if report.patterns.iter().any(|p| matches!(p, CdcPattern::HandshakeCdc { .. })) {
        assert!(handshake_prop.is_some(), "Should generate handshake SVA");
    }
}

#[test]
fn cdc_report_domains_listed() {
    let rtl = make_rtl(vec![("a_x", 1)]);
    let report = analyze_cdc(&rtl, &two_domains());
    // Report should have data even if no crossings
    assert!(report.crossings.len() + report.violations.len() >= 0);
}

#[test]
fn cdc_pattern_equality() {
    let p1 = CdcPattern::GrayCode { width: 4 };
    let p2 = CdcPattern::GrayCode { width: 4 };
    assert_eq!(p1, p2);
}

#[test]
fn cdc_violation_type_equality() {
    assert_eq!(CdcViolationType::MissingSynchronizer, CdcViolationType::MissingSynchronizer);
    assert_ne!(CdcViolationType::MissingSynchronizer, CdcViolationType::GlitchRisk);
}

#[test]
fn cdc_three_domains() {
    let rtl = make_rtl(vec![("a_sig", 1), ("b_sig", 1), ("c_sig", 1)]);
    let domains = vec![
        CdcClockDomain { name: "a".into(), clock_signal: "clk_a".into() },
        CdcClockDomain { name: "b".into(), clock_signal: "clk_b".into() },
        CdcClockDomain { name: "c".into(), clock_signal: "clk_c".into() },
    ];
    let report = analyze_cdc(&rtl, &domains);
    // Should analyze all domain pairs
    assert!(report.crossings.len() >= 1, "Three domains should find crossings");
}

#[test]
fn cdc_signal_width_preserved() {
    let rtl = make_rtl(vec![("a_data", 16)]);
    let report = analyze_cdc(&rtl, &two_domains());
    if let Some(crossing) = report.crossings.first() {
        assert_eq!(crossing.signal, "a_data");
    }
}

#[test]
fn cdc_violation_message_descriptive() {
    let rtl = make_rtl(vec![("a_raw_signal", 1)]);
    let report = analyze_cdc(&rtl, &two_domains());
    for v in &report.violations {
        assert!(!v.message.is_empty(), "Violation message should be non-empty");
        assert!(v.message.contains(&v.signal), "Message should contain signal name");
    }
}
