//! SUPERCRUSH Sprint S4A: CI/CD Integration (SARIF Output)

use logicaffeine_compile::codegen_sva::ci::*;

#[test]
fn ci_sarif_valid() {
    let report = run_ci_verification(&["spec1"], &[], &CiConfig::default());
    assert_eq!(report.sarif["version"], "2.1.0");
    assert!(report.sarif["$schema"].as_str().unwrap().contains("sarif"));
    assert!(report.sarif["runs"].is_array());
}

#[test]
fn ci_sarif_has_results() {
    let report = run_ci_verification(&["spec1", "spec2"], &[], &CiConfig::default());
    let results = &report.sarif["runs"][0]["results"];
    assert!(results.is_array());
    assert_eq!(results.as_array().unwrap().len(), 2);
}

#[test]
fn ci_pass_result() {
    let report = run_ci_verification(&["valid spec"], &[], &CiConfig::default());
    let result = &report.sarif["runs"][0]["results"][0];
    assert_eq!(result["level"], "note", "Passing should be 'note'");
}

#[test]
fn ci_fail_result() {
    let report = run_ci_verification(&[""], &[], &CiConfig::default());
    let result = &report.sarif["runs"][0]["results"][0];
    assert_eq!(result["level"], "error", "Empty spec (failing) should be 'error'");
}

#[test]
fn ci_summary_readable() {
    let report = run_ci_verification(&["spec1", "spec2"], &[], &CiConfig::default());
    assert!(report.summary.contains("passed"), "Summary should mention 'passed'");
    assert!(report.summary.contains("failed"), "Summary should mention 'failed'");
}

#[test]
fn ci_duration_tracked() {
    let report = run_ci_verification(&["spec1"], &[], &CiConfig::default());
    assert!(report.duration_ms >= 0, "Duration should be non-negative");
}

#[test]
fn ci_multiple_specs() {
    let report = run_ci_verification(&["s1", "s2", "s3"], &[], &CiConfig::default());
    assert_eq!(report.properties_checked, 3);
}

#[test]
fn ci_empty_spec() {
    let report = run_ci_verification(&[], &[], &CiConfig::default());
    assert_eq!(report.properties_checked, 0);
    assert_eq!(report.properties_failed, 0);
    let results = report.sarif["runs"][0]["results"].as_array().unwrap();
    assert!(results.is_empty());
}

#[test]
fn ci_counterexample_in_sarif() {
    let report = run_ci_verification(&[""], &[], &CiConfig::default());
    let result = &report.sarif["runs"][0]["results"][0];
    let msg = result["message"]["text"].as_str().unwrap();
    assert!(!msg.is_empty(), "Failing result should have a message");
}

#[test]
fn ci_property_location() {
    let report = run_ci_verification(&["spec"], &[], &CiConfig::default());
    let result = &report.sarif["runs"][0]["results"][0];
    assert!(result["locations"].is_array(), "Should have locations array");
}

#[test]
fn ci_exit_code_pass() {
    let report = run_ci_verification(&["spec1"], &[], &CiConfig::default());
    assert_eq!(exit_code(&report), 0, "All pass should be exit 0");
}

#[test]
fn ci_exit_code_fail() {
    let report = run_ci_verification(&[""], &[], &CiConfig::default());
    assert_eq!(exit_code(&report), 1, "Any fail should be exit 1");
}

#[test]
fn ci_report_serializable() {
    let report = run_ci_verification(&["spec1"], &[], &CiConfig::default());
    let json_str = serde_json::to_string(&report.sarif).unwrap();
    assert!(!json_str.is_empty());
}

#[test]
fn ci_template_generated() {
    let template = generate_github_actions_template();
    assert!(template.contains("logicaffeine"), "Template should mention logicaffeine");
    assert!(template.contains("sarif"), "Template should mention SARIF");
    assert!(template.contains("github"), "Template should mention GitHub");
}

#[test]
fn ci_tool_info() {
    let report = run_ci_verification(&["spec"], &[], &CiConfig::default());
    let driver = &report.sarif["runs"][0]["tool"]["driver"];
    assert_eq!(driver["name"], "logicaffeine");
}
