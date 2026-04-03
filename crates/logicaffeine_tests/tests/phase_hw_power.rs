//! SUPERCRUSH Sprint S2C: Power-Aware Formal Verification

use logicaffeine_compile::codegen_sva::power::*;

#[test]
fn power_isolation_required() {
    let domains = vec![
        PowerDomain { name: "core".into(), state: PowerState::On, signals: vec!["data".into()], always_on: false },
    ];
    let crossings = vec![
        PowerCrossing { signal: "data".into(), source_domain: "core".into(), dest_domain: "io".into(), has_isolation: false, has_level_shifter: true },
    ];
    let report = analyze_power(&domains, &crossings);
    assert!(report.violations.iter().any(|v| v.violation_type == PowerViolationType::MissingIsolation),
        "Missing isolation should be flagged. Got: {:?}", report.violations);
}

#[test]
fn power_isolation_present() {
    let domains = vec![
        PowerDomain { name: "core".into(), state: PowerState::On, signals: vec!["data".into()], always_on: false },
    ];
    let crossings = vec![
        PowerCrossing { signal: "data".into(), source_domain: "core".into(), dest_domain: "io".into(), has_isolation: true, has_level_shifter: true },
    ];
    let report = analyze_power(&domains, &crossings);
    assert!(!report.violations.iter().any(|v| v.violation_type == PowerViolationType::MissingIsolation),
        "Isolation present should not be flagged");
}

#[test]
fn power_retention_preserved() {
    let domains = vec![
        PowerDomain { name: "mem".into(), state: PowerState::Retention, signals: vec!["reg_a".into()], always_on: false },
    ];
    let report = analyze_power(&domains, &[]);
    assert!(!report.violations.iter().any(|v| v.violation_type == PowerViolationType::MissingRetention),
        "Domain with signals should not flag missing retention");
}

#[test]
fn power_sequence_correct() {
    let domains = vec![
        PowerDomain { name: "cpu".into(), state: PowerState::Off, signals: vec!["out".into()], always_on: false },
    ];
    let props = power_sequence_properties(&domains);
    assert!(!props.is_empty(), "Should generate sequence properties");
    assert!(props[0].body.contains("power_on"), "Should reference power on");
}

#[test]
fn power_domain_modeling() {
    let domain = PowerDomain { name: "d1".into(), state: PowerState::On, signals: vec![], always_on: true };
    assert_eq!(domain.state, PowerState::On);
    assert!(domain.always_on);
}

#[test]
fn power_multiple_domains() {
    let domains = vec![
        PowerDomain { name: "a".into(), state: PowerState::On, signals: vec!["s1".into()], always_on: false },
        PowerDomain { name: "b".into(), state: PowerState::Off, signals: vec!["s2".into()], always_on: false },
        PowerDomain { name: "c".into(), state: PowerState::Retention, signals: vec!["s3".into()], always_on: false },
    ];
    let report = analyze_power(&domains, &[]);
    assert_eq!(report.domains.len(), 3);
}

#[test]
fn power_always_on() {
    let domains = vec![
        PowerDomain { name: "aon".into(), state: PowerState::On, signals: vec!["clk".into()], always_on: true },
    ];
    let crossings = vec![
        PowerCrossing { signal: "clk".into(), source_domain: "aon".into(), dest_domain: "core".into(), has_isolation: false, has_level_shifter: true },
    ];
    let report = analyze_power(&domains, &crossings);
    assert!(!report.violations.iter().any(|v| v.violation_type == PowerViolationType::MissingIsolation),
        "Always-on domain should not need isolation");
}

#[test]
fn power_level_shifter() {
    let domains = vec![
        PowerDomain { name: "lv".into(), state: PowerState::On, signals: vec!["sig".into()], always_on: false },
    ];
    let crossings = vec![
        PowerCrossing { signal: "sig".into(), source_domain: "lv".into(), dest_domain: "hv".into(), has_isolation: true, has_level_shifter: false },
    ];
    let report = analyze_power(&domains, &crossings);
    assert!(report.violations.iter().any(|v| v.violation_type == PowerViolationType::MissingLevelShifter),
        "Missing level shifter should be flagged");
}

#[test]
fn power_sva_generated() {
    let domain = PowerDomain { name: "core".into(), state: PowerState::On, signals: vec!["data_out".into()], always_on: false };
    let props = verify_isolation(&domain);
    assert!(!props.is_empty(), "Should generate isolation SVA");
    assert!(props[0].body.contains("power_off"), "SVA should reference power off");
}

#[test]
fn power_no_power_mgmt() {
    let report = analyze_power(&[], &[]);
    assert!(report.violations.is_empty(), "No domains should mean no violations");
    assert!(report.crossings.is_empty());
}

#[test]
fn power_always_on_no_sva() {
    let domain = PowerDomain { name: "aon".into(), state: PowerState::On, signals: vec!["clk".into()], always_on: true };
    let props = verify_isolation(&domain);
    assert!(props.is_empty(), "Always-on domain should not need isolation SVA");
}

#[test]
fn power_violation_message() {
    let domains = vec![
        PowerDomain { name: "d".into(), state: PowerState::On, signals: vec!["x".into()], always_on: false },
    ];
    let crossings = vec![
        PowerCrossing { signal: "x".into(), source_domain: "d".into(), dest_domain: "e".into(), has_isolation: false, has_level_shifter: true },
    ];
    let report = analyze_power(&domains, &crossings);
    let v = report.violations.iter().find(|v| v.violation_type == PowerViolationType::MissingIsolation).unwrap();
    assert!(v.message.contains("x"), "Message should contain signal name");
}

#[test]
fn power_crossing_info() {
    let crossing = PowerCrossing {
        signal: "data".into(),
        source_domain: "src".into(),
        dest_domain: "dst".into(),
        has_isolation: true,
        has_level_shifter: true,
    };
    assert_eq!(crossing.signal, "data");
    assert!(crossing.has_isolation);
}

#[test]
fn power_report_structure() {
    let domains = vec![
        PowerDomain { name: "a".into(), state: PowerState::On, signals: vec!["s".into()], always_on: false },
    ];
    let report = analyze_power(&domains, &[]);
    assert_eq!(report.domains.len(), 1);
    assert!(report.crossings.is_empty());
}
